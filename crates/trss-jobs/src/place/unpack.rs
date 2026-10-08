//! Unpacking a received archive (`docs/specs/subtitles.md`, 압축 해제의 격리와
//! 한도 and the package analysis's rows for archives and split archives).
//!
//! Before the analysis, each received archive not tried yet is unpacked by a
//! child process ([`trss_archive::run::Unpacker`], the `trss-extract`
//! program beside the worker) into the receive area's
//! `<job id>/.unpack/<file id>/` ([`ReceiveArea::unpack_dir`]):
//!
//! - The receipts are put together first, those of one post and one folder of
//!   it ([`group`]): the volumes of a split archive (`.part1.rar`, `.rar` and
//!   `.r00`, `.7z.001`) are unpacked as one, from their first volume. A set
//!   missing its first volume or one in the middle, one with two volumes of a
//!   number, a ZIP spanned over `.z01` volumes (사용자 결정, 2026-10-05) and a
//!   RAR cut into `.rar.001` are not unpacked, with the reason. An archive is
//!   what [`is_archive_receipt`] says: an upload's or a find job's by the
//!   kind it kept, another's by its name and bytes.
//! - The members come back with their paths inside the archive, lengths and
//!   SHA-256; each is checked as a received file is
//!   ([`verify::check_within`], the ZIPs among them inflating no more than
//!   [`MEMBERS_INFLATE_BUDGET`] together), and they are recorded with the
//!   time (`unpacked_at`) in one transaction, the later volumes naming their
//!   first (`volume_of`). The analysis then plans the members in place of
//!   the archive ([`super::records::PlanRow::member`]).
//! - An archive the child refuses (a limit, a path, a link, a password, a
//!   volume missing, nothing in it) is not unpacked (풀지 못함): the reason is
//!   recorded (`unpack_error`) and logged, no row is planned for it, and it
//!   stays in the receive area, so a later build can analyse it without
//!   receiving it again. The job ends failed, or partly failed when something
//!   else of it was received.
//! - A try this machine fails (a full disk, the memory or time limit, a
//!   child that died or did not start; [`Unpacked::Failed`]) is not the
//!   archive's: the archive is tried again from what was received, up to
//!   [`UNPACK_TRIES`] tries with the first (사용자 결정, 2026-10-05). The next
//!   try goes [`UNPACK_RETRY_AFTER`] after the failure, or at a worker's
//!   start if that comes first ([`crate::JobStore::requeue_waiting_for_sources`]
//!   lets it go, [`crate::JobStore::requeue_unpack_retries`] puts the job back
//!   in line when its hour is up): freeing the disk needs no restart, and
//!   changing the memory limit comes with a deploy. Until then the package
//!   waits ([`UNPACK_AGAIN`], `자막 대기`), and the failure is logged with
//!   which try it was. The last try's failure is 풀지 못함 as above.
//! - A worker that stopped while a child unpacked finds no `unpacked_at`: the
//!   folder is made anew from the received archive. The child dies with the
//!   worker (its parent-death signal), and its process group is killed at the
//!   time limit.
//!
//! A placer with no unpacker (a worker without the program beside it) leaves
//! archives as they are: their packages wait ([`super::ARCHIVE_LATER`]), an
//! upload's or a find job's too.

use std::path::PathBuf;

use rusqlite::{params, Connection, OptionalExtension};
use tokio_util::sync::CancellationToken;
use trss_archive::run::Unpacked;
use trss_core::Millis;
use trss_subtitles::{
    upload::{is_archive_name, NO_FIRST_VOLUME},
    verify::{self, Format},
};

use super::{
    blocking,
    package::{member, Member},
    records::durable,
    Placer,
};
use crate::{
    area::ReceiveArea,
    model::{FileState, ItemState},
    store::{FileRow, ItemRow, JobError},
};

/// The program that unpacks an archive, beside the worker's
/// (`src/bin/trss-extract.rs`).
pub const PROGRAM: &str = "trss-extract";

/// Why a split archive with a volume missing between its first and its last
/// is not unpacked.
pub const MISSING_VOLUME: &str = "나뉜 압축 파일의 조각이 모자라요";
/// Why a ZIP spanned over `.z01` volumes is not unpacked.
pub const SPANNED_ZIP: &str = "`.z01`처럼 나눈 ZIP은 풀지 않아요";
/// Why a RAR cut into `.rar.001` pieces is not unpacked.
pub const CUT_RAR: &str = "`.rar.001`처럼 잘라 나눈 RAR은 풀지 않아요";
/// Why an archive with no file in it has nothing to analyse.
pub const EMPTY_ARCHIVE: &str = "압축 파일 안에 파일이 없어요";
/// Why a split archive with two volumes of one number (names that differ in
/// case only) is not unpacked: which one is the volume is not known.
pub const DUPLICATE_VOLUME: &str = "나뉜 압축 파일에 같은 번호의 조각이 둘 있어요";

/// How many tries an archive gets when this machine fails them, the first
/// included.
pub const UNPACK_TRIES: u32 = 3;
/// How long after a try this machine failed the next one goes, unless a
/// worker starts sooner.
pub const UNPACK_RETRY_AFTER: Millis = 60 * 60 * 1000;
/// What a package whose archive waits for its next try waits for.
pub const UNPACK_AGAIN: &str = "압축 파일을 풀지 못해 다시 풀기를 기다려요";

/// How many bytes the checks of one archive's members may inflate together
/// (a ZIP among them that is no archive by its name, a `.docx`): the limit of
/// what the archive itself may unpack to.
pub const MEMBERS_INFLATE_BUDGET: u64 = 512 << 20;
/// Why a ZIP member is not checked once its archive's budget is spent.
pub const MEMBERS_BUDGET_SPENT: &str =
    "압축 파일 안의 ZIP을 확인할 수 있는 양을 넘어서 이 파일은 받지 않아요";

/// Why a try did not unpack an archive.
enum NotUnpacked {
    Cancelled,
    /// The archive's own reason: no try of this build unpacks it.
    Refused(String),
    /// This machine's ([`Unpacked::Failed`]): tried again.
    Failed(String),
}

/// A set of volumes: its kind (0 `.partN.rar`, 1 `.rar` and `.rNN`, 2 cut,
/// 3 spanned ZIP, 4 cut RAR) and name, and its volumes' numbers and indexes.
type VolumeSet = ((u8, String), Vec<(u64, usize)>);

/// How received archives are put together, by their index among the names
/// [`group`] was given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Group {
    /// One archive: its volumes in order (one for a whole archive).
    Archive(Vec<usize>),
    /// Volumes not unpacked, for `reason`: the first of them carries it and
    /// the others are its volumes.
    Refused { volumes: Vec<usize>, reason: String },
}

/// Whether a received file is an archive to unpack: a name that says one
/// (`.zip`, `.tar.gz`, `.part2.rar`, `.r00`, `.7z.001`), or a ZIP's bytes
/// under another name that is no document's (a `.docx` is a ZIP, and an
/// attachment).
pub fn is_archive(name: &str, format: Option<Format>) -> bool {
    is_archive_name(name) || format.is_some_and(|f| member(name, f) == Member::Archive)
}

/// Whether the receipt `file`, of `format`, is an archive to unpack: one an
/// upload or a find job kept as an archive by its bytes ([`FileRow::archive`],
/// a RAR with no extension too), or one [`is_archive`] says is.
pub fn is_archive_receipt(file: &FileRow, format: Option<Format>) -> bool {
    file.archive.is_some() || is_archive(&file.name, format)
}

/// The name a receipt is put together with others under ([`group`]): its
/// folder in the post included, so that volumes of one name in two folders
/// are two sets.
fn grouped_name(file: &FileRow) -> String {
    match &file.folder {
        Some(folder) => format!("{folder}/{}", file.name),
        None => file.name.clone(),
    }
}

/// What a received archive's name says of the set it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Volume {
    /// `<set>.partN.rar`.
    Part {
        set: String,
        number: u64,
    },
    /// `<set>.rar` (number 0) or `<set>.rNN` (number NN + 1).
    OldRar {
        set: String,
        number: u64,
    },
    /// `<set>.<ext>.NNN`, cut from one archive.
    Cut {
        set: String,
        number: u64,
        rar: bool,
    },
    /// `<set>.zNN` or the `<set>.zip` that ends them.
    Spanned {
        set: String,
    },
    Whole,
}

fn volume(name: &str) -> Volume {
    let lower = name.to_lowercase();
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let Some((stem, ext)) = lower.rsplit_once('.') else {
        return Volume::Whole;
    };
    if ext == "rar" {
        if let Some((set, number)) = stem.rsplit_once(".part") {
            if digits(number) {
                return Volume::Part {
                    set: set.to_owned(),
                    number: number.parse().unwrap_or(u64::MAX),
                };
            }
        }
        return Volume::OldRar {
            set: stem.to_owned(),
            number: 0,
        };
    }
    if ext.len() >= 3 && digits(ext) {
        if let Some((_, inner)) = stem.rsplit_once('.') {
            if matches!(inner, "7z" | "zip" | "rar" | "gz" | "bz2" | "xz" | "tar") {
                return Volume::Cut {
                    set: stem.to_owned(),
                    number: ext.parse().unwrap_or(u64::MAX),
                    rar: inner == "rar",
                };
            }
        }
        return Volume::Whole;
    }
    if let Some(number) = ext.strip_prefix('r').filter(|n| n.len() >= 2 && digits(n)) {
        return Volume::OldRar {
            set: stem.to_owned(),
            number: number.parse::<u64>().map_or(u64::MAX, |n| n + 1),
        };
    }
    if ext
        .strip_prefix('z')
        .is_some_and(|n| n.len() >= 2 && digits(n))
    {
        return Volume::Spanned {
            set: stem.to_owned(),
        };
    }
    Volume::Whole
}

/// Puts received archives named `names` (their folders included) together:
/// each whole archive alone, and the volumes of a split one as one archive
/// from its first volume (`docs/specs/subtitles.md`, ZIP 밖의 압축 형식·나뉜 압축
/// 파일). A `.rar` with no `.r00` beside it is a whole archive, and so is a
/// `.zip` with no `.z01`.
pub fn group(names: &[&str]) -> Vec<Group> {
    let volumes: Vec<Volume> = names.iter().map(|n| volume(n)).collect();
    let spanned: Vec<&str> = volumes
        .iter()
        .filter_map(|v| match v {
            Volume::Spanned { set } => Some(set.as_str()),
            _ => None,
        })
        .collect();
    let old_rar: Vec<&str> = volumes
        .iter()
        .filter_map(|v| match v {
            Volume::OldRar { set, number } if *number > 0 => Some(set.as_str()),
            _ => None,
        })
        .collect();

    // A lone `.rar` or `.zip` is whole.
    let mut sets: Vec<VolumeSet> = Vec::new();
    let mut groups = Vec::new();
    for (i, (name, v)) in names.iter().zip(&volumes).enumerate() {
        let key = match v {
            Volume::Part { set, number } => Some(((0, set.clone()), *number)),
            Volume::OldRar { set, number } if *number > 0 || old_rar.contains(&set.as_str()) => {
                Some(((1, set.clone()), *number))
            }
            Volume::Cut { set, number, rar } => {
                Some(((if *rar { 4 } else { 2 }, set.clone()), *number))
            }
            Volume::Spanned { set } => Some(((3, set.clone()), 0)),
            _ => None,
        };
        let key = match key {
            Some(key) => Some(key),
            None => {
                let lower = name.to_lowercase();
                lower
                    .strip_suffix(".zip")
                    .filter(|set| spanned.contains(set))
                    .map(|set| ((3, set.to_owned()), 0))
            }
        };
        match key {
            Some((key, number)) => match sets.iter_mut().find(|(k, _)| *k == key) {
                Some((_, volumes)) => volumes.push((number, i)),
                None => sets.push((key, vec![(number, i)])),
            },
            None => groups.push((i, Group::Archive(vec![i]))),
        }
    }
    for ((kind, _), mut volumes) in sets {
        volumes.sort();
        let indexes: Vec<usize> = volumes.iter().map(|(_, i)| *i).collect();
        let first = match kind {
            0 | 2 | 4 => 1,
            _ => 0,
        };
        let reason = if kind == 3 {
            Some(SPANNED_ZIP)
        } else if kind == 4 {
            Some(CUT_RAR)
        } else if volumes.windows(2).any(|w| w[0].0 == w[1].0) {
            Some(DUPLICATE_VOLUME)
        } else if volumes[0].0 != first {
            Some(NO_FIRST_VOLUME)
        } else if volumes
            .iter()
            .enumerate()
            .any(|(at, (number, _))| *number != first + at as u64)
        {
            Some(MISSING_VOLUME)
        } else {
            None
        };
        let at = indexes[0];
        groups.push((
            at,
            match reason {
                Some(reason) => Group::Refused {
                    volumes: indexes,
                    reason: reason.to_owned(),
                },
                None => Group::Archive(indexes),
            },
        ));
    }
    groups.sort_by_key(|(at, _)| *at);
    groups.into_iter().map(|(_, g)| g).collect()
}

/// A member an archive was unpacked to, as recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberRow {
    pub file_id: String,
    pub position: i64,
    pub path: String,
    pub size: u64,
    pub sha256: String,
    /// What its check found it to be; `Err` why it is not a file.
    pub format: Result<Format, String>,
}

impl Placer {
    /// Unpacks the job's received archives not tried yet (see the module
    /// docs). `None` when `cancel` fired in the middle.
    pub(super) async fn unpack(
        &self,
        job: &str,
        cancel: &CancellationToken,
    ) -> Result<Option<()>, JobError> {
        let Some(unpacker) = self.unpacker.clone() else {
            return Ok(Some(()));
        };
        let items = self.store.items(job).await?;
        // The volumes of a set are one post's, in one folder of it.
        let mut sets: Vec<(Vec<&FileRow>, Option<String>)> = Vec::new();
        for item in &items {
            let archives: Vec<&FileRow> = own_receipts(std::slice::from_ref(item))
                .into_iter()
                .filter(|f| is_archive_receipt(f, f.format))
                .collect();
            let names: Vec<String> = archives.iter().map(|f| grouped_name(f)).collect();
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            for group in group(&names) {
                let (volumes, refused) = match group {
                    Group::Archive(volumes) => (volumes, None),
                    Group::Refused { volumes, reason } => (volumes, Some(reason)),
                };
                sets.push((volumes.into_iter().map(|i| archives[i]).collect(), refused));
            }
        }
        for (volumes, refused) in sets {
            let first = volumes[0];
            if first.unpacked_at.is_some() || first.unpack_error.is_some() {
                continue;
            }
            // A try this machine failed waits its hour, unless a worker
            // started since.
            if first.unpack_retry_at.is_some_and(|at| at > self.now()) {
                continue;
            }
            if cancel.is_cancelled() {
                return Ok(None);
            }
            let later: Vec<String> = volumes[1..].iter().map(|f| f.id.clone()).collect();
            if let Some(reason) = refused {
                self.unpack_failed(job, first, later, reason).await?;
                continue;
            }
            let parts: Option<Vec<PathBuf>> = volumes
                .iter()
                .map(|f| f.path.as_deref().map(|p| self.area.at(p)))
                .collect();
            let Some(parts) = parts else {
                continue;
            };
            let out = self.area.at(&ReceiveArea::unpack_dir(job, &first.id));
            let (unpacker, name, cancel) = (unpacker.clone(), first.name.clone(), cancel.clone());
            let unpacked = blocking(move || {
                // This thread waits in `run` until the child ends, as its
                // parent-death signal needs. `run` makes the folder anew
                // (what a kill cut short left is not this run's) and leaves
                // it only with the members.
                let unpacked = unpacker.run(&parts, &name, &out, &|| cancel.is_cancelled());
                // Each member is checked as a received file is, the ZIPs
                // among them inflating no more than the archive's limit
                // together. Its file in the folder is named by its position.
                let mut budget = MEMBERS_INFLATE_BUDGET;
                let checked = match unpacked {
                    Unpacked::Done(members) if members.is_empty() => {
                        Err(NotUnpacked::Refused(EMPTY_ARCHIVE.to_owned()))
                    }
                    Unpacked::Done(members) => members
                        .into_iter()
                        .map(|m| {
                            if cancel.is_cancelled() {
                                return Err(NotUnpacked::Cancelled);
                            }
                            let position = m.file.parse::<i64>().map_err(|_| {
                                NotUnpacked::Failed(format!(
                                    "압축을 푼 프로그램이 알 수 없는 파일 이름을 알렸어요: {}",
                                    m.file
                                ))
                            })?;
                            let base = m.path.rsplit('/').next().unwrap_or(&m.path).to_owned();
                            let format =
                                verify::check_within(&out.join(&m.file), &base, &mut budget)
                                    .map_err(|f| match f.reason == verify::INFLATE_BUDGET_SPENT {
                                        true => MEMBERS_BUDGET_SPENT.to_owned(),
                                        false => f.reason,
                                    });
                            Ok((position, m, format))
                        })
                        .collect::<Result<Vec<_>, _>>(),
                    Unpacked::Refused(refusal) => Err(NotUnpacked::Refused(refusal.to_string())),
                    Unpacked::Failed(reason) => Err(NotUnpacked::Failed(reason)),
                    Unpacked::Cancelled => Err(NotUnpacked::Cancelled),
                };
                // Nor with members that are not recorded.
                if matches!(
                    checked,
                    Err(NotUnpacked::Refused(_) | NotUnpacked::Failed(_))
                ) {
                    let _ = std::fs::remove_dir_all(&out);
                }
                checked
            })
            .await;
            match unpacked {
                Ok(members) => {
                    let rows: Vec<MemberRow> = members
                        .into_iter()
                        .map(|(position, m, format)| MemberRow {
                            file_id: first.id.clone(),
                            position,
                            path: m.path,
                            size: m.size,
                            sha256: m.sha256,
                            format,
                        })
                        .collect();
                    let count = rows.len();
                    let (id, now) = (first.id.clone(), self.now());
                    self.write(move |c| record_unpacked(c, &id, &later, &rows, now))
                        .await?;
                    let detail = match first.unpack_tries {
                        0 => format!("파일 {count}개"),
                        n => format!("파일 {count}개 ({})", attempt(n + 1)),
                    };
                    self.event(
                        job,
                        format!("{}: 압축 파일을 풀었어요", first.name),
                        Some(detail),
                    )
                    .await?;
                }
                Err(NotUnpacked::Cancelled) => return Ok(None),
                Err(NotUnpacked::Refused(reason)) => {
                    self.unpack_failed(job, first, later, reason).await?;
                }
                Err(NotUnpacked::Failed(reason)) => {
                    self.try_failed(job, first, later, reason).await?;
                }
            }
        }
        Ok(Some(()))
    }

    async fn unpack_failed(
        &self,
        job: &str,
        first: &FileRow,
        later: Vec<String>,
        reason: String,
    ) -> Result<(), JobError> {
        let (id, why, now) = (first.id.clone(), reason.clone(), self.now());
        self.write(move |c| record_refused(c, &id, &later, &why, now))
            .await?;
        // A try after ones this machine failed says which it was.
        let detail = match first.unpack_tries {
            0 => reason,
            n => format!("{reason} ({})", attempt(n + 1)),
        };
        self.event(
            job,
            format!("{}: 압축 파일을 풀지 못했어요", first.name),
            Some(detail),
        )
        .await
    }

    /// A try this machine failed, for `reason`: the archive is tried again
    /// [`UNPACK_RETRY_AFTER`] later or at a worker's start, until the
    /// [`UNPACK_TRIES`]th try, whose failure leaves it not unpacked.
    async fn try_failed(
        &self,
        job: &str,
        first: &FileRow,
        later: Vec<String>,
        reason: String,
    ) -> Result<(), JobError> {
        let tries = first.unpack_tries + 1;
        let now = self.now();
        let last = tries >= UNPACK_TRIES;
        let retry_at = (!last).then_some(now + UNPACK_RETRY_AFTER);
        let (id, why) = (first.id.clone(), reason.clone());
        self.write(move |c| record_failed_try(c, &id, &later, &why, tries, retry_at, now))
            .await?;
        let (message, detail) = match last {
            true => (
                format!("{}: 압축 파일을 풀지 못했어요", first.name),
                format!("{reason} ({})", attempt(tries)),
            ),
            false => (
                format!("{}: 압축 파일을 풀지 못해 다시 풀어요", first.name),
                format!(
                    "{reason} ({}). 1시간 뒤나 worker가 다시 시작할 때 다시 풀어요",
                    attempt(tries)
                ),
            ),
        };
        self.event(job, message, Some(detail)).await
    }
}

/// `3번 중 2번째 시도`: which of an archive's tries `n` is.
fn attempt(n: u32) -> String {
    format!("{UNPACK_TRIES}번 중 {n}번째 시도")
}

/// The receipts of the job's received items that hold their own bytes.
pub(super) fn own_receipts(items: &[ItemRow]) -> Vec<&FileRow> {
    items
        .iter()
        .filter(|i| i.state == ItemState::Done && i.unchanged_from.is_none())
        .flat_map(|i| i.files.iter())
        .filter(|f| f.state == FileState::Done && f.same_as.is_none() && f.path.is_some())
        .collect()
}

// ---------------------------------------------------------------------------
// Records

fn record_unpacked(
    c: &mut Connection,
    file_id: &str,
    later: &[String],
    members: &[MemberRow],
    now: Millis,
) -> Result<(), JobError> {
    durable(c, |c| {
        let tx = c.transaction()?;
        for m in members {
            let (format, reason) = match &m.format {
                Ok(format) => (Some(format.code()), None),
                Err(reason) => (None, Some(reason.as_str())),
            };
            tx.prepare_cached(
                "INSERT INTO subtitle_job_members
                 (file_id, position, path, size, sha256, format, reason)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?
            .execute(params![
                m.file_id,
                m.position,
                m.path,
                m.size as i64,
                m.sha256,
                format,
                reason
            ])?;
        }
        tx.prepare_cached("UPDATE subtitle_job_files SET unpacked_at = ?2, unpack_retry_at = NULL, updated_at = ?2
             WHERE id = ?1")?.execute(
            params![file_id, now],
        )?;
        mark_volumes(&tx, file_id, later, now)?;
        tx.commit()?;
        Ok(())
    })
}

/// The archive is not unpacked, for `reason` of its own. A try after ones
/// this machine failed counts with them, and no next try waits.
fn record_refused(
    c: &mut Connection,
    file_id: &str,
    later: &[String],
    reason: &str,
    now: Millis,
) -> Result<(), JobError> {
    durable(c, |c| {
        let tx = c.transaction()?;
        tx.prepare_cached(
            "UPDATE subtitle_job_files
                SET unpack_error = ?2, unpack_retry_at = NULL,
                    unpack_tries = unpack_tries + (unpack_tries > 0), updated_at = ?3
              WHERE id = ?1",
        )?
        .execute(params![file_id, reason, now])?;
        mark_volumes(&tx, file_id, later, now)?;
        tx.commit()?;
        Ok(())
    })
}

/// A try this machine failed: the `tries`th, for `reason`; the next goes
/// at `retry_at`, and with none left the archive is not unpacked
/// (`unpack_error`).
fn record_failed_try(
    c: &mut Connection,
    file_id: &str,
    later: &[String],
    reason: &str,
    tries: u32,
    retry_at: Option<Millis>,
    now: Millis,
) -> Result<(), JobError> {
    let error = retry_at.is_none().then_some(reason);
    durable(c, |c| {
        let tx = c.transaction()?;
        tx.prepare_cached(
            "UPDATE subtitle_job_files
                SET unpack_tries = ?2, unpack_failure = ?3, unpack_retry_at = ?4,
                    unpack_error = ?5, updated_at = ?6
              WHERE id = ?1",
        )?
        .execute(params![file_id, tries, reason, retry_at, error, now])?;
        mark_volumes(&tx, file_id, later, now)?;
        tx.commit()?;
        Ok(())
    })
}

fn mark_volumes(
    c: &Connection,
    first: &str,
    later: &[String],
    now: Millis,
) -> rusqlite::Result<()> {
    for id in later {
        c.prepare_cached(
            "UPDATE subtitle_job_files SET volume_of = ?2, updated_at = ?3 WHERE id = ?1",
        )?
        .execute(params![id, first, now])?;
    }
    Ok(())
}

/// The job's received archives that could not be unpacked, each as
/// `<name>: <reason>`, and whether a receipt that could have rows has none:
/// not one of them, not a later volume (its first stands for it) and not of
/// an item whose files are unchanged since an earlier receipt.
pub fn standing(c: &Connection, job_id: &str) -> rusqlite::Result<(Vec<String>, bool)> {
    let mut stmt = c.prepare_cached(
        "SELECT name || ': ' || unpack_error FROM subtitle_job_files
          WHERE job_id = ?1 AND state = 'done' AND same_as IS NULL
            AND unpack_error IS NOT NULL
          ORDER BY created_at, id",
    )?;
    let failures = stmt
        .query_map([job_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    let unplanned = c
        .prepare_cached(
            "SELECT EXISTS (
             SELECT 1 FROM subtitle_job_files f
              WHERE f.job_id = ?1 AND f.state = 'done' AND f.same_as IS NULL
                AND f.unpack_error IS NULL AND f.volume_of IS NULL
                AND NOT EXISTS (SELECT 1 FROM subtitle_job_plan p WHERE p.file_id = f.id)
                AND NOT EXISTS (SELECT 1 FROM subtitle_job_items i
                                 WHERE i.id = f.item_id AND i.unchanged_from IS NOT NULL))",
        )?
        .query_row([job_id], |r| r.get(0))?;
    Ok((failures, unplanned))
}

/// The members `file_id` was unpacked to, in order.
pub fn members(c: &Connection, file_id: &str) -> rusqlite::Result<Vec<MemberRow>> {
    let mut stmt = c.prepare_cached(
        "SELECT file_id, position, path, size, sha256, format, reason
         FROM subtitle_job_members WHERE file_id = ?1 ORDER BY position",
    )?;
    let rows = stmt.query_map([file_id], member_row)?;
    rows.collect()
}

/// The members of the job's received archives, by archive and in order.
pub fn job_members(c: &Connection, job_id: &str) -> rusqlite::Result<Vec<MemberRow>> {
    let mut stmt = c.prepare_cached(
        "SELECT m.file_id, m.position, m.path, m.size, m.sha256, m.format, m.reason
         FROM subtitle_job_members m JOIN subtitle_job_files f ON f.id = m.file_id
         WHERE f.job_id = ?1 ORDER BY m.file_id, m.position",
    )?;
    let rows = stmt.query_map([job_id], member_row)?;
    rows.collect()
}

fn member_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<MemberRow> {
    let format: Option<String> = r.get(5)?;
    let reason: Option<String> = r.get(6)?;
    Ok(MemberRow {
        file_id: r.get(0)?,
        position: r.get(1)?,
        path: r.get(2)?,
        size: r.get::<_, i64>(3)? as u64,
        sha256: r.get(4)?,
        format: match format.as_deref().and_then(Format::parse) {
            Some(format) => Ok(format),
            None => Err(reason.unwrap_or_default()),
        },
    })
}

/// Where the member of `file_id` at `path` is in the unpack folder: its
/// position.
pub fn member_position(c: &Connection, file_id: &str, path: &str) -> rusqlite::Result<Option<i64>> {
    c.prepare_cached("SELECT position FROM subtitle_job_members WHERE file_id = ?1 AND path = ?2")?
        .query_row(params![file_id, path], |r| r.get(0))
        .optional()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_archives_stand_alone() {
        assert_eq!(
            group(&["a.zip", "b.rar", "c.tar.gz", "d.7z"]),
            vec![
                Group::Archive(vec![0]),
                Group::Archive(vec![1]),
                Group::Archive(vec![2]),
                Group::Archive(vec![3]),
            ]
        );
    }

    #[test]
    fn the_volumes_of_a_split_archive_are_one_from_the_first() {
        assert_eq!(
            group(&[
                "Show.part2.rar",
                "x.zip",
                "Show.part1.rar",
                "Show.part3.rar"
            ]),
            vec![Group::Archive(vec![1]), Group::Archive(vec![2, 0, 3])]
        );
        assert_eq!(
            group(&["s.r01", "s.rar", "s.r00"]),
            vec![Group::Archive(vec![1, 2, 0])]
        );
        assert_eq!(
            group(&["s.7z.002", "s.7z.001"]),
            vec![Group::Archive(vec![1, 0])]
        );
        assert_eq!(
            group(&["S.PART01.RAR", "s.part02.rar"]),
            vec![Group::Archive(vec![0, 1])]
        );
    }

    #[test]
    fn a_set_missing_a_volume_is_not_unpacked() {
        assert_eq!(
            group(&["s.part1.rar", "s.part3.rar"]),
            vec![Group::Refused {
                volumes: vec![0, 1],
                reason: MISSING_VOLUME.to_owned()
            }]
        );
        assert_eq!(
            group(&["s.7z.002"]),
            vec![Group::Refused {
                volumes: vec![0],
                reason: NO_FIRST_VOLUME.to_owned()
            }]
        );
        assert_eq!(
            group(&["s.r00"]),
            vec![Group::Refused {
                volumes: vec![0],
                reason: NO_FIRST_VOLUME.to_owned()
            }]
        );
        // Two first volumes whose names differ in case only.
        assert_eq!(
            group(&["s.part1.rar", "S.part1.rar", "s.part2.rar"]),
            vec![Group::Refused {
                volumes: vec![0, 1, 2],
                reason: DUPLICATE_VOLUME.to_owned()
            }]
        );
    }

    #[test]
    fn spanned_zips_and_cut_rars_are_not_unpacked() {
        assert_eq!(
            group(&["s.z01", "s.zip", "t.zip"]),
            vec![
                Group::Refused {
                    volumes: vec![0, 1],
                    reason: SPANNED_ZIP.to_owned()
                },
                Group::Archive(vec![2]),
            ]
        );
        assert_eq!(
            group(&["s.rar.001", "s.rar.002"]),
            vec![Group::Refused {
                volumes: vec![0, 1],
                reason: CUT_RAR.to_owned()
            }]
        );
    }

    #[test]
    fn documents_that_are_zips_are_no_archives() {
        assert!(!is_archive("notes.docx", Some(Format::Zip)));
        assert!(is_archive("pack", Some(Format::Zip)));
        assert!(is_archive("pack.7z.002", Some(Format::Other)));
        assert!(!is_archive("a.ass", Some(Format::Ass)));
    }
}
