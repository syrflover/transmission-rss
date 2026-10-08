//! Cleaning a work's stored files, one stored subtitle at a time
//! (`docs/specs/library.md`, 보관 파일의 정리; `docs/specs/subtitles.md`,
//! 보관본과 적용본 and 폰트; `docs/specs/jobs.md`, 체크포인트와 중단 복구;
//! `migrations/jobs/cleanup.sql` in `trss-core`).
//!
//! # What a person may clean
//!
//! [`cleanable`] lists the work's stored subtitles a person did not clean yet
//! and that have no applied copy beside a video, each of one kind
//! ([`CleanKind`], the first that holds): `past` (지난 수정본: an applied
//! copy of it was taken away), `awaiting_video` (영상 대기, as
//! [`super::records::StoredOnly::awaiting_video`]), `unplaced` (회차에 붙지
//! 않음) and `stored` (보관만 함). None of the work is cleanable while the
//! work folder is not a folder on disk ([`FOLDER_AWAY`]: the caller looks,
//! before any transaction), so a cleanup never hides a stored copy whose
//! files it could not remove. Otherwise one is not cleanable now, with the
//! first reason that holds ([`Cleanable::blocked`]), while a job may still
//! use it:
//! a plan row of it of a job that is pending or running ([`RUNNING_JOB`]),
//! held ([`HELD_JOB`]), or waiting with the row not settled
//! ([`WAITING_JOB`]); a replacement plan that puts it beside a video, open or
//! approved ([`AWAITING_APPROVAL`]); or a file effect of a row of it that did
//! not end ([`UNFINISHED_EFFECT`]). A row that waits for its video
//! (`no_video`) does not keep it: a person may clean what waits for a video.
//!
//! The files that go with it ([`Cleanable::with`]) are its subtitle's file
//! and each font, attachment and companion file linked to it, but those
//! something else still uses ([`Cleanable::kept`], with why, the first that
//! holds): another stored subtitle not cleaned with the same file
//! ([`SAME_FILE`]) or linked to it ([`OTHER_SUBTITLE`]); a package none of
//! whose files is a subtitle (a fonts-only upload, kept on purpose) that has
//! it ([`FONTS_ONLY`]: by its entries, not its stored subtitles, since a
//! subtitle received again with the same bytes reuses the first package's
//! stored subtitle and leaves its own package with none); a plan row that
//! kept it of a job not ended that may still link it: one with an item not
//! received yet, a subtitle row not stored yet, or a stored subtitle not
//! cleaned whose links are not known (`links_known = 0`: a run cut between
//! the store and the link, or a row given a stored subtitle of its own link
//! after the link step, [`records::relink`], which the next run links)
//! ([`JOB_FILE`]); a receipt of a job not ended that
//! uses it instead of receiving an unchanged Drive font, until its row is
//! stored ([`super::unchanged`], the same reason); an effect not ended that
//! aims at its path (the same reason). A job past its links (waiting for a video
//! with every item received and its stored subtitles linked, or put back in
//! line by a cleanup) keeps no file this way, and no link is ever made to a
//! removed file
//! ([`records::link`]). The list and the worker's look again use the same
//! rules. An asset already removed is in neither list, and a file the
//! records do not have is never in one: nothing is removed by its name.
//!
//! # Confirming
//!
//! A person confirms one stored subtitle with the files the dialog showed
//! ([`ask`], the web), in one transaction: refused while it is not cleanable,
//! and when the files that would go are not the ones shown. The cleanup is
//! recorded `asked` with each file shown `named`, and the stored subtitle is
//! cleaned (`cleaned_at`): from then no list shows it and no job picks,
//! reuses or applies it ([`super::records`]). Its rows that waited for their
//! video are settled as stored ([`CLEANED_WHILE_WAITING`]); a job that waited
//! for its video with no such row left goes back in line ([`REFLECT`]), and
//! its next run settles it. The rules above leave no other row of it to be
//! applied (no job that runs, is held or waits on it, no replacement); a row
//! of it that comes to be applied all the same ends stored
//! ([`NOT_APPLIED`]), never failed.
//!
//! # Removing
//!
//! The worker takes each asked cleanup up ([`super::Placer::run_cleanups`])
//! in the task that runs the jobs, under the worker lock, before the jobs it
//! runs: no store, reuse or link of a job runs between its steps, and the web
//! makes none. That is how a check that a file is unused and its removal are
//! kept together (the spec's serialization of a font's cleanup).
//!
//! 1. One transaction ([`intend`]) looks at each named file again with the
//!    rules above: `intended` when nothing uses it, else `kept` with why. A
//!    file that was not named is never removed.
//! 2. Each intended file is looked for where it is kept: the work folder
//!    for a subtitle or a font (no folder, or no `.trss/subtitles` in it as
//!    on an empty mount point: `held`, [`NO_FOLDER`]), the app data folder
//!    for the rest (no `subtitle-files` in it: `held`, [`NO_APP_DATA`]). A file that is not there counts as removed.
//!    One whose length and SHA-256 are the recorded ones is removed and its
//!    folder synced; any other is `held` ([`CHANGED`]) and left as it is.
//!    The folders of a removed file left empty go too, up to the folder its
//!    kind is kept in (`.trss/subtitles`, `subtitle-files`); one with
//!    anything in it stays, and a folder that cannot go holds nothing.
//!    The folder may also go away after the confirmation: then the pass
//!    holds ([`NO_FOLDER`]).
//! 3. One transaction ([`finish`]) records the removed ones `done`, their
//!    assets `removed_at`, and the cleanup `done`, or `held` with the first
//!    held file's reason.
//!
//! # Restart
//!
//! | Record | On disk | Then |
//! | --- | --- | --- |
//! | `asked`, files `named` | anything | step 1 onward |
//! | `asked`, files `intended` | the recorded file | looked at again (step 1), then removed |
//! | `asked`, files `intended` | no file | removed already: `done` |
//! | `asked`, files `intended` | another file | `held` |
//!
//! Nothing is written `done` before its file is found gone, and a file is
//! only removed after the transaction that found it unused.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use trss_core::Millis;

use super::{
    blocking,
    files::{self, facts},
    records::{self, durable},
    Placer,
};
use crate::{
    area::sync_dir,
    model::{AssetKind, SubtitleFormat},
    store::JobError,
};

/// A plan row of the stored subtitle is of a job that is pending or running.
pub const RUNNING_JOB: &str = "진행 중인 작업이 이 보관본을 써요";
/// … of a job that is held.
pub const HELD_JOB: &str = "보류한 작업이 이 보관본을 써요";
/// … not settled, of a job that waits (회차 확인 among others).
pub const WAITING_JOB: &str = "회차 확인을 기다리는 작업이 이 보관본을 써요";
/// A replacement plan open or approved puts it beside a video.
pub const AWAITING_APPROVAL: &str = "교체 승인을 기다리는 작업이 이 보관본을 써요";
/// A file effect of a row of it did not end.
pub const UNFINISHED_EFFECT: &str = "보관·적용이 끝나지 않은 작업이 이 보관본을 써요";
/// It has an applied copy beside a video.
pub const APPLIED: &str = "영상 옆에 적용한 보관본은 정리할 수 없어요";

/// A file another stored subtitle not cleaned is made of.
pub const SAME_FILE: &str = "다른 보관본이 같은 파일을 써요";
/// A file another stored subtitle not cleaned is linked to.
pub const OTHER_SUBTITLE: &str = "다른 자막도 이 파일을 써요";
/// A file of a package none of whose entries is a subtitle (fonts and
/// attachments kept on purpose).
pub const FONTS_ONLY: &str = "폰트만 올린 묶음에 들어 있어요";
/// A file a job not ended kept, or an effect not ended aims at.
pub const JOB_FILE: &str = "진행 중인 작업이 이 파일을 써요";

/// The work folder of a subtitle's or a font's file is not there.
pub const NO_FOLDER: &str = "작품 폴더를 찾지 못했어요";
/// The work folder is not a folder on disk now (a share not mounted, a work
/// moved): nothing of the work is cleaned until it is, so a cleanup does not
/// hide a stored copy whose files it cannot remove.
pub const FOLDER_AWAY: &str = "작품 폴더를 찾지 못해 지금은 정리할 수 없어요";
/// The app data folder of another file is not known.
pub const NO_APP_DATA: &str = "앱 데이터 폴더를 찾지 못했어요";
/// The file is not the one recorded.
pub const CHANGED: &str = "기록과 내용이 달라 지우지 않았어요";

/// The note of a row that waited for its video when its stored subtitle was
/// cleaned.
pub const CLEANED_WHILE_WAITING: &str = "영상을 기다리다 정리했어요";
/// The note of a row to apply whose stored subtitle was cleaned: it is
/// settled as stored, not applied and not failed ([`super::Placer`]).
pub const NOT_APPLIED: &str = "정리한 보관본이라 적용하지 않았어요";
/// The note of a job that waited for its video and goes back in line.
pub const REFLECT: &str = "정리한 보관본을 반영해요";

/// What a cleanable stored subtitle is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanKind {
    /// 지난 수정본: an applied copy of it was taken away.
    Past,
    /// 영상 대기: a job applies it once the episode's video comes.
    AwaitingVideo,
    /// 회차에 붙지 않음.
    Unplaced,
    /// 보관만 함.
    Stored,
}

impl CleanKind {
    pub fn code(self) -> &'static str {
        match self {
            CleanKind::Past => "past",
            CleanKind::AwaitingVideo => "awaiting_video",
            CleanKind::Unplaced => "unplaced",
            CleanKind::Stored => "stored",
        }
    }
}

/// A file that would go with a stored subtitle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileGoing {
    pub id: String,
    pub name: String,
    pub kind: AssetKind,
    pub size: u64,
}

/// A file that stays when a stored subtitle is cleaned, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStaying {
    pub id: String,
    pub name: String,
    pub kind: AssetKind,
    pub reason: &'static str,
}

/// A stored subtitle a person may clean (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleanable {
    pub id: String,
    /// Its file's name.
    pub name: String,
    pub season: u32,
    pub episode: Option<i64>,
    pub creator: Option<String>,
    pub format: SubtitleFormat,
    /// Its file's length.
    pub size: u64,
    pub stored_at: Millis,
    pub kind: CleanKind,
    /// Why it is not cleanable now.
    pub blocked: Option<&'static str>,
    /// The files that would be removed with it now, its own first.
    pub with: Vec<FileGoing>,
    pub kept: Vec<FileStaying>,
}

/// A cleanup the worker has not finished (`asked`) or held (`held`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleaning {
    pub id: String,
    /// The stored subtitle's file name.
    pub name: String,
    /// `asked` or `held`.
    pub state: String,
    pub reason: Option<String>,
}

/// One asset as the rules read it.
struct AssetRow {
    id: String,
    kind: AssetKind,
    relative_path: String,
    size: u64,
}

fn file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_owned()
}

fn exists(c: &Connection, sql: &str, p: impl rusqlite::Params) -> rusqlite::Result<bool> {
    c.prepare_cached(&format!("SELECT EXISTS ({sql})"))?
        .query_row(p, |r| r.get(0))
}

/// Why the stored subtitle is not cleanable now, if it is not.
fn blocked(c: &Connection, stored: &str) -> rusqlite::Result<Option<&'static str>> {
    let of_jobs = |states: &str| {
        format!(
            "SELECT 1 FROM subtitle_job_plan p JOIN subtitle_jobs j ON j.id = p.job_id
              WHERE p.stored_id = ?1 AND j.state IN ({states})"
        )
    };
    if exists(c, &of_jobs("'pending', 'running'"), [stored])? {
        return Ok(Some(RUNNING_JOB));
    }
    if exists(c, &of_jobs("'held'"), [stored])? {
        return Ok(Some(HELD_JOB));
    }
    // A replacement waits as a waiting job too; name the approval.
    if exists(
        c,
        "SELECT 1 FROM subtitle_replacements
          WHERE stored_id = ?1 AND state IN ('open', 'approved')",
        [stored],
    )? {
        return Ok(Some(AWAITING_APPROVAL));
    }
    if exists(
        c,
        &format!("{} AND p.outcome IS NULL", of_jobs("'waiting'")),
        [stored],
    )? {
        return Ok(Some(WAITING_JOB));
    }
    if exists(
        c,
        "SELECT 1 FROM subtitle_file_effects e
           JOIN subtitle_job_plan p ON p.job_id = e.job_id AND p.position = e.position
          WHERE p.stored_id = ?1 AND e.state IN ('intended', 'prepared', 'set_aside', 'held')",
        [stored],
    )? {
        return Ok(Some(UNFINISHED_EFFECT));
    }
    Ok(None)
}

/// What uses the asset besides the stored subtitle `stored`, if anything
/// does: why it stays (see the module docs).
fn in_use(
    c: &Connection,
    work_id: &str,
    asset: &str,
    stored: &str,
) -> rusqlite::Result<Option<&'static str>> {
    if exists(
        c,
        "SELECT 1 FROM subtitle_stored
          WHERE subtitle_asset_id = ?1 AND id <> ?2 AND cleaned_at IS NULL",
        [asset, stored],
    )? {
        return Ok(Some(SAME_FILE));
    }
    if exists(
        c,
        "SELECT 1 FROM subtitle_stored_assets l JOIN subtitle_stored s ON s.id = l.stored_id
          WHERE l.asset_id = ?1 AND s.id <> ?2 AND s.cleaned_at IS NULL",
        [asset, stored],
    )? {
        return Ok(Some(OTHER_SUBTITLE));
    }
    if exists(
        c,
        "SELECT 1 FROM subtitle_package_entries e
          WHERE e.asset_id = ?1
            AND NOT EXISTS (SELECT 1 FROM subtitle_package_entries o
                              JOIN subtitle_assets a ON a.id = o.asset_id
                             WHERE o.package_id = e.package_id AND a.kind = 'subtitle')",
        [asset],
    )? {
        return Ok(Some(FONTS_ONLY));
    }
    // A job not ended that kept the file and may still link it: one with a
    // subtitle not stored yet, one stored whose links it has not made (a
    // run cut between the store and the link), or an item not received yet
    // (its post's rows, planned when it is, link the post's files). A job
    // past its links, such as one that waits for a video with every item
    // received or that a cleanup put back in line, makes no link any more.
    if exists(
        c,
        "SELECT 1 FROM subtitle_job_plan p JOIN subtitle_jobs j ON j.id = p.job_id
          WHERE p.asset_id = ?1 AND j.state NOT IN ('done', 'failed', 'partial')
            AND (EXISTS (SELECT 1 FROM subtitle_job_items i
                          WHERE i.job_id = j.id
                            AND i.state IN ('pending', 'running', 'waiting', 'held'))
                 OR EXISTS (SELECT 1 FROM subtitle_job_plan u
                          WHERE u.job_id = j.id AND u.kind = 'subtitle'
                            AND u.action <> 'drop' AND u.stored_id IS NULL
                            AND u.outcome IS NULL)
                 OR EXISTS (SELECT 1 FROM subtitle_job_plan u
                              JOIN subtitle_stored s ON s.id = u.stored_id
                             WHERE u.job_id = j.id AND s.links_known = 0
                               AND s.cleaned_at IS NULL))",
        [asset],
    )? {
        return Ok(Some(JOB_FILE));
    }
    // A job not ended that did not receive a Drive font because the work
    // keeps it ([`super::unchanged`]): it uses the font when its row is
    // stored, and until then no plan row names the font.
    if exists(
        c,
        "SELECT 1 FROM subtitle_job_files f JOIN subtitle_jobs j ON j.id = f.job_id
          WHERE f.unchanged_asset = ?1 AND f.state = 'done'
            AND j.state NOT IN ('done', 'failed', 'partial')
            AND NOT EXISTS (SELECT 1 FROM subtitle_job_plan p
                             WHERE p.file_id = f.id AND p.asset_id IS NOT NULL)",
        [asset],
    )? {
        return Ok(Some(JOB_FILE));
    }
    // An effect not ended that aims at its path, or takes a file off it.
    if exists(
        c,
        "SELECT 1 FROM subtitle_assets a, subtitle_file_effects e
           JOIN subtitle_jobs j ON j.id = e.job_id
          WHERE a.id = ?1 AND j.work_id = ?2
            AND e.state IN ('intended', 'prepared', 'set_aside', 'held')
            AND (lower(e.target) = lower(a.relative_path)
                 OR lower(e.source) = lower(a.relative_path))",
        [asset, work_id],
    )? {
        return Ok(Some(JOB_FILE));
    }
    Ok(None)
}

/// The stored subtitle's files that are not removed: its own, then the ones
/// linked to it (fonts, attachments, companion files).
fn files_of(c: &Connection, stored: &str) -> rusqlite::Result<Vec<AssetRow>> {
    let mut stmt = c.prepare_cached(
        "SELECT a.id, a.kind, a.relative_path, a.byte_size, 0
           FROM subtitle_stored s JOIN subtitle_assets a ON a.id = s.subtitle_asset_id
          WHERE s.id = ?1 AND a.removed_at IS NULL
         UNION ALL
         SELECT a.id, a.kind, a.relative_path, a.byte_size,
                CASE l.role WHEN 'font' THEN 1 WHEN 'attachment' THEN 2 ELSE 3 END
           FROM subtitle_stored_assets l JOIN subtitle_assets a ON a.id = l.asset_id
          WHERE l.stored_id = ?1 AND a.removed_at IS NULL
          ORDER BY 5, 3",
    )?;
    let rows = stmt.query_map([stored], |r| {
        Ok(AssetRow {
            id: r.get(0)?,
            kind: r.get(1)?,
            relative_path: r.get(2)?,
            size: r.get::<_, i64>(3)? as u64,
        })
    })?;
    rows.collect()
}

/// The work's cleanable stored subtitles (`only`: that one), by season,
/// episode (none last) and when they were stored.
/// `folder_there` says whether the work folder is a folder on disk now (the
/// caller looks, outside any transaction).
fn entries(
    c: &Connection,
    work_id: &str,
    only: Option<&str>,
    folder_there: bool,
) -> rusqlite::Result<Vec<Cleanable>> {
    let heads: Vec<Cleanable> = {
        let mut stmt = c.prepare_cached(
            "SELECT s.id, a.relative_path, s.season, s.episode, s.creator, s.format, a.byte_size,
                    s.stored_at,
                    EXISTS (SELECT 1 FROM subtitle_applied ap
                             WHERE ap.stored_id = s.id AND ap.removed_at IS NOT NULL),
                    EXISTS (SELECT 1 FROM subtitle_job_plan p
                              JOIN subtitle_jobs j ON j.id = p.job_id
                             WHERE p.stored_id = s.id AND p.action = 'apply'
                               AND p.outcome = 'no_video'
                               AND (j.state = 'waiting' AND j.wait IN ('video', 'subtitle')
                                    OR j.state IN ('partial', 'pending', 'running')))
               FROM subtitle_stored s JOIN subtitle_assets a ON a.id = s.subtitle_asset_id
              WHERE s.work_id = ?1 AND s.cleaned_at IS NULL AND (?2 IS NULL OR s.id = ?2)
                AND NOT EXISTS (SELECT 1 FROM subtitle_applied ap
                                 WHERE ap.stored_id = s.id AND ap.removed_at IS NULL)
              ORDER BY s.season, s.episode IS NULL, s.episode, s.stored_at, s.id",
        )?;
        let rows = stmt.query_map(params![work_id, only], |r| {
            let path: String = r.get(1)?;
            let episode: Option<i64> = r.get(3)?;
            let (past, waiting): (bool, bool) = (r.get(8)?, r.get(9)?);
            Ok(Cleanable {
                id: r.get(0)?,
                name: file_name(&path),
                season: r.get(2)?,
                episode,
                creator: r.get(4)?,
                format: r.get(5)?,
                size: r.get::<_, i64>(6)? as u64,
                stored_at: r.get(7)?,
                kind: match (past, waiting, episode) {
                    (true, ..) => CleanKind::Past,
                    (_, true, _) => CleanKind::AwaitingVideo,
                    (_, _, None) => CleanKind::Unplaced,
                    _ => CleanKind::Stored,
                },
                blocked: None,
                with: Vec::new(),
                kept: Vec::new(),
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut entries = Vec::with_capacity(heads.len());
    for mut entry in heads {
        entry.blocked = match folder_there {
            true => blocked(c, &entry.id)?,
            false => Some(FOLDER_AWAY),
        };
        for asset in files_of(c, &entry.id)? {
            match in_use(c, work_id, &asset.id, &entry.id)? {
                Some(reason) => entry.kept.push(FileStaying {
                    name: file_name(&asset.relative_path),
                    id: asset.id,
                    kind: asset.kind,
                    reason,
                }),
                None => entry.with.push(FileGoing {
                    name: file_name(&asset.relative_path),
                    id: asset.id,
                    kind: asset.kind,
                    size: asset.size,
                }),
            }
        }
        entries.push(entry);
    }
    Ok(entries)
}

/// The work's cleanable stored subtitles (see the module docs);
/// `folder_there`: whether the work folder is a folder on disk now.
pub fn cleanable(
    c: &Connection,
    work_id: &str,
    folder_there: bool,
) -> rusqlite::Result<Vec<Cleanable>> {
    entries(c, work_id, None, folder_there)
}

/// The work's cleanups the worker has not finished, and the held ones, in
/// the order they were asked.
pub fn cleaning(c: &Connection, work_id: &str) -> rusqlite::Result<Vec<Cleaning>> {
    let mut stmt = c.prepare_cached(
        "SELECT k.id, a.relative_path, k.state, k.reason
           FROM subtitle_cleanups k JOIN subtitle_stored s ON s.id = k.stored_id
                JOIN subtitle_assets a ON a.id = s.subtitle_asset_id
          WHERE k.work_id = ?1 AND k.state IN ('asked', 'held')
          ORDER BY k.asked_at, k.id",
    )?;
    let rows = stmt.query_map([work_id], |r| {
        Ok(Cleaning {
            id: r.get(0)?,
            name: file_name(&r.get::<_, String>(1)?),
            state: r.get(2)?,
            reason: r.get(3)?,
        })
    })?;
    rows.collect()
}

/// What a work keeps and what a person may clean of it, for its page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkFiles {
    /// The length of its files that are not removed ([`total`]).
    pub total: u64,
    pub cleanable: Vec<Cleanable>,
    pub cleaning: Vec<Cleaning>,
}

/// The length of the work's files that are not removed.
pub fn total(c: &Connection, work_id: &str) -> rusqlite::Result<u64> {
    c.prepare_cached(
        "SELECT coalesce(sum(byte_size), 0) FROM subtitle_assets
          WHERE work_id = ?1 AND removed_at IS NULL",
    )?
    .query_row([work_id], |r| r.get::<_, i64>(0).map(|n| n as u64))
}

/// How many files of one kind a work keeps, and their length: `subtitle`,
/// `font` or `attachment` (attachments, companion files and any other).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindUse {
    pub kind: &'static str,
    pub count: u64,
    pub size: u64,
}

/// What one work keeps ([`storage`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkStorage {
    pub work_id: String,
    pub total: u64,
    pub kinds: Vec<KindUse>,
    /// How many of its stored subtitles a person may clean now.
    pub cleanable: usize,
}

/// What each work with a file not removed keeps, by work ID.
/// `folder_there` says of a work ID whether its folder is a folder on disk
/// now.
pub fn storage(
    c: &Connection,
    folder_there: &dyn Fn(&str) -> bool,
) -> rusqlite::Result<Vec<WorkStorage>> {
    let rows: Vec<(String, String, i64, i64)> = {
        let mut stmt = c.prepare_cached(
            "SELECT work_id,
                    CASE kind WHEN 'subtitle' THEN 'subtitle' WHEN 'font' THEN 'font'
                              ELSE 'attachment' END AS k,
                    count(*), sum(byte_size)
               FROM subtitle_assets WHERE removed_at IS NULL
              GROUP BY work_id, k
              ORDER BY work_id, CASE k WHEN 'subtitle' THEN 0 WHEN 'font' THEN 1 ELSE 2 END",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let mut works: Vec<WorkStorage> = Vec::new();
    for (work, kind, count, size) in rows {
        if works.last().is_none_or(|w| w.work_id != work) {
            works.push(WorkStorage {
                work_id: work.clone(),
                total: 0,
                kinds: Vec::new(),
                cleanable: 0,
            });
        }
        let one = works.last_mut().expect("pushed");
        one.total += size as u64;
        one.kinds.push(KindUse {
            kind: match kind.as_str() {
                "subtitle" => "subtitle",
                "font" => "font",
                _ => "attachment",
            },
            count: count as u64,
            size: size as u64,
        });
    }
    for work in &mut works {
        work.cleanable = cleanable(c, &work.work_id, folder_there(&work.work_id))?
            .iter()
            .filter(|e| e.blocked.is_none())
            .count();
    }
    Ok(works)
}

/// What came of a person's confirming a cleanup ([`ask`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    /// Recorded; the cleanup's ID. The worker removes the files.
    Asked(String),
    /// No such stored subtitle of the work, or one cleaned already.
    NotFound,
    /// Why it is not cleanable now, for the person.
    Refused(&'static str),
    /// The files that would go are not the ones named: the entry as it is
    /// now.
    Changed(Box<Cleanable>),
}

/// A person's confirming of cleaning the work's stored subtitle `stored_id`
/// with the files `assets` the dialog showed would go (see the module docs).
/// One synced transaction; `folder_there` is whether the work folder was a
/// folder on disk when the caller looked, just before.
pub fn ask(
    c: &mut Connection,
    work_id: &str,
    stored_id: &str,
    assets: &[String],
    folder_there: bool,
    now: Millis,
) -> Result<Asked, JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let applied: Option<bool> = tx
            .prepare_cached(
                "SELECT EXISTS (SELECT 1 FROM subtitle_applied ap
                                 WHERE ap.stored_id = s.id AND ap.removed_at IS NULL)
                   FROM subtitle_stored s
                  WHERE s.id = ?1 AND s.work_id = ?2 AND s.cleaned_at IS NULL",
            )?
            .query_row(params![stored_id, work_id], |r| r.get(0))
            .optional()?;
        match applied {
            None => return Ok(Asked::NotFound),
            Some(true) => return Ok(Asked::Refused(APPLIED)),
            Some(false) => {}
        }
        let Some(entry) = entries(&tx, work_id, Some(stored_id), folder_there)?
            .into_iter()
            .next()
        else {
            return Ok(Asked::NotFound);
        };
        if let Some(reason) = entry.blocked {
            return Ok(Asked::Refused(reason));
        }
        let mut named: Vec<&str> = assets.iter().map(String::as_str).collect();
        named.sort_unstable();
        named.dedup();
        let mut going: Vec<&str> = entry.with.iter().map(|f| f.id.as_str()).collect();
        going.sort_unstable();
        if named != going {
            return Ok(Asked::Changed(Box::new(entry)));
        }
        let id = uuid::Uuid::new_v4().to_string();
        tx.prepare_cached(
            "INSERT INTO subtitle_cleanups (id, work_id, stored_id, state, asked_at, updated_at)
             VALUES (?1, ?2, ?3, 'asked', ?4, ?4)",
        )?
        .execute(params![id, work_id, stored_id, now])?;
        for asset in &going {
            tx.prepare_cached(
                "INSERT INTO subtitle_asset_removals (cleanup_id, asset_id, state)
                 VALUES (?1, ?2, 'named')",
            )?
            .execute(params![id, asset])?;
        }
        tx.prepare_cached("UPDATE subtitle_stored SET cleaned_at = ?2 WHERE id = ?1")?
            .execute(params![stored_id, now])?;
        // What waited for its video is settled as stored, and a job that
        // waited for nothing else settles at its next run.
        let jobs: Vec<String> = {
            let mut stmt = tx.prepare_cached(
                "SELECT DISTINCT job_id FROM subtitle_job_plan
                  WHERE stored_id = ?1 AND action = 'apply' AND outcome = 'no_video'",
            )?;
            let rows = stmt.query_map([stored_id], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        tx.prepare_cached(
            "UPDATE subtitle_job_plan SET outcome = 'stored', note = ?2, updated_at = ?3
              WHERE stored_id = ?1 AND action = 'apply' AND outcome = 'no_video'",
        )?
        .execute(params![stored_id, CLEANED_WHILE_WAITING, now])?;
        for job in jobs {
            let queued = tx
                .prepare_cached(
                    "UPDATE subtitle_jobs
                    SET state = 'pending', wait = NULL, finished_at = NULL, note = ?2,
                        state_at = ?3, updated_at = ?3
                  WHERE id = ?1 AND state = 'waiting' AND wait = 'video'
                    AND NOT EXISTS (SELECT 1 FROM subtitle_job_plan
                                     WHERE job_id = ?1 AND action = 'apply'
                                       AND outcome = 'no_video')",
                )?
                .execute(params![job, REFLECT, now])?;
            if queued == 1 {
                tx.prepare_cached(
                    "INSERT INTO subtitle_job_events (job_id, at, message, detail)
                     VALUES (?1, ?2, ?3, ?4)",
                )?
                .execute(params![job, now, REFLECT, entry.name])?;
            }
        }
        tx.commit()?;
        Ok(Asked::Asked(id))
    })
}

/// Whether a cleanup waits for the worker.
pub fn any_asked(c: &Connection) -> rusqlite::Result<bool> {
    exists(
        c,
        "SELECT 1 FROM subtitle_cleanups WHERE state = 'asked'",
        [],
    )
}

/// The folder of each work with a file not removed ([`records::work_folder`]:
/// none for a work or watch folder the library no longer has), for a look at
/// which are on disk now.
pub fn work_folders(c: &Connection) -> rusqlite::Result<Vec<(String, Option<String>)>> {
    let works: Vec<String> = {
        let mut stmt = c.prepare_cached(
            "SELECT DISTINCT work_id FROM subtitle_assets WHERE removed_at IS NULL ORDER BY 1",
        )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    works
        .into_iter()
        .map(|work| {
            let folder = records::work_folder(c, &work)?;
            Ok((work, folder))
        })
        .collect()
}

/// Whether a person cleaned the stored subtitle `stored_id`.
pub fn cleaned(c: &Connection, stored_id: &str) -> rusqlite::Result<bool> {
    exists(
        c,
        "SELECT 1 FROM subtitle_stored WHERE id = ?1 AND cleaned_at IS NOT NULL",
        [stored_id],
    )
}

/// The cleanups that wait for the worker, the oldest first.
pub fn asked(c: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = c.prepare_cached(
        "SELECT id FROM subtitle_cleanups WHERE state = 'asked' ORDER BY asked_at, id",
    )?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    rows.collect()
}

/// A file a cleanup is to remove, as recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    pub asset_id: String,
    /// `work` (relative to the work folder) or `app_data`.
    pub base: String,
    pub relative_path: String,
    pub size: u64,
    pub sha256: String,
}

/// What [`intend`] found of an asked cleanup: its work and the files to
/// remove.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pass {
    pub cleanup_id: String,
    pub work_id: String,
    pub removals: Vec<Removal>,
}

/// Step 1 of removing (see the module docs): each named or intended file of
/// the asked cleanup is looked at again, `intended` when nothing uses it,
/// else `kept` with why (`done` when another cleanup removed it already).
/// One synced transaction; `None` for a cleanup that is not asked.
pub fn intend(c: &mut Connection, cleanup_id: &str, now: Millis) -> Result<Option<Pass>, JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let found: Option<(String, String)> = tx
            .prepare_cached(
                "SELECT work_id, stored_id FROM subtitle_cleanups
                  WHERE id = ?1 AND state = 'asked'",
            )?
            .query_row([cleanup_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let Some((work_id, stored)) = found else {
            return Ok(None);
        };
        let named: Vec<(Removal, bool)> = {
            let mut stmt = tx.prepare_cached(
                "SELECT a.id, a.base, a.relative_path, a.byte_size, a.sha256,
                        a.removed_at IS NOT NULL
                   FROM subtitle_asset_removals m JOIN subtitle_assets a ON a.id = m.asset_id
                  WHERE m.cleanup_id = ?1 AND m.state IN ('named', 'intended')
                  ORDER BY a.relative_path",
            )?;
            let rows = stmt.query_map([cleanup_id], |r| {
                Ok((
                    Removal {
                        asset_id: r.get(0)?,
                        base: r.get(1)?,
                        relative_path: r.get(2)?,
                        size: r.get::<_, i64>(3)? as u64,
                        sha256: r.get(4)?,
                    },
                    r.get(5)?,
                ))
            })?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut removals = Vec::new();
        for (removal, removed) in named {
            let (state, reason) = match removed {
                true => ("done", None),
                false => match in_use(&tx, &work_id, &removal.asset_id, &stored)? {
                    Some(reason) => ("kept", Some(reason)),
                    None => ("intended", None),
                },
            };
            tx.prepare_cached(
                "UPDATE subtitle_asset_removals SET state = ?3, reason = ?4
                  WHERE cleanup_id = ?1 AND asset_id = ?2",
            )?
            .execute(params![cleanup_id, removal.asset_id, state, reason])?;
            if state == "intended" {
                removals.push(removal);
            }
        }
        tx.prepare_cached("UPDATE subtitle_cleanups SET updated_at = ?2 WHERE id = ?1")?
            .execute(params![cleanup_id, now])?;
        tx.commit()?;
        Ok(Some(Pass {
            cleanup_id: cleanup_id.to_owned(),
            work_id,
            removals,
        }))
    })
}

/// What came of removing one intended file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Removed {
    /// The file is gone.
    Done,
    /// It was not removed: why.
    Held(String),
}

/// Step 3 of removing (see the module docs): the files found gone are `done`
/// and their assets removed, the others `held`; the cleanup is `done`, or
/// `held` with the first held file's reason. One synced transaction.
pub fn finish(
    c: &mut Connection,
    cleanup_id: &str,
    results: &[(String, Removed)],
    now: Millis,
) -> Result<(), JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (asset, result) in results {
            match result {
                Removed::Done => {
                    tx.prepare_cached(
                        "UPDATE subtitle_asset_removals SET state = 'done', reason = NULL
                          WHERE cleanup_id = ?1 AND asset_id = ?2 AND state = 'intended'",
                    )?
                    .execute(params![cleanup_id, asset])?;
                    tx.prepare_cached(
                        "UPDATE subtitle_assets SET removed_at = ?2
                          WHERE id = ?1 AND removed_at IS NULL",
                    )?
                    .execute(params![asset, now])?;
                }
                Removed::Held(reason) => {
                    tx.prepare_cached(
                        "UPDATE subtitle_asset_removals SET state = 'held', reason = ?3
                          WHERE cleanup_id = ?1 AND asset_id = ?2 AND state = 'intended'",
                    )?
                    .execute(params![cleanup_id, asset, reason])?;
                }
            }
        }
        let held = results.iter().find_map(|(_, r)| match r {
            Removed::Held(reason) => Some(reason.clone()),
            Removed::Done => None,
        });
        tx.prepare_cached(
            "UPDATE subtitle_cleanups
                SET state = CASE WHEN ?2 IS NULL THEN 'done' ELSE 'held' END, reason = ?2,
                    updated_at = ?3
              WHERE id = ?1 AND state = 'asked'
                AND NOT EXISTS (SELECT 1 FROM subtitle_asset_removals
                                 WHERE cleanup_id = ?1 AND state IN ('named', 'intended'))",
        )?
        .execute(params![cleanup_id, held, now])?;
        tx.commit()?;
        Ok(())
    })
}

/// Removes each folder of the removed file's path that is empty now, the
/// deepest first (the creator folder, then the work's in the app data
/// folder), up to but not the folder where its kind is kept
/// ([`files::SUBTITLES_DIR`] in the work folder, [`files::APP_FILES_DIR`] in
/// the app data folder), and syncs the folder above each one removed.
/// `remove_dir` leaves a folder with anything in it (a file the app did not
/// record keeps its folder); a folder that stays ends the climb, and is no
/// failure of the cleanup.
fn remove_empty_folders(folder: &Path, removal: &Removal) {
    let base = match removal.base.as_str() {
        "work" => files::SUBTITLES_DIR,
        _ => files::APP_FILES_DIR,
    };
    let parts: Vec<&str> = removal.relative_path.split('/').collect();
    let kept = base.split('/').count();
    if parts.len() <= kept + 1 || parts[..kept] != *base.split('/').collect::<Vec<_>>() {
        return;
    }
    for depth in (kept + 1..parts.len()).rev() {
        let dir = files::within(folder, &parts[..depth].join("/"));
        if std::fs::remove_dir(&dir).is_err() {
            return;
        }
        if let Some(parent) = dir.parent() {
            // The removal stands without the sync; nothing to hold.
            let _ = sync_dir(parent);
        }
    }
}

/// Step 2 of removing one file (see the module docs), in `folder`.
fn remove_recorded(folder: &Path, removal: &Removal) -> Removed {
    let path = files::within(folder, &removal.relative_path);
    match facts(&path) {
        Ok(None) => {
            // Removed already (a pass cut after it): its folders as below.
            remove_empty_folders(folder, removal);
            Removed::Done
        }
        Ok(Some((size, sha256, _))) if size == removal.size && sha256 == removal.sha256 => {
            let removed = files::remove_known(&path).and_then(|()| match path.parent() {
                Some(parent) => sync_dir(parent),
                None => Ok(()),
            });
            match removed {
                Ok(()) => {
                    remove_empty_folders(folder, removal);
                    Removed::Done
                }
                Err(err) => Removed::Held(format!("파일을 지우지 못했어요: {err}")),
            }
        }
        Ok(Some(_)) => Removed::Held(CHANGED.to_owned()),
        Err(err) => Removed::Held(format!("파일을 확인하지 못했어요: {err}")),
    }
}

impl Placer {
    /// Whether a cleanup waits for the worker.
    pub async fn has_cleanups(&self) -> Result<bool, JobError> {
        self.read(any_asked).await
    }

    /// Carries out each asked cleanup (see the module docs). The caller
    /// holds the worker lock and runs no job meanwhile. Returns how many
    /// ended, done or held.
    /// One that fails (the database refused a step) is told and stays
    /// asked; the others go on.
    pub async fn run_cleanups(&self) -> Result<usize, JobError> {
        let ids = self.read(asked).await?;
        let mut ended = 0;
        for id in ids {
            match self.clean(&id).await {
                Ok(true) => ended += 1,
                Ok(false) => {}
                Err(err) => eprintln!("Stored files: cannot carry out cleanup {id}: {err}"),
            }
        }
        Ok(ended)
    }

    /// Carries out the asked cleanup `id`; whether it ended.
    pub async fn clean(&self, id: &str) -> Result<bool, JobError> {
        let (cid, now) = (id.to_owned(), self.now());
        let Some(pass) = self.write(move |c| intend(c, &cid, now)).await? else {
            return Ok(false);
        };
        let work_folder = match pass.removals.iter().any(|r| r.base == "work") {
            true => {
                let work = pass.work_id.clone();
                // The folder kept files are under must be there too: an
                // empty mount point is a folder whose files all look gone.
                self.read(move |c| records::work_folder(c, &work))
                    .await?
                    .filter(|f| files::within(Path::new(f), files::SUBTITLES_DIR).is_dir())
            }
            false => None,
        };
        let app_data = self
            .area
            .app_data()
            .filter(|root| files::within(root, files::APP_FILES_DIR).is_dir())
            .map(Path::to_path_buf);
        let mut results = Vec::with_capacity(pass.removals.len());
        for removal in pass.removals {
            let folder = match removal.base.as_str() {
                "work" => work_folder.as_deref().map(Path::new).ok_or(NO_FOLDER),
                _ => app_data.as_deref().ok_or(NO_APP_DATA),
            };
            let result = match folder {
                Err(reason) => Removed::Held(reason.to_owned()),
                Ok(folder) => {
                    let (folder, one) = (folder.to_path_buf(), removal.clone());
                    blocking(move || remove_recorded(&folder, &one)).await
                }
            };
            results.push((removal.asset_id, result));
        }
        let held = results
            .iter()
            .filter(|(_, r)| matches!(r, Removed::Held(_)))
            .count();
        let (cid, now) = (id.to_owned(), self.now());
        self.write(move |c| finish(c, &cid, &results, now)).await?;
        println!(
            "Stored files: cleanup {id} {}",
            match held {
                0 => "done".to_owned(),
                n => format!("held ({n} not removed)"),
            }
        );
        Ok(true)
    }
}
