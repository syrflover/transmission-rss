//! The row types of the job records and the mappers of more than one feature.

use rusqlite::{Connection, Row};
use trss_core::Millis;
use trss_subtitles::{
    upload::{Archive, Kind},
    verify::Format,
    FailureKind,
};

use super::JobError;
use super::{FIND, UPLOAD};
use crate::model::{FileState, ItemState, JobState, StepKind, StepState, Wait};

/// A job as the lists show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRow {
    pub seq: i64,
    pub id: String,
    /// How it was asked for (`pick`, [`super::AUTO`]).
    pub origin: String,
    /// For a revision: the observation whose subtitle was received before, and
    /// the latest job that received it.
    pub revision_of: Option<i64>,
    pub revises_job: Option<String>,
    /// A revision of a subtitle file whose creator the user named.
    pub revises_attributed: bool,
    pub state: JobState,
    pub wait: Option<Wait>,
    pub stage: Option<StepKind>,
    pub note: Option<String>,
    pub state_at: Millis,
    pub created_at: Millis,
    pub finished_at: Option<Millis>,
    pub work_id: Option<String>,
    /// The work's folder name, while the library has the work.
    pub work_name: Option<String>,
    pub season: Option<i64>,
    pub anime_no: Option<i64>,
    /// The Anissia title of the anime, while its snapshot is kept.
    pub anime_title: Option<String>,
    pub creator: Option<String>,
    /// The items' episodes, in order.
    pub episodes: Vec<String>,
    /// The first item's post host.
    pub source: Option<String>,
    pub progress: Progress,
    /// The class of the first failed item's failure, when it has one.
    pub failure: Option<FailureKind>,
    /// For an upload or a find job: what it kept and dropped.
    pub upload: Option<UploadSummary>,
    /// For a find job: a person asked it to finish and its 받기 has not
    /// ended yet.
    pub finishing: bool,
    /// For a find job: its 받기 has not ended ([`super::JobRun::end_find`] makes
    /// its item done), so its remote screen and `받기 끝내기` still apply.
    pub receiving: bool,
}

impl JobRow {
    /// What to call the job: its anime's title, else its work's name.
    pub fn title(&self) -> String {
        self.anime_title
            .clone()
            .or_else(|| self.work_name.clone())
            .unwrap_or_else(|| "작품을 찾지 못한 작업".to_owned())
    }

    /// Whether the job waits for a person to pass a site's check (`인증
    /// 필요`). A find job, which waits the same way while a person browses, is
    /// the person's own doing and does not. [`WAITS_FOR_CHECK`] is the same
    /// condition in SQL.
    pub fn waits_for_check(&self) -> bool {
        self.state == JobState::Waiting && self.wait == Some(Wait::Auth) && self.origin != FIND
    }

    /// Whether the job waits for a person to say which episode a file is (its
    /// 배치 확인, `회차 확인 필요`). [`WAITS_FOR_PLACEMENT`] is the same condition
    /// in SQL.
    pub fn waits_for_placement(&self) -> bool {
        self.state == JobState::Waiting && self.wait == Some(Wait::Placement)
    }

    /// Where the job sits among the open jobs that are neither failed nor
    /// running: a person's check first, then the other waits (a find job,
    /// which waits for a person's browsing, among them), held, then pending.
    /// `None` for any other.
    fn waiting_rank(&self) -> Option<u8> {
        match self.state {
            JobState::Waiting if self.waits_for_check() => Some(0),
            JobState::Waiting => Some(1),
            JobState::Held => Some(2),
            JobState::Pending => Some(3),
            _ => None,
        }
    }
}

/// [`JobRow::waits_for_check`] over the table `subtitle_jobs j`.
pub(super) const WAITS_FOR_CHECK: &str =
    "j.state = 'waiting' AND j.wait = 'auth' AND j.origin <> 'find'";

/// [`JobRow::waits_for_placement`] over the table `subtitle_jobs j`.
pub(super) const WAITS_FOR_PLACEMENT: &str = "j.state = 'waiting' AND j.wait = 'placement'";

/// The jobs that are not done, in the groups the job list shows them in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OpenGroups {
    /// `failed` and `partial`, the newest state change first.
    pub failed: Vec<JobRow>,
    /// A person's check first, then the other waits, held, then pending
    /// (each oldest first).
    pub waiting: Vec<JobRow>,
    /// Oldest first.
    pub running: Vec<JobRow>,
}

impl OpenGroups {
    /// Groups `open`, the jobs that are not done, oldest first.
    pub fn of(open: Vec<JobRow>) -> OpenGroups {
        let mut groups = OpenGroups::default();
        let mut waiting = Vec::new();
        for row in open {
            match row.state {
                JobState::Failed | JobState::Partial => groups.failed.push(row),
                JobState::Running => groups.running.push(row),
                _ => waiting.extend(row.waiting_rank().map(|rank| (rank, row))),
            }
        }
        groups
            .failed
            .sort_by_key(|r| std::cmp::Reverse((r.state_at, r.seq)));
        waiting.sort_by_key(|(rank, r)| (*rank, r.seq));
        groups.waiting = waiting.into_iter().map(|(_, row)| row).collect();
        groups
    }
}

/// What an upload or a find job kept, by kind, and how many files it dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UploadSummary {
    pub subtitles: usize,
    pub fonts: usize,
    pub archives: usize,
    pub dropped: usize,
}

impl UploadSummary {
    /// What it kept, by kind.
    pub(super) fn counts(&self) -> crate::upload::Counts {
        crate::upload::Counts {
            subtitles: self.subtitles,
            fonts: self.fonts,
            archives: self.archives,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Progress {
    pub done: usize,
    pub failed: usize,
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRow {
    pub step: StepKind,
    pub state: StepState,
    pub at: Millis,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRow {
    pub id: i64,
    pub position: i64,
    pub observation_id: Option<i64>,
    pub episode: String,
    pub post_url: String,
    pub state: ItemState,
    pub wait: Option<Wait>,
    pub reason: Option<String>,
    /// The class of a failed item's failure (`docs/specs/jobs.md`, 공통 수신
    /// 결과와 실패 분류), when it has one.
    pub failure: Option<FailureKind>,
    /// For an item of a revision job whose files are the same bytes as the
    /// earlier receipt's: that receipt's job. There is nothing to replace.
    pub unchanged_from: Option<String>,
    pub files: Vec<FileRow>,
}

/// A candidate a job took: an item that names its observation, with how the
/// item and its job stand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    pub observation_id: i64,
    pub source_id: Option<String>,
    pub episode: String,
    pub post_url: String,
    pub item_state: ItemState,
    pub item_wait: Option<Wait>,
    pub job_id: String,
    pub job_state: JobState,
    pub updated_at: Millis,
}

/// One receipt of a file (see the schema's comment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub id: String,
    pub item_id: i64,
    pub file_key: String,
    pub name: String,
    pub state: FileState,
    pub same_as: Option<String>,
    pub temp_dir: Option<String>,
    pub expected_size: Option<u64>,
    pub size: Option<u64>,
    pub sha256: Option<String>,
    pub object: Option<String>,
    pub path: Option<String>,
    pub reason: Option<String>,
    pub created_at: Millis,
    /// What the received bytes are, found when they were checked.
    pub format: Option<Format>,
    /// The class of a failed (or retried) attempt.
    pub failure: Option<FailureKind>,
    /// The answer's status and media type, and for a failure the size of the
    /// answer that showed it.
    pub http_status: Option<u16>,
    pub content_type: Option<String>,
    pub response_size: Option<u64>,
    /// What the source read about the file, as a JSON array of `[name,
    /// value]` pairs ([`super::snapshot_json`]).
    pub snapshot: Option<String>,
    /// For a file a person uploaded: what its content check judged it to be.
    pub kind: Option<Kind>,
    /// For an uploaded archive: which format its first bytes said.
    pub archive: Option<Archive>,
    /// The folders the file is published under within the job's folder
    /// (`회차/2화`), when the post shows it in some.
    pub folder: Option<String>,
    /// When its bytes were removed from the receive area, once stored
    /// ([`crate::place`]).
    pub cleared_at: Option<Millis>,
    /// For a later volume of a split archive: the first volume, which it is
    /// unpacked, kept and cleared with ([`crate::place::unpack`]).
    pub volume_of: Option<String>,
    /// For an archive: when its members were recorded.
    pub unpacked_at: Option<Millis>,
    /// For an archive: why it could not be unpacked (풀지 못함).
    pub unpack_error: Option<String>,
    /// For an archive: how many tries to unpack it failed for this machine
    /// (a full disk, a limit, the child), and why the last one did; it is
    /// tried again until [`crate::place::unpack::UNPACK_TRIES`]. A try after
    /// them that the archive's own reason ended counts too.
    pub unpack_tries: u32,
    pub unpack_failure: Option<String>,
    /// When the next try may go: an hour after the failure, or `None` once a
    /// worker started after it.
    pub unpack_retry_at: Option<Millis>,
    /// For a Google Drive font not received because it did not change: the
    /// stored font it uses instead ([`crate::place::unchanged`]). Such a
    /// receipt is `done` with no path.
    pub unchanged_asset: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    pub at: Millis,
    pub message: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobDetail {
    pub row: JobRow,
    pub steps: Vec<StepRow>,
    pub items: Vec<ItemRow>,
    /// For an upload job: the files it did not keep, in the order they were
    /// listed.
    pub dropped: Vec<DroppedRow>,
    /// Newest first.
    pub events: Vec<EventRow>,
}

/// A file an upload did not keep, with why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DroppedRow {
    pub name: String,
    pub reason: String,
}

/// A page of done jobs, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DonePage {
    pub items: Vec<JobRow>,
    /// The cursor of the next page; `None` at the end.
    pub next: Option<String>,
    pub total: usize,
}

/// The files a job dropped, in order.
pub(super) fn dropped_rows(c: &Connection, id: &str) -> Result<Vec<DroppedRow>, JobError> {
    let mut stmt = c.prepare_cached(
        "SELECT name, reason FROM subtitle_job_dropped WHERE job_id = ?1 ORDER BY position",
    )?;
    let rows = stmt
        .query_map([id], |r| {
            Ok(DroppedRow {
                name: r.get(0)?,
                reason: r.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

const JOB_COLUMNS: &str = "
    SELECT j.seq, j.id, j.state, j.wait, j.stage, j.note, j.state_at, j.created_at,
           j.finished_at, j.work_id, w.dir_name, j.season, j.anime_no, a.subject, j.creator,
           j.origin, j.revision_of,
           (SELECT r.job_id FROM subtitle_job_items r
              JOIN subtitle_jobs rj ON rj.id = r.job_id
             WHERE r.observation_id = j.revision_of AND r.state = 'done' AND rj.seq < j.seq
             ORDER BY r.id DESC LIMIT 1),
           j.revises_attributed,
           j.finish_at IS NOT NULL AND j.state NOT IN ('done', 'failed', 'partial')
             AND EXISTS (SELECT 1 FROM subtitle_job_items i
                         WHERE i.job_id = j.id AND i.state <> 'done'),
           j.origin = 'find' AND j.state NOT IN ('done', 'failed', 'partial')
             AND EXISTS (SELECT 1 FROM subtitle_job_items i
                         WHERE i.job_id = j.id AND i.state <> 'done')
    FROM subtitle_jobs j
    LEFT JOIN works w ON w.id = j.work_id
    LEFT JOIN anissia_anime a ON a.anime_no = j.anime_no";

fn job_row(r: &Row<'_>) -> rusqlite::Result<JobRow> {
    Ok(JobRow {
        seq: r.get(0)?,
        id: r.get(1)?,
        origin: r.get(15)?,
        revision_of: r.get(16)?,
        revises_job: r.get(17)?,
        revises_attributed: r.get(18)?,
        state: r.get(2)?,
        wait: r.get(3)?,
        stage: r.get(4)?,
        note: r.get(5)?,
        state_at: r.get(6)?,
        created_at: r.get(7)?,
        finished_at: r.get(8)?,
        work_id: r.get(9)?,
        work_name: r.get(10)?,
        season: r.get(11)?,
        anime_no: r.get(12)?,
        anime_title: r.get(13)?,
        creator: r.get(14)?,
        episodes: Vec::new(),
        source: None,
        progress: Progress::default(),
        failure: None,
        upload: None,
        finishing: r.get(19)?,
        receiving: r.get(20)?,
    })
}

/// The jobs `tail` picks, with their items' episodes, source and progress.
pub(super) fn rows<P: rusqlite::Params>(
    c: &Connection,
    tail: &str,
    p: P,
) -> Result<Vec<JobRow>, JobError> {
    let mut stmt = c.prepare_cached(&format!("{JOB_COLUMNS} {tail}"))?;
    let mut jobs = stmt.query_map(p, job_row)?.collect::<Result<Vec<_>, _>>()?;
    let mut items = c.prepare_cached(
        "SELECT episode, post_url, state, failure FROM subtitle_job_items
         WHERE job_id = ?1 ORDER BY position",
    )?;
    for job in &mut jobs {
        let mut first_failed_seen = false;
        let rows = items.query_map([&job.id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, ItemState>(2)?,
                failure_at(r, 3)?,
            ))
        })?;
        for row in rows {
            let (episode, post, state, failure) = row?;
            // The first failed item's class, as the note is its reason: a
            // later item's class would name another failure.
            if state == ItemState::Failed && !first_failed_seen {
                first_failed_seen = true;
                job.failure = failure;
            }
            if job.source.is_none() {
                job.source = url::Url::parse(&post)
                    .ok()
                    .and_then(|u| u.host_str().map(str::to_owned));
            }
            // An upload's or a find job's item stands for the package: it has
            // no episode.
            if !((job.origin == UPLOAD || job.origin == FIND) && episode.is_empty()) {
                job.episodes.push(episode);
            }
            job.progress.total += 1;
            match state {
                ItemState::Done => job.progress.done += 1,
                ItemState::Failed => job.progress.failed += 1,
                _ => {}
            }
        }
    }
    for job in &mut jobs {
        if job.origin == UPLOAD || job.origin == FIND {
            job.upload = Some(upload_summary(c, &job.id)?);
        }
    }
    Ok(jobs)
}

/// An upload job's note: what it kept (`올린 파일: 자막 3개`).
pub(super) fn upload_note(counts: &crate::upload::Counts) -> String {
    format!("올린 파일: {}", counts.sentence())
}

/// The note of a find job that ended with files: what it kept and dropped.
pub(super) fn found_note(summary: &UploadSummary) -> String {
    let dropped = match summary.dropped {
        0 => String::new(),
        n => format!(" · 뺀 파일 {n}개"),
    };
    format!("받은 파일: {}{dropped}", summary.counts().sentence())
}

/// What the upload job `id` kept and dropped.
pub(super) fn upload_summary(c: &Connection, id: &str) -> Result<UploadSummary, JobError> {
    let mut summary = UploadSummary::default();
    let mut stmt = c.prepare_cached(
        "SELECT kind, count(*) FROM subtitle_job_files
         WHERE job_id = ?1 AND state = 'done' AND kind IS NOT NULL GROUP BY kind",
    )?;
    let kinds = stmt
        .query_map([id], |r| Ok((kind_at(r, 0)?, r.get::<_, i64>(1)? as usize)))?
        .collect::<Result<Vec<_>, _>>()?;
    for (kind, count) in kinds {
        match kind {
            Some(Kind::Subtitle) => summary.subtitles = count,
            Some(Kind::Font) => summary.fonts = count,
            Some(Kind::Archive) => summary.archives = count,
            None => {}
        }
    }
    summary.dropped = c
        .prepare_cached("SELECT count(*) FROM subtitle_job_dropped WHERE job_id = ?1")?
        .query_row([id], |r| r.get::<_, i64>(0))? as usize;
    Ok(summary)
}

pub(super) const FILE_COLUMNS: &str = "
    SELECT id, item_id, file_key, name, state, same_as, temp_dir, expected_size, size, sha256,
           object, path, reason, created_at, format, failure, http_status, content_type,
           response_size, snapshot, kind, archive_type, folder, cleared_at, volume_of,
           unpacked_at, unpack_error, unchanged_asset, unpack_tries, unpack_failure,
           unpack_retry_at
    FROM subtitle_job_files";

/// A failure class column.
fn failure_at(r: &Row<'_>, i: usize) -> rusqlite::Result<Option<FailureKind>> {
    let code: Option<String> = r.get(i)?;
    code.map(|code| {
        FailureKind::parse(&code).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                i,
                rusqlite::types::Type::Text,
                format!("unknown failure class {code:?}").into(),
            )
        })
    })
    .transpose()
}

fn format_at(r: &Row<'_>, i: usize) -> rusqlite::Result<Option<Format>> {
    let code: Option<String> = r.get(i)?;
    code.map(|code| {
        Format::parse(&code).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                i,
                rusqlite::types::Type::Text,
                format!("unknown format {code:?}").into(),
            )
        })
    })
    .transpose()
}

pub(super) fn file_row(r: &Row<'_>) -> rusqlite::Result<FileRow> {
    Ok(FileRow {
        id: r.get(0)?,
        item_id: r.get(1)?,
        file_key: r.get(2)?,
        name: r.get(3)?,
        state: r.get(4)?,
        same_as: r.get(5)?,
        temp_dir: r.get(6)?,
        expected_size: r
            .get::<_, Option<i64>>(7)?
            .and_then(|s| u64::try_from(s).ok()),
        size: r
            .get::<_, Option<i64>>(8)?
            .and_then(|s| u64::try_from(s).ok()),
        sha256: r.get(9)?,
        object: r.get(10)?,
        path: r.get(11)?,
        reason: r.get(12)?,
        created_at: r.get(13)?,
        format: format_at(r, 14)?,
        failure: failure_at(r, 15)?,
        http_status: r.get(16)?,
        content_type: r.get(17)?,
        response_size: r
            .get::<_, Option<i64>>(18)?
            .and_then(|s| u64::try_from(s).ok()),
        snapshot: r.get(19)?,
        kind: kind_at(r, 20)?,
        archive: archive_at(r, 21)?,
        folder: r.get(22)?,
        cleared_at: r.get(23)?,
        volume_of: r.get(24)?,
        unpacked_at: r.get(25)?,
        unpack_error: r.get(26)?,
        unchanged_asset: r.get(27)?,
        unpack_tries: r.get(28)?,
        unpack_failure: r.get(29)?,
        unpack_retry_at: r.get(30)?,
    })
}

fn archive_at(r: &Row<'_>, i: usize) -> rusqlite::Result<Option<Archive>> {
    let code: Option<String> = r.get(i)?;
    code.map(|code| {
        Archive::parse(&code).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                i,
                rusqlite::types::Type::Text,
                format!("unknown archive type {code:?}").into(),
            )
        })
    })
    .transpose()
}

fn kind_at(r: &Row<'_>, i: usize) -> rusqlite::Result<Option<Kind>> {
    let code: Option<String> = r.get(i)?;
    code.map(|code| {
        Kind::parse(&code).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                i,
                rusqlite::types::Type::Text,
                format!("unknown file kind {code:?}").into(),
            )
        })
    })
    .transpose()
}

pub(super) fn items(c: &Connection, job_id: &str) -> Result<Vec<ItemRow>, JobError> {
    let mut stmt = c.prepare_cached(
        "SELECT id, position, observation_id, episode, post_url, state, wait, reason, failure,
                unchanged_from
         FROM subtitle_job_items WHERE job_id = ?1 ORDER BY position",
    )?;
    let mut items = stmt
        .query_map([job_id], |r| {
            Ok(ItemRow {
                id: r.get(0)?,
                position: r.get(1)?,
                observation_id: r.get(2)?,
                episode: r.get(3)?,
                post_url: r.get(4)?,
                state: r.get(5)?,
                wait: r.get(6)?,
                reason: r.get(7)?,
                failure: failure_at(r, 8)?,
                unchanged_from: r.get(9)?,
                files: Vec::new(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut files = c.prepare_cached(&format!(
        "{FILE_COLUMNS} WHERE item_id = ?1 ORDER BY created_at, id"
    ))?;
    for item in &mut items {
        item.files = files
            .query_map([item.id], file_row)?
            .collect::<Result<Vec<_>, _>>()?;
    }
    Ok(items)
}

pub(super) fn steps(c: &Connection, job_id: &str) -> Result<Vec<StepRow>, JobError> {
    let mut stmt =
        c.prepare_cached("SELECT step, state, at, note FROM subtitle_job_steps WHERE job_id = ?1")?;
    let rows = stmt
        .query_map([job_id], |r| {
            Ok(StepRow {
                step: r.get(0)?,
                state: r.get(1)?,
                at: r.get(2)?,
                note: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(
        seq: i64,
        origin: &str,
        state: JobState,
        wait: Option<Wait>,
        state_at: Millis,
    ) -> JobRow {
        JobRow {
            seq,
            id: format!("j{seq}"),
            origin: origin.to_owned(),
            revision_of: None,
            revises_job: None,
            revises_attributed: false,
            state,
            wait,
            stage: None,
            note: None,
            state_at,
            created_at: 0,
            finished_at: None,
            work_id: None,
            work_name: None,
            season: None,
            anime_no: None,
            anime_title: None,
            creator: None,
            episodes: Vec::new(),
            source: None,
            progress: Progress::default(),
            failure: None,
            upload: None,
            finishing: false,
            receiving: false,
        }
    }

    fn ids(rows: &[JobRow]) -> Vec<&str> {
        rows.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn a_job_waits_for_a_persons_check_unless_it_is_a_find_job_that_waits_for_browsing() {
        use JobState::*;
        // (state, wait, origin, waits for a check, waits for a placement)
        let cases = [
            (Waiting, Some(Wait::Auth), "pick", true, false),
            (Waiting, Some(Wait::Auth), "auto", true, false),
            (Waiting, Some(Wait::Auth), FIND, false, false),
            (Waiting, Some(Wait::Placement), "upload", false, true),
            (Waiting, Some(Wait::Placement), FIND, false, true),
            (Waiting, Some(Wait::Subtitle), "pick", false, false),
            (Waiting, Some(Wait::Approval), "pick", false, false),
            (Waiting, None, "pick", false, false),
            (Held, Some(Wait::Auth), "pick", false, false),
            (Running, Some(Wait::Placement), "pick", false, false),
            (Pending, None, "pick", false, false),
        ];
        for (state, wait, origin, check, placement) in cases {
            let row = job(1, origin, state, wait, 0);
            assert_eq!(row.waits_for_check(), check, "{state:?} {wait:?} {origin}");
            assert_eq!(
                row.waits_for_placement(),
                placement,
                "{state:?} {wait:?} {origin}"
            );
        }
    }

    #[test]
    fn the_open_jobs_are_grouped_failed_newest_first_and_waiting_by_what_they_wait_for() {
        use JobState::*;
        let open = vec![
            job(1, "pick", Pending, None, 50),
            job(2, "pick", Failed, None, 100),
            job(3, "pick", Held, None, 50),
            job(4, "pick", Waiting, Some(Wait::Subtitle), 50),
            job(5, "pick", Waiting, Some(Wait::Auth), 60),
            job(6, "pick", Partial, None, 200),
            job(7, FIND, Waiting, Some(Wait::Auth), 60),
            job(8, "pick", Running, None, 70),
            job(9, "pick", Waiting, Some(Wait::Auth), 60),
            job(10, "pick", Running, None, 30),
            // The same time as another failure: the later job first.
            job(11, "pick", Failed, None, 100),
        ];

        let groups = OpenGroups::of(open);

        assert_eq!(ids(&groups.failed), ["j6", "j11", "j2"]);
        // A person's check first (a find job is no check), then the other
        // waits, held, then pending, each oldest first.
        assert_eq!(ids(&groups.waiting), ["j5", "j9", "j4", "j7", "j3", "j1"]);
        assert_eq!(ids(&groups.running), ["j8", "j10"]);
    }
}
