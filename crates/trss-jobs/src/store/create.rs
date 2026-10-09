//! Making a job: the candidates a person picked or the app took for a
//! subscribed creator, and the files a person uploaded.

use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use trss_core::Millis;
use trss_subtitles::{
    upload::{Archive, Kind},
    verify::Format,
};

use super::{rows::upload_note, JobError, JobStore, AUTO, UPLOAD};
use crate::model::JobState;

/// A job to make: the candidates a person picked, or the one the app takes
/// for the subscribed creator ([`crate::follow`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewJob {
    /// The ID the browser made for the action (`auto:<observation id>` for a
    /// job the app makes).
    pub command_id: String,
    /// The request's content in canonical JSON, to tell a repeat from another
    /// request with the same ID.
    pub request: String,
    /// How it was asked for: `pick` (a person picked the candidates) or
    /// [`AUTO`].
    pub origin: String,
    pub work_id: Option<String>,
    pub season: Option<i64>,
    pub anime_no: Option<i64>,
    pub source_id: Option<String>,
    pub creator: Option<String>,
    /// The observation whose subtitle the job receives a revision of: the
    /// creator's subtitle of the episode received before.
    pub revision_of: Option<i64>,
    /// The job receives a line of the creator for an episode whose subtitle
    /// file the user gave this creator (`revision_of` is `None`: nothing of it
    /// was received before).
    pub revises_attributed: bool,
    /// In the order to receive them.
    pub items: Vec<NewItem>,
}

/// One candidate of a job, as it was picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewItem {
    pub observation_id: Option<i64>,
    pub episode: String,
    pub post_url: String,
    pub found_at: Millis,
}

/// The post address of the one item of an upload job: there is no post.
const UPLOAD_POST: &str = "upload:";

/// An upload to record: the job is made `pending` for the worker's analysis
/// and a person's 배치 확인, with its files and the names of those it
/// dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewUpload {
    /// The job's ID, which names its folder in the receive area.
    pub id: String,
    pub command_id: String,
    /// The request's content in canonical JSON.
    pub request: String,
    pub work_id: String,
    pub season: i64,
    pub anime_no: Option<i64>,
    pub source_id: Option<String>,
    pub creator: Option<String>,
    pub files: Vec<UploadedFile>,
    pub dropped: Vec<crate::upload::Dropped>,
}

/// A file an upload kept, already in the job's folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadedFile {
    pub id: String,
    /// Where the file was in what the person gave: unique within the job.
    pub file_key: String,
    pub name: String,
    /// Relative to the receive area.
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub object: String,
    pub format: Format,
    pub kind: Kind,
    /// Which archive format an `archive` is.
    pub archive: Option<Archive>,
}

/// What [`JobStore::create`] did, with the job's ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Created {
    Created(String),
    /// The browser's ID had made this job with the same request before.
    Existing(String),
    /// The browser's ID made another request before; nothing was stored.
    Mismatch(String),
}

/// The mapping version a job was decided under ([`JobStore::create_under_mapping`]).
#[derive(Debug, Clone)]
pub struct MappingStamp {
    pub work_id: String,
    pub season: u32,
    pub source_id: String,
    /// The version read (`0` for a source with no mapping).
    pub version: i64,
}

impl JobStore {
    /// Stores `job` as `pending` with its `found` step done at `now`, unless
    /// its browser ID is known. The check and the insert are one write
    /// transaction, so two deliveries at once store one job.
    pub async fn create(&self, job: NewJob, now: Millis) -> Result<Created, JobError> {
        let made = self.db.run(move |c| create(c, &job, now, None)).await?;
        Ok(made.unwrap_or_else(|| unreachable!("a job with no stamp is never refused")))
    }

    /// [`JobStore::create`] for a job made under a source's episode mapping:
    /// the mapping's version is checked in the same write transaction as the
    /// insert, and `None` (nothing stored) says the mapping changed since
    /// `under` was read, so the job was decided under a mapping that no longer
    /// stands.
    pub async fn create_under_mapping(
        &self,
        job: NewJob,
        now: Millis,
        under: MappingStamp,
    ) -> Result<Option<Created>, JobError> {
        self.db
            .run(move |c| create(c, &job, now, Some(&under)))
            .await
    }

    /// Records an upload as a job whose 받기 is over and that waits for the
    /// worker (`pending`), unless its command ID is known. The check and the writes are one transaction, so two
    /// deliveries at once make one job.
    pub async fn create_upload(&self, upload: NewUpload, now: Millis) -> Result<Created, JobError> {
        self.db.run(move |c| create_upload(c, &upload, now)).await
    }

    /// Which of `ids` name a job or a file record (a receipt's attempt folder
    /// is named by its file's ID).
    pub async fn known_receive_ids(&self, ids: Vec<String>) -> Result<HashSet<String>, JobError> {
        self.db
            .run(move |c| {
                let mut stmt = c.prepare_cached(
                    "SELECT EXISTS(SELECT 1 FROM subtitle_jobs WHERE id = ?1)
                         OR EXISTS(SELECT 1 FROM subtitle_job_files WHERE id = ?1)",
                )?;
                let mut known = HashSet::new();
                for id in ids {
                    if stmt.query_row([&id], |r| r.get::<_, bool>(0))? {
                        known.insert(id);
                    }
                }
                Ok(known)
            })
            .await
    }

    /// Which of the items `ids` still have to receive their files: pending,
    /// running or waiting.
    pub async fn open_items(&self, ids: Vec<i64>) -> Result<HashSet<i64>, JobError> {
        self.db
            .run(move |c| {
                let mut stmt = c.prepare_cached(
                    "SELECT EXISTS(SELECT 1 FROM subtitle_job_items
                                   WHERE id = ?1 AND state IN ('pending', 'running', 'waiting'))",
                )?;
                let mut open = HashSet::new();
                for id in ids {
                    if stmt.query_row([id], |r| r.get::<_, bool>(0))? {
                        open.insert(id);
                    }
                }
                Ok(open)
            })
            .await
    }
}

fn create(
    c: &mut Connection,
    job: &NewJob,
    now: Millis,
    under: Option<&MappingStamp>,
) -> Result<Option<Created>, JobError> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(under) = under {
        let version: i64 = tx
            .prepare_cached(
                "SELECT version FROM subtitle_episode_mappings
                  WHERE work_id = ?1 AND season = ?2 AND source_id = ?3",
            )?
            .query_row(params![under.work_id, under.season, under.source_id], |r| {
                r.get(0)
            })
            .optional()?
            .unwrap_or(0);
        if version != under.version {
            return Ok(None);
        }
    }
    let known: Option<(String, String)> = tx
        .prepare_cached("SELECT id, request FROM subtitle_jobs WHERE command_id = ?1")?
        .query_row([&job.command_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    if let Some((id, request)) = known {
        return Ok(Some(match request == job.request {
            true => Created::Existing(id),
            false => Created::Mismatch(id),
        }));
    }
    let id = uuid::Uuid::new_v4().to_string();
    tx.prepare_cached(
        "INSERT INTO subtitle_jobs
             (id, command_id, request, origin, work_id, season, anime_no, source_id, creator,
              revision_of, revises_attributed, state, created_at, updated_at, state_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'pending', ?12, ?12, ?12)",
    )?
    .execute(params![
        id,
        job.command_id,
        job.request,
        job.origin,
        job.work_id,
        job.season,
        job.anime_no,
        job.source_id,
        job.creator,
        job.revision_of,
        job.revises_attributed,
        now
    ])?;
    for (position, item) in job.items.iter().enumerate() {
        tx.prepare_cached(
            "INSERT INTO subtitle_job_items
                 (job_id, position, observation_id, episode, post_url, found_at, state,
                  updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7)",
        )?
        .execute(params![
            id,
            position as i64,
            item.observation_id,
            item.episode,
            item.post_url,
            item.found_at,
            now
        ])?;
    }
    let count = job.items.len();
    tx.prepare_cached(
        "INSERT INTO subtitle_job_steps (job_id, step, state, at, note)
         VALUES (?1, 'found', 'done', ?2, ?3)",
    )?
    .execute(params![id, now, format!("후보 {count}개")])?;
    let message = match (
        job.origin == AUTO,
        job.revision_of.is_some(),
        job.revises_attributed,
    ) {
        (true, _, true) => {
            "구독 제작자의 수정본이 제작자를 붙인 자막에 맞아 자동으로 작업을 만들었어요"
        }
        (true, true, false) => "구독 제작자의 수정본이라 자동으로 작업을 만들었어요",
        (true, false, false) => "구독 제작자의 새 회차라 자동으로 작업을 만들었어요",
        _ => "작업을 만들었어요",
    };
    tx.prepare_cached(
        "INSERT INTO subtitle_job_events (job_id, at, message, detail)
         VALUES (?1, ?2, ?3, ?4)",
    )?
    .execute(params![id, now, message, format!("후보 {count}개")])?;
    tx.commit()?;
    Ok(Some(Created::Created(id)))
}

fn create_upload(c: &mut Connection, up: &NewUpload, now: Millis) -> Result<Created, JobError> {
    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let known: Option<(String, String)> = tx
        .prepare_cached("SELECT id, request FROM subtitle_jobs WHERE command_id = ?1")?
        .query_row([&up.command_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    if let Some((id, request)) = known {
        return Ok(match request == up.request {
            true => Created::Existing(id),
            false => Created::Mismatch(id),
        });
    }
    let counts = crate::upload::Counts {
        subtitles: up.files.iter().filter(|f| f.kind == Kind::Subtitle).count(),
        fonts: up.files.iter().filter(|f| f.kind == Kind::Font).count(),
        archives: up.files.iter().filter(|f| f.kind == Kind::Archive).count(),
    };
    let kept = counts.sentence();
    let dropped_note = match up.dropped.len() {
        0 => String::new(),
        n => format!(" · 뺀 파일 {n}개"),
    };
    // The worker analyses what was uploaded (unpacking an archive,
    // `crate::place::unpack`), and the job waits for the person's
    // 배치 확인 before anything is kept.
    tx.prepare_cached(
        "INSERT INTO subtitle_jobs
             (id, command_id, request, origin, work_id, season, anime_no, source_id, creator,
              state, note, created_at, updated_at, state_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12, ?12)",
    )?
    .execute(params![
        up.id,
        up.command_id,
        up.request,
        UPLOAD,
        up.work_id,
        up.season,
        up.anime_no,
        up.source_id,
        up.creator,
        JobState::Pending,
        upload_note(&counts),
        now
    ])?;
    tx.prepare_cached(
        "INSERT INTO subtitle_job_items
             (job_id, position, observation_id, episode, post_url, found_at, state, updated_at)
         VALUES (?1, 0, NULL, '', ?2, ?3, 'done', ?3)",
    )?
    .execute(params![up.id, UPLOAD_POST, now])?;
    let item_id = tx.last_insert_rowid();
    for file in &up.files {
        tx.prepare_cached(
            "INSERT INTO subtitle_job_files
                 (id, job_id, item_id, file_key, name, state, size, sha256, object, path,
                  created_at, updated_at, format, kind, archive_type)
             VALUES (?1, ?2, ?3, ?4, ?5, 'done', ?6, ?7, ?8, ?9, ?10, ?10, ?11, ?12, ?13)",
        )?
        .execute(params![
            file.id,
            up.id,
            item_id,
            file.file_key,
            file.name,
            i64::try_from(file.size).unwrap_or(i64::MAX),
            file.sha256,
            file.object,
            file.path,
            now,
            file.format.code(),
            file.kind.code(),
            file.archive.map(Archive::code)
        ])?;
    }
    for (position, file) in up.dropped.iter().enumerate() {
        tx.prepare_cached(
            "INSERT INTO subtitle_job_dropped (job_id, position, name, reason)
             VALUES (?1, ?2, ?3, ?4)",
        )?
        .execute(params![up.id, position as i64, file.name, file.reason])?;
    }
    tx.prepare_cached(
        "INSERT INTO subtitle_job_steps (job_id, step, state, at, note)
         VALUES (?1, 'receive', 'done', ?2, ?3)",
    )?
    .execute(params![up.id, now, format!("{kept}{dropped_note}")])?;
    tx.prepare_cached(
        "INSERT INTO subtitle_job_events (job_id, at, message, detail)
         VALUES (?1, ?2, '자막과 폰트를 올렸어요', ?3)",
    )?
    .execute(params![up.id, now, format!("{kept}{dropped_note}")])?;
    tx.commit()?;
    Ok(Created::Created(up.id.clone()))
}
