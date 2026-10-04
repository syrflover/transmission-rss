//! Storing what a job received and applying it beside its video
//! (`docs/specs/subtitles.md`, 보관본과 적용본 and 파일의 회차;
//! `docs/specs/jobs.md`, 체크포인트와 중단 복구).
//!
//! # Order
//!
//! After its items are received, every run of a job goes on here
//! ([`Placer::run`]):
//!
//! 1. The effects an earlier start left unfinished are compared with the
//!    disk (the table below).
//! 2. The analysis makes the job's plan ([`records::PlanRow`]) from each
//!    post's receipts not planned yet: what each file is and which episode
//!    of the job's season it goes on ([`episode::of_candidate`]). A post
//!    whose package this build cannot analyse yet (more than one file, an
//!    archive, a font) leaves the job waiting with the reason (`자막 대기`):
//!    a later build takes it up when the worker starts.
//!    Uploads and find jobs wait for a person's placement first and are not
//!    placed here yet.
//! 3. Each row to keep is stored: copied into the work folder's
//!    `.trss/subtitles/<creator>/<name>` ([`store_name`]), or, when a file of
//!    that name (or a numbered one) with the same bytes is stored already,
//!    that one is used. Another file under the name (another case of it
//!    too), or another job's effect under way to it, numbers the new one
//!    (`<stem> (2).<ext>`). A work folder that is not there (a share not
//!    mounted, a work moved) leaves the job waiting (`영상 대기`): nothing
//!    was written, and the worker tries again when it starts.
//! 4. A receipt every row of which is stored leaves the receive area.
//! 5. A row whose episode has one video and no subtitle is applied: the
//!    stored file is copied beside the video under its stem. An episode with
//!    a subtitle, or with another job's apply under way, keeps it and the
//!    row is stored only, as is one without a video; a row whose episode a
//!    person has to say waits for them (`회차 확인 필요`). An earlier applied
//!    copy recorded at the path is recorded as removed: the rename replaced
//!    nothing, so a person removed it.
//!
//! # One effect
//!
//! Storing and applying are each an effect on one file ([`files`]):
//!
//! 1. `intended`: the effect's ID, its temporary file `.trss/tmp/<ID>` in the
//!    work folder, its target and the bytes' length and SHA-256, before
//!    anything is written.
//! 2. The bytes are copied to the temporary file, synced and read back.
//! 3. `prepared`: the temporary file's object.
//! 4. Right before an apply is published, the episode is looked at again: a
//!    subtitle that came meanwhile stays, nothing is applied and the row is
//!    held for a person, as it is when its name is taken at the rename.
//! 5. A rename that replaces nothing publishes the file, and the folders are
//!    synced.
//! 6. `done`, with what it made (the asset and stored subtitle, or the
//!    applied copy) in the same transaction.
//!
//! # Restart
//!
//! | Record | On disk | Then |
//! | --- | --- | --- |
//! | `intended` | anything at the temporary file | it is removed (only this effect writes there), `abandoned`, and the row is done anew |
//! | `prepared` | the temporary file, recorded object, length and hash | published (from step 4) |
//! | `prepared` | no temporary file, the target is the recorded object, length and hash | the rename happened: synced, then `done` |
//! | `prepared` | anything else | `held` |
//!
//! A held effect holds its row and the job: the runner does not take it up
//! again by itself, and its files stay as they are. The effects under way of
//! a job held for never ending a run are held too, and so is an effect whose
//! published file the database refuses to record. A held effect no longer
//! keeps its target from other effects.

pub mod episode;
pub mod files;
pub mod records;

use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::{Path, PathBuf},
};

use tokio_util::sync::CancellationToken;
use trss_core::{Clock, Db, Millis};
use trss_subtitles::verify::{self, Format};

use crate::{
    area::{safe_name, ReceiveArea},
    follow::Follow,
    model::{
        AssetKind, EffectKind, EffectState, FileState, ItemState, Outcome, PlanAction, StepKind,
        StepState, SubtitleFormat,
    },
    runner::episode_label,
    store::{FileRow, ItemRow, JobError, JobStore, FIND, UPLOAD},
};
use episode::{named, Named, Target};
use files::{Copied, Published};
use records::{Effect, JobFacts, Kept, NewApplied, NewStored, Placed, PlanRow};

/// The creator folder of a subtitle whose creator nobody named.
pub const UNKNOWN_CREATOR: &str = "제작자 알 수 없음";

/// What a package this build cannot analyse waits for.
pub const ARCHIVE_LATER: &str = "압축 파일을 풀어 분석하는 일은 아직 할 수 없어요";
pub const PACKAGE_LATER: &str =
    "자막이 여럿이거나 폰트·첨부가 함께 온 묶음의 분석은 아직 할 수 없어요";

/// The extensions of subtitles the app keeps but does not apply by itself.
const OTHER_SUBTITLES: [&str; 8] = ["ssa", "vtt", "sup", "sub", "idx", "ttml", "dfxp", "lrc"];
/// The extensions a subtitle beside a video may have, for "the episode has a
/// subtitle".
const SUBTITLE_EXTENSIONS: [&str; 8] = ["ass", "ssa", "srt", "smi", "vtt", "sup", "sub", "idx"];
const FONT_EXTENSIONS: [&str; 5] = ["ttf", "otf", "ttc", "woff", "woff2"];
const ARCHIVE_EXTENSIONS: [&str; 8] = ["zip", "rar", "7z", "gz", "bz2", "xz", "tar", "tgz"];

/// How many names past a taken one a store tries.
const MAX_NAMES: usize = 1000;
/// How many times a store takes the next name when its target is taken
/// between choosing and publishing.
const MAX_RETARGETS: usize = 5;

/// What a run of the placement leaves for the job's state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Placement {
    /// Why a package of the job waits for a later build to analyse it.
    pub unanalysed: Option<String>,
    /// Why the job's files wait for its work folder to be there again.
    pub no_folder: Option<String>,
    /// Why a received file needs a person before it can be placed.
    pub blocked: Option<String>,
}

/// How a job's plan stands, for its state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Standing {
    pub rows: usize,
    /// Rows whose episode a person has to say.
    pub questions: usize,
    /// The first held row's reason.
    pub held: Option<String>,
    pub failed: usize,
    /// The first failed row's reason.
    pub failure: Option<String>,
}

/// The name a store takes ([`Placer::choose_name`]).
enum Choice {
    New(String),
    /// A stored file of this name holds the bytes already: the asset.
    Reuse(String, String),
    /// Every name tried is taken.
    Full,
    /// The creator folder could not be read.
    Unreadable(io::Error),
}

/// What a member of a package is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Member {
    Subtitle(SubtitleFormat),
    Font,
    Archive,
    Other,
}

/// `name`'s extension in lower case.
fn extension(name: &str) -> Option<String> {
    let base = name.rsplit('/').next().unwrap_or(name);
    base.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, ext)| ext.to_lowercase())
}

/// What a received file is, from its checked `format` and its name.
fn member(name: &str, format: Format) -> Member {
    let ext = extension(name);
    let ext = ext.as_deref().unwrap_or("");
    match format {
        // SSA opens as ASS does, but is not applied by itself.
        Format::Ass if ext == "ssa" => Member::Subtitle(SubtitleFormat::Other),
        Format::Ass => Member::Subtitle(SubtitleFormat::Ass),
        Format::Srt => Member::Subtitle(SubtitleFormat::Srt),
        Format::Smi => Member::Subtitle(SubtitleFormat::Smi),
        Format::Zip => Member::Archive,
        Format::Other if OTHER_SUBTITLES.contains(&ext) => Member::Subtitle(SubtitleFormat::Other),
        Format::Other if FONT_EXTENSIONS.contains(&ext) => Member::Font,
        Format::Other if ARCHIVE_EXTENSIONS.contains(&ext) => Member::Archive,
        Format::Other => Member::Other,
    }
}

/// `creator` as the name of a folder every share shows the same: a safe name
/// ([`safe_name`]) without the characters Windows and SMB refuse, without
/// trailing dots and spaces and not a reserved device name; [`UNKNOWN_CREATOR`]
/// for none.
pub fn creator_folder(creator: Option<&str>) -> String {
    let named = creator.map(str::trim).filter(|c| !c.is_empty());
    let Some(creator) = named else {
        return UNKNOWN_CREATOR.to_owned();
    };
    let cleaned: String = safe_name(creator)
        .chars()
        .map(|c| match c {
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .collect();
    let cleaned = cleaned.trim_end_matches(['.', ' ']);
    if cleaned.is_empty() {
        return UNKNOWN_CREATOR.to_owned();
    }
    let stem = cleaned
        .split('.')
        .next()
        .unwrap_or(cleaned)
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    match reserved {
        true => format!("{cleaned}_"),
        false => cleaned.to_owned(),
    }
}

/// The name a file is stored under in its creator folder: the one of
/// `name`, `<stem> (2).<ext>`, … that `same` says holds these bytes already,
/// else the first that no other file takes, whatever the case of its
/// letters (`taken`, lower-cased). `None` when every name tried is taken.
pub fn store_name(
    name: &str,
    taken: &[String],
    same: impl Fn(&str) -> bool,
) -> Option<(String, bool)> {
    let safe = safe_name(name);
    let names: Vec<String> = crate::area::name_candidates(&safe)
        .take(MAX_NAMES)
        .collect();
    let same_one = names.iter().find(|n| same(&n.to_lowercase()));
    match same_one {
        Some(name) => Some((name.clone(), true)),
        None => names
            .into_iter()
            .find(|n| !taken.contains(&n.to_lowercase()))
            .map(|name| (name, false)),
    }
}

/// The text encoding of a subtitle's bytes when it shows: a BOM, or UTF-8
/// that reads as such. `None`: not known.
fn encoding_of(bytes: &[u8]) -> Option<String> {
    match bytes {
        [0xEF, 0xBB, 0xBF, ..] => Some("utf-8".to_owned()),
        [0xFF, 0xFE, ..] => Some("utf-16le".to_owned()),
        [0xFE, 0xFF, ..] => Some("utf-16be".to_owned()),
        bytes if std::str::from_utf8(bytes).is_ok() => Some("utf-8".to_owned()),
        _ => None,
    }
}

/// Runs blocking file work off the runtime.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    match tokio::task::spawn_blocking(work).await {
        Ok(value) => value,
        Err(err) => std::panic::resume_unwind(err.into_panic()),
    }
}

/// How a row reads in the log: its episode, else its name.
fn row_label(row: &PlanRow) -> String {
    match &row.placed {
        Some(placed) => episode_label(&placed.episode.to_string()),
        None => row.name.clone(),
    }
}

/// The video's folder (`""` for the work folder) and stem.
fn video_parts(video: &str) -> (&str, &str) {
    let (dir, file) = video.rsplit_once('/').unwrap_or(("", video));
    let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file);
    (dir, stem)
}

fn joined(dir: &str, name: &str) -> String {
    match dir.is_empty() {
        true => name.to_owned(),
        false => format!("{dir}/{name}"),
    }
}

/// Stores and applies what jobs received. Cheap to clone.
#[derive(Clone)]
pub struct Placer {
    db: Db,
    store: JobStore,
    area: ReceiveArea,
    clock: Clock,
    follow: Follow,
}

impl Placer {
    pub fn new(store: JobStore, area: ReceiveArea, clock: Clock) -> Placer {
        let db = store.db().clone();
        Placer {
            follow: Follow::new(db.clone()),
            db,
            store,
            area,
            clock,
        }
    }

    fn now(&self) -> Millis {
        (self.clock)()
    }

    async fn read<T: Send + 'static>(
        &self,
        f: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
    ) -> Result<T, JobError> {
        self.db.run(move |c| Ok::<_, JobError>(f(c)?)).await
    }

    async fn write<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut rusqlite::Connection) -> Result<T, JobError> + Send + 'static,
    ) -> Result<T, JobError> {
        self.db.run(f).await
    }

    async fn event(
        &self,
        job: &str,
        message: String,
        detail: Option<String>,
    ) -> Result<(), JobError> {
        self.store.event(job, message, detail, self.now()).await
    }

    /// How the job's plan stands.
    pub async fn standing(&self, job: &str) -> Result<Standing, JobError> {
        let id = job.to_owned();
        let rows = self.read(move |c| records::plan(c, &id)).await?;
        let first = |o: Outcome| {
            rows.iter()
                .find(|r| r.outcome == Some(o))
                .map(|r| r.note.clone().unwrap_or_default())
        };
        Ok(Standing {
            rows: rows.len(),
            questions: rows
                .iter()
                .filter(|r| r.question.is_some() && r.outcome != Some(Outcome::Dropped))
                .count(),
            held: first(Outcome::Held),
            failed: rows
                .iter()
                .filter(|r| r.outcome == Some(Outcome::Failed))
                .count(),
            failure: first(Outcome::Failed),
        })
    }

    /// One run of the placement of the job (see the module docs). `None`
    /// when `cancel` fired in the middle.
    pub async fn run(
        &self,
        job: &str,
        cancel: &CancellationToken,
    ) -> Result<Option<Placement>, JobError> {
        let id = job.to_owned();
        let Some(facts) = self.read(move |c| records::job_facts(c, &id)).await? else {
            return Ok(Some(Placement::default()));
        };
        if facts.origin == UPLOAD || facts.origin == FIND {
            return Ok(Some(Placement::default()));
        }
        let (Some(work_id), Some(season)) = (facts.work_id.clone(), facts.season) else {
            return Ok(Some(Placement::default()));
        };
        let items = self.store.items(job).await?;

        let id = job.to_owned();
        for effect in self
            .read(move |c| records::unfinished_effects(c, &id))
            .await?
        {
            self.recover(&facts, &items, effect).await?;
        }

        let mut placement = self.analyse(job, &facts, &work_id, season, &items).await?;

        let id = job.to_owned();
        let rows = self.read(move |c| records::plan(c, &id)).await?;
        let to_store: Vec<&PlanRow> = rows
            .iter()
            .filter(|r| {
                r.action != PlanAction::Drop && r.stored_id.is_none() && r.outcome.is_none()
            })
            .collect();
        let wid = work_id.clone();
        let folder = self.read(move |c| records::work_folder(c, &wid)).await?;
        let Some(folder) = folder.filter(|f| Path::new(f).is_dir()) else {
            // Nothing was written, so nothing needs a person: the job waits
            // for the folder (a share not mounted yet, a work moved).
            if !to_store.is_empty() {
                placement.no_folder =
                    Some("작품 폴더를 찾지 못해 받은 자막을 보관하지 못했어요".to_owned());
            }
            return Ok(Some(placement));
        };
        if !to_store.is_empty() {
            let now = self.now();
            self.store.set_stage(job, StepKind::Store, now).await?;
            self.store.begin_step(job, StepKind::Store, now).await?;
        }
        for row in to_store {
            if cancel.is_cancelled() {
                return Ok(None);
            }
            self.store_row(&facts, &items, &folder, row).await?;
        }
        self.end_store_step(job).await?;
        self.clear(job).await?;

        let id = job.to_owned();
        let rows = self.read(move |c| records::plan(c, &id)).await?;
        let to_apply: Vec<&PlanRow> = rows
            .iter()
            .filter(|r| {
                r.action == PlanAction::Apply
                    && r.stored_id.is_some()
                    && r.outcome.is_none()
                    && r.question.is_none()
            })
            .collect();
        if !to_apply.is_empty() {
            let now = self.now();
            self.store.set_stage(job, StepKind::Apply, now).await?;
            self.store.begin_step(job, StepKind::Apply, now).await?;
        }
        for row in to_apply {
            if cancel.is_cancelled() {
                return Ok(None);
            }
            self.apply_row(&facts, &folder, row).await?;
        }
        self.end_apply_step(job).await?;
        Ok(Some(placement))
    }

    // -----------------------------------------------------------------------
    // Analysis

    /// Plans each post's receipts not planned yet; why a package waits for
    /// a later build or a receipt for a person, if one does.
    async fn analyse(
        &self,
        job: &str,
        facts: &JobFacts,
        work_id: &str,
        season: u32,
        items: &[ItemRow],
    ) -> Result<Placement, JobError> {
        let id = job.to_owned();
        let mut planned: Vec<String> = self
            .read(move |c| records::plan(c, &id))
            .await?
            .into_iter()
            .map(|r| r.file_id)
            .collect();
        let receipts: HashMap<&str, &FileRow> = items
            .iter()
            .flat_map(|i| i.files.iter())
            .map(|f| (f.id.as_str(), f))
            .collect();
        // A post's items and the receipts they received, the shared ones
        // once.
        let mut posts: BTreeMap<&str, (Vec<&ItemRow>, Vec<&FileRow>)> = BTreeMap::new();
        for item in items
            .iter()
            .filter(|i| i.state == ItemState::Done && i.unchanged_from.is_none())
        {
            let (candidates, files) = posts.entry(item.post_url.as_str()).or_default();
            candidates.push(item);
            for file in item.files.iter().filter(|f| f.state == FileState::Done) {
                let own = match &file.same_as {
                    Some(first) => receipts.get(first.as_str()).copied(),
                    None => Some(file),
                };
                if let Some(own) = own {
                    if !files.iter().any(|f| f.id == own.id) {
                        files.push(own);
                    }
                }
            }
        }

        let mut later = None;
        let mut blocked = None;
        let mut mappings = None;
        let mut total = None;
        for (_, (candidates, files)) in posts {
            if files.is_empty() || files.iter().all(|f| planned.contains(&f.id)) {
                continue;
            }
            let mut members = Vec::new();
            for file in &files {
                let format = match file.format {
                    Some(format) => format,
                    // Bytes gone or no longer passing the checks: no later
                    // build changes that.
                    None => match self.check_again(file).await {
                        Some(format) => format,
                        None => {
                            blocked.get_or_insert_with(|| {
                                "받은 파일의 형식을 다시 확인하지 못했어요".to_owned()
                            });
                            continue;
                        }
                    },
                };
                members.push((*file, format, member(&file.name, format)));
            }
            if members.len() != files.len() {
                continue;
            }
            let [(file, _, Member::Subtitle(format))] = members.as_slice() else {
                let archive = members.iter().any(|(_, _, m)| *m == Member::Archive);
                later.get_or_insert_with(|| {
                    match archive {
                        true => ARCHIVE_LATER,
                        false => PACKAGE_LATER,
                    }
                    .to_owned()
                });
                continue;
            };
            if mappings.is_none() {
                mappings = Some(
                    self.follow
                        .mappings(work_id, season)
                        .await
                        .map_err(follow_error)?,
                );
                total = self
                    .follow
                    .season_facts(work_id, season)
                    .await
                    .map_err(follow_error)?
                    .total;
            }
            let mapping = facts
                .source_id
                .as_ref()
                .and_then(|s| mappings.as_ref().and_then(|m| m.get(s)));
            let name = match &file.folder {
                Some(folder) => format!("{folder}/{}", file.name),
                None => file.name.clone(),
            };
            let base = file.name.as_str();
            // The candidate whose episode the file is: the one when it is
            // alone, else the one the file's name says.
            let alone = candidates.len() == 1;
            let decided: Vec<(&ItemRow, Target)> = candidates
                .iter()
                .map(|c| {
                    (
                        *c,
                        episode::of_candidate(&c.episode, mapping, base, total, alone),
                    )
                })
                .collect();
            let chosen: Vec<&(&ItemRow, Target)> = decided
                .iter()
                .filter(|(_, t)| matches!(t, Target::Episode { .. }))
                .collect();
            let (item, target) = match chosen.as_slice() {
                [one] => (one.0, one.1.clone()),
                _ => match decided.first() {
                    Some((item, Target::Ask(reason))) if alone => {
                        (*item, Target::Ask(reason.clone()))
                    }
                    Some((item, _)) => (
                        *item,
                        Target::Ask("이 파일 하나를 여러 회차가 함께 받았어요".to_owned()),
                    ),
                    None => continue,
                },
            };
            let (placed, question) = match target {
                Target::Episode {
                    episode,
                    assignment,
                    basis,
                } => (
                    Some(Placed {
                        episode,
                        assignment,
                        basis,
                    }),
                    None,
                ),
                Target::Ask(reason) => (None, Some(reason)),
            };
            let applies = format.extension().is_some();
            let (Some(size), Some(sha256)) = (file.size, file.sha256.clone()) else {
                continue;
            };
            let row = PlanRow {
                job_id: job.to_owned(),
                position: 0,
                file_id: file.id.clone(),
                member: None,
                name,
                kind: AssetKind::Subtitle,
                format: Some(*format),
                size,
                sha256,
                item_id: Some(item.id),
                anissia_episode: Some(item.episode.clone()),
                attachment_episode: match named(base) {
                    Named::One(key) => Some(key),
                    _ => None,
                },
                // An unsupported format is kept on its episode but asks
                // nothing: it is not applied whatever the answer.
                question: question.filter(|_| applies),
                placed,
                action: match applies {
                    true => PlanAction::Apply,
                    false => PlanAction::Store,
                },
                stored_id: None,
                outcome: None,
                note: (!applies).then(|| "자동으로 적용하지 않는 형식이라 보관만 해요".to_owned()),
                applied_id: None,
            };
            let id = job.to_owned();
            let now = self.now();
            let added = self
                .write(move |c| records::add_plan(c, &id, vec![row], now))
                .await?;
            // A receipt another post shares is planned once.
            planned.extend(added.iter().map(|r| r.file_id.clone()));
            for row in added {
                if let Some(question) = &row.question {
                    self.event(
                        job,
                        format!("{}: 회차를 확인해야 해요", row.name),
                        Some(question.clone()),
                    )
                    .await?;
                }
            }
        }
        Ok(Placement {
            unanalysed: later,
            no_folder: None,
            blocked,
        })
    }

    /// The format of a receipt recorded before formats were: checked again.
    async fn check_again(&self, file: &FileRow) -> Option<Format> {
        let path = self.area.at(file.path.as_deref()?);
        let name = file.name.clone();
        blocking(move || verify::check(&path, &name).ok()).await
    }

    // -----------------------------------------------------------------------
    // Storing

    async fn store_row(
        &self,
        facts: &JobFacts,
        items: &[ItemRow],
        folder: &str,
        row: &PlanRow,
    ) -> Result<(), JobError> {
        let job = row.job_id.as_str();
        let Some(receipt) = items
            .iter()
            .flat_map(|i| i.files.iter())
            .find(|f| f.id == row.file_id)
        else {
            return self
                .fail_row(row, "받은 파일의 기록을 찾지 못했어요".to_owned())
                .await;
        };
        let Some(path) = &receipt.path else {
            return self
                .fail_row(row, "받은 파일의 경로 기록이 없어요".to_owned())
                .await;
        };
        let source = self.area.at(path);

        let name = match self.choose_name(facts, folder, row, None).await? {
            Choice::New(name) => name,
            Choice::Reuse(name, asset) => {
                return self.reuse(facts, items, row, folder, (name, asset)).await
            }
            Choice::Full => {
                return self
                    .fail_row(row, "보관할 이름을 정하지 못했어요".to_owned())
                    .await
            }
            Choice::Unreadable(err) => {
                return self
                    .fail_row(row, format!("보관 폴더를 읽지 못했어요: {err}"))
                    .await
            }
        };
        let relative = format!("{}/{}", self.creator_dir(facts, folder).await?, name);

        let effect = Effect {
            id: uuid::Uuid::new_v4().to_string(),
            job_id: job.to_owned(),
            position: row.position,
            kind: EffectKind::Store,
            state: EffectState::Intended,
            folder: folder.to_owned(),
            temp: format!("{}/{}", files::TEMP_DIR, uuid::Uuid::new_v4()),
            target: relative,
            video: None,
            size: row.size,
            sha256: row.sha256.clone(),
            object: None,
            reason: None,
        };
        let e = effect.clone();
        let now = self.now();
        self.write(move |c| records::intend(c, &e, now)).await?;
        let temp = files::within(Path::new(folder), &effect.temp);
        let (size, sha) = (effect.size, effect.sha256.clone());
        match blocking(move || files::copy_to_temp(&source, &temp, size, &sha)).await {
            Copied::Ready(object) => {
                let (id, o, now) = (effect.id.clone(), object.clone(), self.now());
                self.write(move |c| records::prepared(c, &id, &o, now))
                    .await?;
                let effect = Effect {
                    state: EffectState::Prepared,
                    object: Some(object),
                    ..effect
                };
                self.finish_store(facts, items, effect).await
            }
            Copied::SourceChanged(reason) => {
                self.hold_effect(
                    &effect,
                    &format!("받은 파일이 기록과 달라 보관하지 못했어요 ({reason})"),
                )
                .await
            }
            Copied::Failed(err) if err.kind() == io::ErrorKind::NotFound => {
                self.hold_effect(&effect, "수신 영역에 받은 파일이 없어 보관하지 못했어요")
                    .await
            }
            Copied::Failed(err) => {
                let reason = format!("보관본을 쓰지 못했어요: {err}");
                self.fail_effect(&effect, reason).await
            }
        }
    }

    /// The creator folder of the job's files, relative to the work folder:
    /// a folder of the same name in another case the work has already is
    /// used, so one creator has one folder on every share.
    async fn creator_dir(&self, facts: &JobFacts, _folder: &str) -> Result<String, JobError> {
        let wanted = creator_folder(facts.creator.as_deref());
        let work = facts.work_id.clone().unwrap_or_default();
        let assets = self
            .read(move |c| {
                records::work_assets_under(c, &work, &format!("{}/", files::SUBTITLES_DIR))
            })
            .await?;
        let lower = wanted.to_lowercase();
        let existing = assets.iter().find_map(|a| {
            let rest = a
                .relative_path
                .strip_prefix(&format!("{}/", files::SUBTITLES_DIR))?;
            let (dir, _) = rest.split_once('/')?;
            (dir.to_lowercase() == lower).then(|| dir.to_owned())
        });
        Ok(format!(
            "{}/{}",
            files::SUBTITLES_DIR,
            existing.unwrap_or(wanted)
        ))
    }

    /// Uses the stored file `name` (the asset `asset`) that holds the row's
    /// bytes already.
    async fn reuse(
        &self,
        facts: &JobFacts,
        items: &[ItemRow],
        row: &PlanRow,
        folder: &str,
        (name, asset): (String, String),
    ) -> Result<(), JobError> {
        let relative = format!("{}/{name}", self.creator_dir(facts, folder).await?);
        let at = files::within(Path::new(folder), &relative);
        self.record_stored(facts, items, row, Kept::Reused(asset), &at)
            .await?;
        self.event(
            &row.job_id,
            format!("{}: 같은 내용의 보관본이 있어 그것을 써요", row_label(row)),
            Some(relative),
        )
        .await
    }

    /// The name the row's file is stored under in its creator folder, or the
    /// stored file under one of its names that holds the same bytes already.
    /// The names of files there, registered or not, and the targets of other
    /// effects under way (but `except`) are taken.
    async fn choose_name(
        &self,
        facts: &JobFacts,
        folder: &str,
        row: &PlanRow,
        except: Option<&str>,
    ) -> Result<Choice, JobError> {
        let dir = self.creator_dir(facts, folder).await?;
        let work = facts.work_id.clone().unwrap_or_default();
        let prefix = format!("{dir}/");
        let assets = self
            .read(move |c| records::work_assets_under(c, &work, &prefix))
            .await?;
        let on_disk = {
            let path = files::within(Path::new(folder), &dir);
            blocking(move || files::names_in(&path)).await
        };
        let on_disk = match on_disk {
            Ok(names) => names,
            Err(err) => return Ok(Choice::Unreadable(err)),
        };
        // Names held by a file, registered or not, lower-cased.
        let mut taken: Vec<String> = on_disk.iter().map(|n| n.to_lowercase()).collect();
        let (f, e) = (folder.to_owned(), except.map(str::to_owned));
        let busy = self
            .read(move |c| records::busy_targets(c, &f, e.as_deref()))
            .await?;
        let lower_dir = format!("{}/", dir.to_lowercase());
        taken.extend(busy.iter().filter_map(|t| {
            t.strip_prefix(&lower_dir)
                .filter(|n| !n.contains('/'))
                .map(str::to_owned)
        }));
        let mut same_bytes: HashMap<String, String> = HashMap::new();
        for asset in &assets {
            let Some(name) = asset.relative_path.rsplit('/').next() else {
                continue;
            };
            let lower = name.to_lowercase();
            if asset.size == row.size && asset.sha256 == row.sha256 {
                // Used only while the file is there with these bytes.
                let path = files::within(Path::new(folder), &asset.relative_path);
                let ok = blocking(move || files::facts(&path))
                    .await
                    .ok()
                    .flatten()
                    .is_some_and(|(n, s, _)| n == row.size && s == row.sha256);
                if ok {
                    same_bytes.insert(lower.clone(), asset.id.clone());
                }
            }
            taken.push(lower);
        }
        let base = row.name.rsplit('/').next().unwrap_or(&row.name);
        Ok(
            match store_name(base, &taken, |lower| same_bytes.contains_key(lower)) {
                Some((name, true)) => {
                    let asset = same_bytes[&name.to_lowercase()].clone();
                    Choice::Reuse(name, asset)
                }
                Some((name, false)) => Choice::New(name),
                None => Choice::Full,
            },
        )
    }

    /// Publishes a prepared store, taking the next free name when another
    /// file took its target meanwhile.
    async fn finish_store(
        &self,
        facts: &JobFacts,
        items: &[ItemRow],
        mut effect: Effect,
    ) -> Result<(), JobError> {
        let Some(row) = self.row_of(&effect).await? else {
            return Ok(());
        };
        let folder = PathBuf::from(&effect.folder);
        let temp = files::within(&folder, &effect.temp);
        for _ in 0..=MAX_RETARGETS {
            let target = files::within(&folder, &effect.target);
            let from = temp.clone();
            match blocking(move || files::publish(&from, &target)).await {
                Published::Done => return self.stored_by(facts, items, &row, &effect).await,
                Published::Occupied => {
                    let name = match self
                        .choose_name(facts, &effect.folder, &row, Some(&effect.id))
                        .await?
                    {
                        Choice::New(name) => name,
                        Choice::Reuse(name, asset) => {
                            return self
                                .reuse_instead(facts, items, &row, &effect, &temp, (name, asset))
                                .await
                        }
                        Choice::Full => {
                            return self
                                .hold_effect(&effect, "보관할 이름을 정하지 못해 보류했어요")
                                .await
                        }
                        Choice::Unreadable(err) => {
                            let reason = format!("보관 폴더를 읽지 못해 보류했어요: {err}");
                            return self.hold_effect(&effect, &reason).await;
                        }
                    };
                    let dir = self.creator_dir(facts, &effect.folder).await?;
                    effect.target = format!("{dir}/{name}");
                    let (id, target, now) = (effect.id.clone(), effect.target.clone(), self.now());
                    self.write(move |c| records::retarget(c, &id, &target, now))
                        .await?;
                }
                Published::Failed(err) => {
                    return self.unsure(&effect, &temp, err).await;
                }
            }
        }
        self.hold_effect(
            &effect,
            "보관할 이름이 계속 다른 파일에 먼저 쓰여 보류했어요",
        )
        .await
    }

    /// A prepared store whose target was taken by a stored file of the same
    /// bytes: its temporary file goes and that file is used.
    async fn reuse_instead(
        &self,
        facts: &JobFacts,
        items: &[ItemRow],
        row: &PlanRow,
        effect: &Effect,
        temp: &Path,
        found: (String, String),
    ) -> Result<(), JobError> {
        let temp = temp.to_owned();
        if let Err(err) = blocking(move || files::remove_known(&temp)).await {
            let reason = format!("임시 파일을 지우지 못했어요: {err}");
            return self.hold_effect(effect, &reason).await;
        }
        let (id, now) = (effect.id.clone(), self.now());
        self.write(move |c| records::end_effect(c, &id, EffectState::Abandoned, None, now))
            .await?;
        self.reuse(facts, items, row, &effect.folder, found).await
    }

    /// A publish that failed: nothing moved when the temporary file is still
    /// there, so the effect failed and its file goes; otherwise what happened
    /// is not known and the effect is held.
    async fn unsure(&self, effect: &Effect, temp: &Path, err: io::Error) -> Result<(), JobError> {
        let temp = temp.to_owned();
        let still = blocking({
            let temp = temp.clone();
            move || files::facts(&temp)
        })
        .await;
        match still {
            Ok(Some((_, _, object))) if Some(&object) == effect.object.as_ref() => {
                let _ = blocking(move || files::remove_known(&temp)).await;
                let reason = match effect.kind {
                    EffectKind::Store => format!("보관본을 쓰지 못했어요: {err}"),
                    EffectKind::Apply => format!("적용본을 쓰지 못했어요: {err}"),
                };
                self.fail_effect(effect, reason).await
            }
            _ => {
                self.hold_effect(
                    effect,
                    &format!("파일을 공개한 결과를 확인하지 못했어요: {err}"),
                )
                .await
            }
        }
    }

    /// Records a published store.
    async fn stored_by(
        &self,
        facts: &JobFacts,
        items: &[ItemRow],
        row: &PlanRow,
        effect: &Effect,
    ) -> Result<(), JobError> {
        let target = files::within(Path::new(&effect.folder), &effect.target);
        let kept = Kept::New {
            effect_id: effect.id.clone(),
            relative_path: effect.target.clone(),
        };
        match self.record_stored(facts, items, row, kept, &target).await {
            Ok(_) => {}
            Err(err) if is_constraint(&err) => return self.unrecorded(effect, err).await,
            Err(err) => return Err(err),
        }
        self.event(
            &row.job_id,
            format!("{}: 보관했어요", row_label(row)),
            Some(effect.target.clone()),
        )
        .await
    }

    async fn record_stored(
        &self,
        facts: &JobFacts,
        items: &[ItemRow],
        row: &PlanRow,
        kept: Kept,
        bytes_at: &Path,
    ) -> Result<String, JobError> {
        let item = row.item_id.and_then(|id| items.iter().find(|i| i.id == id));
        let observation = match item.and_then(|i| i.observation_id) {
            Some(id) => self.read(move |c| records::observation(c, id)).await?,
            None => None,
        };
        let path = bytes_at.to_owned();
        let encoding =
            blocking(move || std::fs::read(path).ok().and_then(|b| encoding_of(&b))).await;
        let file_id = row.file_id.clone();
        let received_at = self
            .read(move |c| {
                c.query_row(
                    "SELECT updated_at FROM subtitle_job_files WHERE id = ?1",
                    [file_id],
                    |r| r.get::<_, Millis>(0),
                )
            })
            .await
            .ok();
        let new = NewStored {
            work_id: facts.work_id.clone().unwrap_or_default(),
            season: facts.season.unwrap_or(0),
            job_id: row.job_id.clone(),
            source_kind: "post",
            source_page: item.map(|i| i.post_url.clone()),
            received_at,
            source_id: facts.source_id.clone(),
            creator: facts.creator.clone(),
            encoding,
            observation,
        };
        let (row, now) = (row.clone(), self.now());
        self.write(move |c| records::stored(c, &row, &new, kept, now))
            .await
    }

    async fn end_store_step(&self, job: &str) -> Result<(), JobError> {
        let id = job.to_owned();
        let rows = self.read(move |c| records::plan(c, &id)).await?;
        let keep: Vec<&PlanRow> = rows
            .iter()
            .filter(|r| r.action != PlanAction::Drop)
            .collect();
        if keep.is_empty()
            || keep
                .iter()
                .any(|r| r.stored_id.is_none() && r.outcome.is_none())
        {
            return Ok(());
        }
        let failed = keep.iter().filter(|r| r.stored_id.is_none()).count();
        let held = keep.iter().find(|r| r.outcome == Some(Outcome::Held));
        let (state, note) = match (held, failed) {
            (Some(row), _) => (StepState::Waiting, row.note.clone()),
            (None, 0) => (StepState::Done, None),
            (None, n) if n == keep.len() => (StepState::Failed, None),
            (None, n) => (
                StepState::Partial,
                Some(format!("{n}개를 보관하지 못했어요")),
            ),
        };
        self.store
            .set_step(job, StepKind::Store, state, note, self.now())
            .await
    }

    // -----------------------------------------------------------------------
    // The receive area

    /// Removes the receipts whose rows are all stored from the receive area.
    async fn clear(&self, job: &str) -> Result<(), JobError> {
        let id = job.to_owned();
        let clearable = self.read(move |c| records::clearable(c, &id)).await?;
        let mut removed = 0;
        for receipt in clearable {
            if !receipt.cleared {
                let (id, now) = (receipt.id.clone(), self.now());
                self.write(move |c| records::clearing(c, &id, now)).await?;
            }
            let path = self.area.at(&receipt.path);
            let existed = path.exists();
            match blocking(move || files::remove_known(&path)).await {
                Ok(()) if existed => removed += 1,
                Ok(()) => {}
                Err(err) => {
                    self.event(
                        job,
                        "받은 파일을 수신 영역에서 지우지 못했어요".to_owned(),
                        Some(format!("{}: {err}", receipt.path)),
                    )
                    .await?;
                }
            }
        }
        if removed > 0 {
            // The job's folder and the folders under it go once empty.
            let root = self.area.at(&ReceiveArea::job_dir(job));
            blocking(move || remove_empty_dirs(&root)).await;
            self.event(
                job,
                "보관한 파일을 수신 영역에서 지웠어요".to_owned(),
                Some(format!("파일 {removed}개")),
            )
            .await?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Applying

    /// Why the episode of `video` has a subtitle, if it has: one the library
    /// recorded, or a file beside the video under its stem.
    async fn existing_subtitle(
        &self,
        facts: &JobFacts,
        folder: &str,
        episode: i64,
        video: &str,
    ) -> Result<Option<String>, JobError> {
        let work = facts.work_id.clone().unwrap_or_default();
        let season = facts.season.unwrap_or(0);
        let (_, subtitles) = self
            .read(move |c| records::episode_files(c, &work, season, episode))
            .await?;
        if let Some(first) = subtitles.first() {
            return Ok(Some(first.clone()));
        }
        let (dir, stem) = video_parts(video);
        let path = files::within(Path::new(folder), dir);
        let names = blocking(move || files::names_in(&path)).await;
        let prefix = format!("{}.", stem.to_lowercase());
        Ok(match names {
            Ok(names) => names
                .into_iter()
                .find(|n| {
                    let lower = n.to_lowercase();
                    lower.starts_with(&prefix)
                        && extension(&lower)
                            .is_some_and(|e| SUBTITLE_EXTENSIONS.contains(&e.as_str()))
                })
                .map(|n| joined(dir, &n)),
            Err(err) => Some(format!("영상 폴더를 읽지 못했어요: {err}")),
        })
    }

    async fn apply_row(
        &self,
        facts: &JobFacts,
        folder: &str,
        row: &PlanRow,
    ) -> Result<(), JobError> {
        let job = row.job_id.as_str();
        let (Some(placed), Some(stored_id), Some(ext)) = (
            row.placed.as_ref(),
            row.stored_id.clone(),
            row.format.and_then(SubtitleFormat::extension),
        ) else {
            return Ok(());
        };
        let label = row_label(row);
        let work = facts.work_id.clone().unwrap_or_default();
        let season = facts.season.unwrap_or(0);
        let episode = placed.episode;
        let (videos, _) = self
            .read(move |c| records::episode_files(c, &work, season, episode))
            .await?;
        let mut present = Vec::new();
        for video in videos {
            let path = files::within(Path::new(folder), &video);
            if blocking(move || files::occupied(&path))
                .await
                .unwrap_or(false)
            {
                present.push(video);
            }
        }
        let video = match present.as_slice() {
            [] => {
                let note = "영상이 없어 보관만 했어요".to_owned();
                self.settle_row(row, Outcome::NoVideo, note.clone()).await?;
                return self.event(job, format!("{label}: {note}"), None).await;
            }
            [one] => one.clone(),
            _ => {
                let note =
                    "이 회차의 영상이 여럿이라 어느 영상 옆에 둘지 정하지 못해 보관만 했어요"
                        .to_owned();
                self.settle_row(row, Outcome::Stored, note.clone()).await?;
                return self.event(job, format!("{label}: {note}"), None).await;
            }
        };
        if let Some(existing) = self
            .existing_subtitle(facts, folder, episode, &video)
            .await?
        {
            let note = "이 회차에 자막이 이미 있어 그대로 두고 보관만 했어요".to_owned();
            self.settle_row(row, Outcome::Existing, note.clone())
                .await?;
            return self
                .event(job, format!("{label}: {note}"), Some(existing))
                .await;
        }
        let (dir, stem) = video_parts(&video);
        let target = joined(dir, &format!("{stem}.{ext}"));
        // Another job's apply under way here (stopped before it ended) is
        // the episode's subtitle once it ends.
        let (f, lower) = (folder.to_owned(), target.to_lowercase());
        if self
            .read(move |c| records::busy_targets(c, &f, None))
            .await?
            .contains(&lower)
        {
            let note =
                "다른 작업이 이 회차에 자막을 적용하는 중이라 그대로 두고 보관만 했어요".to_owned();
            self.settle_row(row, Outcome::Existing, note.clone())
                .await?;
            return self
                .event(job, format!("{label}: {note}"), Some(target))
                .await;
        }
        let Some(asset) = self
            .read(move |c| records::stored_asset(c, &stored_id))
            .await?
        else {
            return self
                .fail_row(row, "보관본의 기록을 찾지 못했어요".to_owned())
                .await;
        };
        let effect = Effect {
            id: uuid::Uuid::new_v4().to_string(),
            job_id: job.to_owned(),
            position: row.position,
            kind: EffectKind::Apply,
            state: EffectState::Intended,
            folder: folder.to_owned(),
            temp: format!("{}/{}", files::TEMP_DIR, uuid::Uuid::new_v4()),
            target,
            video: Some(video.clone()),
            size: asset.size,
            sha256: asset.sha256.clone(),
            object: None,
            reason: None,
        };
        let e = effect.clone();
        let now = self.now();
        self.write(move |c| records::intend(c, &e, now)).await?;
        let source = files::within(Path::new(folder), &asset.relative_path);
        let temp = files::within(Path::new(folder), &effect.temp);
        let (size, sha) = (effect.size, effect.sha256.clone());
        match blocking(move || files::copy_to_temp(&source, &temp, size, &sha)).await {
            Copied::Ready(object) => {
                let (id, o, now) = (effect.id.clone(), object.clone(), self.now());
                self.write(move |c| records::prepared(c, &id, &o, now))
                    .await?;
                let effect = Effect {
                    state: EffectState::Prepared,
                    object: Some(object),
                    ..effect
                };
                self.finish_apply(facts, effect).await
            }
            Copied::SourceChanged(reason) => {
                self.hold_effect(
                    &effect,
                    &format!("보관본이 기록과 달라 적용하지 않았어요 ({reason})"),
                )
                .await
            }
            Copied::Failed(err) if err.kind() == io::ErrorKind::NotFound => {
                self.hold_effect(&effect, "보관본 파일이 없어 적용하지 않았어요")
                    .await
            }
            Copied::Failed(err) => {
                self.fail_effect(&effect, format!("적용본을 쓰지 못했어요: {err}"))
                    .await
            }
        }
    }

    /// Publishes a prepared apply, unless the episode has a subtitle now or
    /// its name is taken.
    async fn finish_apply(&self, facts: &JobFacts, effect: Effect) -> Result<(), JobError> {
        let Some(row) = self.row_of(&effect).await? else {
            return Ok(());
        };
        let (Some(placed), Some(video), Some(stored_id)) = (
            row.placed.clone(),
            effect.video.clone(),
            row.stored_id.clone(),
        ) else {
            return self
                .hold_effect(&effect, "적용할 회차의 기록이 바뀌었어요")
                .await;
        };
        let folder = PathBuf::from(&effect.folder);
        let temp = files::within(&folder, &effect.temp);
        if let Some(existing) = self
            .existing_subtitle(facts, &effect.folder, placed.episode, &video)
            .await?
        {
            let reason = "적용하려던 회차에 다른 자막이 생겨 덮어쓰지 않고 보류했어요";
            return self.withdraw(&row, &effect, &temp, reason, existing).await;
        }
        let target = files::within(&folder, &effect.target);
        let from = temp.clone();
        match blocking(move || files::publish(&from, &target)).await {
            Published::Done => {
                self.applied_by(facts, &row, &effect, placed.episode, stored_id)
                    .await
            }
            Published::Occupied => {
                let reason = "적용하려던 이름에 다른 파일이 먼저 생겨 덮어쓰지 않고 보류했어요";
                let at = effect.target.clone();
                self.withdraw(&row, &effect, &temp, reason, at).await
            }
            Published::Failed(err) => self.unsure(&effect, &temp, err).await,
        }
    }

    /// An apply that something on disk overtook before it was published:
    /// its temporary file goes, nothing beside the video changes and the row
    /// is held for a person (the effect too when its file cannot go).
    async fn withdraw(
        &self,
        row: &PlanRow,
        effect: &Effect,
        temp: &Path,
        reason: &str,
        found: String,
    ) -> Result<(), JobError> {
        let temp = temp.to_owned();
        if let Err(err) = blocking(move || files::remove_known(&temp)).await {
            return self
                .hold_effect(
                    effect,
                    &format!("{reason} (임시 파일을 지우지 못했어요: {err})"),
                )
                .await;
        }
        let (e, r, now) = (effect.clone(), reason.to_owned(), self.now());
        self.write(move |c| records::withdrawn(c, &e, &r, now))
            .await?;
        self.event(
            &row.job_id,
            format!("{}: {reason}", row_label(row)),
            Some(found),
        )
        .await
    }

    async fn applied_by(
        &self,
        facts: &JobFacts,
        row: &PlanRow,
        effect: &Effect,
        episode: i64,
        stored_id: String,
    ) -> Result<(), JobError> {
        let copy = NewApplied {
            work_id: facts.work_id.clone().unwrap_or_default(),
            stored_id,
            season: facts.season.unwrap_or(0),
            episode,
        };
        let (e, now) = (effect.clone(), self.now());
        let object = effect.object.clone().unwrap_or_default();
        match self
            .write(move |c| records::applied(c, &e, &copy, &object, None, now))
            .await
        {
            Ok(_) => {}
            Err(err) if is_constraint(&err) => return self.unrecorded(effect, err).await,
            Err(err) => return Err(err),
        }
        self.event(
            &row.job_id,
            format!("{}: 영상 옆에 적용했어요", row_label(row)),
            Some(effect.target.clone()),
        )
        .await
    }

    async fn end_apply_step(&self, job: &str) -> Result<(), JobError> {
        let id = job.to_owned();
        let rows = self.read(move |c| records::plan(c, &id)).await?;
        let apply: Vec<&PlanRow> = rows
            .iter()
            .filter(|r| {
                r.action == PlanAction::Apply && r.question.is_none() && r.stored_id.is_some()
            })
            .collect();
        if apply.is_empty() || apply.iter().any(|r| r.outcome.is_none()) {
            return Ok(());
        }
        let applied = apply
            .iter()
            .filter(|r| r.outcome == Some(Outcome::Applied))
            .count();
        let failed = apply
            .iter()
            .filter(|r| r.outcome == Some(Outcome::Failed))
            .count();
        let held = apply.iter().find(|r| r.outcome == Some(Outcome::Held));
        let (state, note) = match held {
            Some(row) => (StepState::Waiting, row.note.clone()),
            None if failed == apply.len() => (StepState::Failed, apply[0].note.clone()),
            None if failed > 0 => (
                StepState::Partial,
                Some(format!("{failed}개를 적용하지 못했어요")),
            ),
            // Done: what was not applied says why.
            None => (
                StepState::Done,
                (applied < apply.len())
                    .then(|| apply.iter().find_map(|r| r.note.clone()))
                    .flatten(),
            ),
        };
        self.store
            .set_step(job, StepKind::Apply, state, note, self.now())
            .await
    }

    // -----------------------------------------------------------------------
    // Restart

    /// Compares an effect an earlier start left unfinished with the disk
    /// (see the module docs).
    async fn recover(
        &self,
        facts: &JobFacts,
        items: &[ItemRow],
        effect: Effect,
    ) -> Result<(), JobError> {
        let folder = PathBuf::from(&effect.folder);
        let temp = files::within(&folder, &effect.temp);
        let target = files::within(&folder, &effect.target);
        match effect.state {
            EffectState::Intended => match blocking(move || files::remove_known(&temp)).await {
                Ok(()) => {
                    let (id, now) = (effect.id.clone(), self.now());
                    self.write(move |c| {
                        records::end_effect(c, &id, EffectState::Abandoned, None, now)
                    })
                    .await
                }
                Err(err) => {
                    self.hold_effect(&effect, &format!("임시 파일을 지우지 못했어요: {err}"))
                        .await
                }
            },
            EffectState::Prepared => {
                let (t, g) = blocking(move || (files::facts(&temp), files::facts(&target))).await;
                let ours = |found: &Option<(u64, String, String)>| {
                    found.as_ref().is_some_and(|(n, s, o)| {
                        *n == effect.size
                            && *s == effect.sha256
                            && Some(o) == effect.object.as_ref()
                    })
                };
                match (t, g) {
                    (Ok(t), _) if ours(&t) => match effect.kind {
                        EffectKind::Store => self.finish_store(facts, items, effect).await,
                        EffectKind::Apply => self.finish_apply(facts, effect).await,
                    },
                    (Ok(None), Ok(g)) if ours(&g) => {
                        let target = files::within(&folder, &effect.target);
                        let synced = blocking(move || match target.parent() {
                            Some(dir) => crate::area::sync_dir(dir),
                            None => Ok(()),
                        })
                        .await;
                        if let Err(err) = synced {
                            return self
                                .hold_effect(
                                    &effect,
                                    &format!("공개한 파일을 동기화하지 못했어요: {err}"),
                                )
                                .await;
                        }
                        let Some(row) = self.row_of(&effect).await? else {
                            return Ok(());
                        };
                        match (effect.kind, row.placed.clone(), row.stored_id.clone()) {
                            (EffectKind::Store, _, _) => {
                                self.stored_by(facts, items, &row, &effect).await
                            }
                            (EffectKind::Apply, Some(placed), Some(stored_id)) => {
                                self.applied_by(facts, &row, &effect, placed.episode, stored_id)
                                    .await
                            }
                            _ => {
                                self.hold_effect(&effect, "적용한 회차의 기록이 바뀌었어요")
                                    .await
                            }
                        }
                    }
                    (Err(err), _) | (_, Err(err)) => {
                        self.hold_effect(
                            &effect,
                            &format!("이전 작업의 파일을 확인하지 못했어요: {err}"),
                        )
                        .await
                    }
                    _ => {
                        self.hold_effect(
                            &effect,
                            "이전 작업이 쓰던 파일이 기록과 달라 결과를 확인할 수 없어요",
                        )
                        .await
                    }
                }
            }
            _ => Ok(()),
        }
    }

    // -----------------------------------------------------------------------
    // Rows

    async fn row_of(&self, effect: &Effect) -> Result<Option<PlanRow>, JobError> {
        let (job, position) = (effect.job_id.clone(), effect.position);
        self.read(move |c| records::plan_row_at(c, &job, position))
            .await
    }

    async fn settle_row(
        &self,
        row: &PlanRow,
        outcome: Outcome,
        note: String,
    ) -> Result<(), JobError> {
        let (job, position, now) = (row.job_id.clone(), row.position, self.now());
        self.write(move |c| records::set_outcome(c, &job, position, outcome, Some(note), now))
            .await
    }

    async fn fail_row(&self, row: &PlanRow, reason: String) -> Result<(), JobError> {
        self.settle_row(row, Outcome::Failed, reason.clone())
            .await?;
        self.event(
            &row.job_id,
            format!("{}: 처리하지 못했어요", row_label(row)),
            Some(reason),
        )
        .await
    }

    async fn fail_effect(&self, effect: &Effect, reason: String) -> Result<(), JobError> {
        let (id, r, now) = (effect.id.clone(), reason.clone(), self.now());
        self.write(move |c| records::end_effect(c, &id, EffectState::Failed, Some(r), now))
            .await?;
        if let Some(row) = self.row_of(effect).await? {
            self.fail_row(&row, reason).await?;
        }
        Ok(())
    }

    /// A published file whose records the database refuses: no later run
    /// changes that, so the effect is held with its file left as it is.
    async fn unrecorded(&self, effect: &Effect, err: JobError) -> Result<(), JobError> {
        let reason = format!("공개한 파일을 기록하지 못했어요: {err}");
        self.hold_effect(effect, &reason).await
    }

    async fn hold_effect(&self, effect: &Effect, reason: &str) -> Result<(), JobError> {
        let (e, r, now) = (effect.clone(), reason.to_owned(), self.now());
        self.write(move |c| records::hold(c, &e, &r, now)).await?;
        let label = match self.row_of(effect).await? {
            Some(row) => row_label(&row),
            None => effect.target.clone(),
        };
        self.event(
            &effect.job_id,
            format!("{label}: 보류했어요"),
            Some(reason.to_owned()),
        )
        .await
    }
}

/// Removes `root` and the folders under it that are empty, deepest first.
fn remove_empty_dirs(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            remove_empty_dirs(&entry.path());
        }
    }
    let _ = std::fs::remove_dir(root);
}

/// A write the database refused for a constraint (a unique index, a check).
fn is_constraint(err: &JobError) -> bool {
    let sqlite = match err {
        JobError::Sqlite(e) | JobError::Db(trss_core::db::DbError::Sqlite(e)) => e,
        _ => return false,
    };
    matches!(
        sqlite,
        rusqlite::Error::SqliteFailure(e, _) if e.code == rusqlite::ErrorCode::ConstraintViolation
    )
}

fn follow_error(err: crate::FollowError) -> JobError {
    JobError::Other(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creator_folders_are_safe_on_every_share() {
        assert_eq!(creator_folder(Some("Doomdos")), "Doomdos");
        assert_eq!(creator_folder(Some("a/b:c?")), "a_b_c_");
        assert_eq!(creator_folder(Some(".hidden")), "hidden");
        assert_eq!(creator_folder(Some("name. ")), "name");
        assert_eq!(creator_folder(Some("con")), "con_");
        assert_eq!(creator_folder(Some("LPT1.x")), "LPT1.x_");
        assert_eq!(creator_folder(Some("  ")), UNKNOWN_CREATOR);
        assert_eq!(creator_folder(None), UNKNOWN_CREATOR);
        assert_eq!(creator_folder(Some("코코렛")), "코코렛");
    }

    #[test]
    fn a_taken_name_is_numbered_before_its_extension_whatever_its_case() {
        let none = |_: &str| false;
        assert_eq!(
            store_name("Show - 01.ass", &[], none),
            Some(("Show - 01.ass".to_owned(), false))
        );
        assert_eq!(
            store_name("Show - 01.ass", &["show - 01.ass".to_owned()], none),
            Some(("Show - 01 (2).ass".to_owned(), false))
        );
        assert_eq!(
            store_name(
                "a.ko.ass",
                &["a.ko.ass".to_owned(), "a.ko (2).ass".to_owned()],
                none
            ),
            Some(("a.ko (3).ass".to_owned(), false))
        );
        // The same bytes under the name are that file.
        assert_eq!(
            store_name("A.ass", &["a.ass".to_owned()], |n| n == "a.ass"),
            Some(("A.ass".to_owned(), true))
        );
        // Under a numbered name too, past a free one.
        assert_eq!(
            store_name(
                "A.ass",
                &["a.ass".to_owned(), "a (3).ass".to_owned()],
                |n| n == "a (3).ass"
            ),
            Some(("A (3).ass".to_owned(), true))
        );
    }

    #[test]
    fn members_are_told_by_format_and_name() {
        assert_eq!(
            member("a.ass", Format::Ass),
            Member::Subtitle(SubtitleFormat::Ass)
        );
        assert_eq!(
            member("a.ssa", Format::Ass),
            Member::Subtitle(SubtitleFormat::Other)
        );
        assert_eq!(
            member("a.vtt", Format::Other),
            Member::Subtitle(SubtitleFormat::Other)
        );
        assert_eq!(member("H2MPRB.TTF", Format::Other), Member::Font);
        assert_eq!(member("pack.rar", Format::Other), Member::Archive);
        assert_eq!(member("a.zip", Format::Zip), Member::Archive);
        assert_eq!(member("setup.exe", Format::Other), Member::Other);
    }

    #[test]
    fn encodings_show_by_bom_or_utf8() {
        assert_eq!(
            encoding_of(b"\xEF\xBB\xBF[Script Info]").as_deref(),
            Some("utf-8")
        );
        assert_eq!(encoding_of(b"\xFF\xFEa\0").as_deref(), Some("utf-16le"));
        assert_eq!(encoding_of("자막".as_bytes()).as_deref(), Some("utf-8"));
        assert_eq!(encoding_of(b"\xC0\xDA\xB8\xB7"), None);
    }
}
