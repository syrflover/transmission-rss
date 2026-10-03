//! The daily recheck of the files of the episodes received from the subscribed
//! creator (`docs/specs/subtitles.md`, 구독 제작자 자동 수신; ADR 0009).
//!
//! Anissia's line stays as it was when a creator fixes a post or its file, so
//! the fix shows only in the post. For 14 days after an episode of the
//! subscribed creator was received, this reads once a day what the source
//! tells about each file without receiving it ([`trss_subtitles::Source::recheck`]),
//! and when a file differs from the one received, makes a job that receives the
//! post again as a revision of the earlier receipt (`revision_of`, the job
//! [`crate::follow`] makes for a new update time of the line). The subtitle in
//! place stays until a replacement is approved.
//!
//! # What is rechecked
//!
//! An item of a job that is done, is the newest receipt of its episode of the
//! subscribed creator's source (the subscription's current creator; a job a
//! person picked counts as much as one the app made), whose episode no job
//! that has not finished holds, and that was received no more than
//! [`WINDOW`] ago. Only while the subscription takes part as for the
//! automatic receipt ([`Follow::subscribed`]: its rule is active, video and
//! subtitles are on, its season is linked to the anime) and the work's folder
//! is there. A receipt a revision job made starts its own 14 days, and the
//! receipt it revised is not read any more. The exception is a revision whose
//! files turned out to be the earlier receipt's bytes
//! (`subtitle_job_items.unchanged_from`) after a recheck made it: it starts no
//! new window (see the breaker below).
//!
//! # When
//!
//! A reading is due [`INTERVAL`] less [`SLACK`] after the receipt or the last
//! reading. The slack is for the worker's hourly look ([`Recheck::run`]): with
//! a strict day each reading would come up to an hour late and the 14th would
//! fall outside the window, so 14 readings are made in the 14 days.
//!
//! A reading is claimed in the database (`subtitle_item_rechecks.checked_at`)
//! before its first request, so a restart or another worker does not read the
//! item again the same day, and one cut short by a shutdown gives its claim
//! back with the reading it replaced. The claim clears that reading's result
//! (`result`, `observed`, `job_id`, `result_at`), so an item claimed and
//! killed shows no result rather than the last one. The pass reads the clock
//! to pick the items and again at the claim; the claim's time stamps the
//! reading, the job and its log. A reading dated after the clock by no more
//! than [`CLOCK_BACK`] is another worker's claim from a little later and
//! stands; one dated further ahead is from a clock that went back and does
//! not hold.
//!
//! Items that share a post are read together: the post is read once and each
//! key asked for once. Requests keep the per-host spacing of the sources
//! (the same [`Sources`] the runner has).
//!
//! # What is a difference
//!
//! Only a value of a file: its size (a Drive file's `Content-Length`, a
//! Tistory attachment's total in the `Content-Range` of a one-byte range
//! request, a Naver attachment's `attachFileSize`) against the size received,
//! and a Drive file's `Last-Modified` against the one it was received with. A
//! value the site does not give, or one the receipt has not, is not a
//! difference (a Naver `publish_date` that appears later is not either). A
//! post's modified time is never compared: a post fixed in its text only does
//! not make a job. So a Tistory or Naver file edited to the same size shows
//! nothing; a Drive file shows by its modified time as well.
//!
//! A difference makes the job `recheck:<item>:<digest of the new values>`, so
//! a second look at the same change (a restart, another worker, the next day)
//! finds that job and makes none; a file that changes again is another job. A
//! revision job that fails is not made again by itself.
//!
//! The breaker: a value that differs from the receipt systematically (a size
//! the recheck reads that is not the length of the bytes a receipt gets) or a
//! file saved again every day without a change of bytes would make a job every
//! day. So when a revision a recheck made received the same bytes as the
//! earlier receipt, the next readings compare the sizes with what that
//! recheck read (not with the receipt's), ignore `Last-Modified`, and run in
//! the window of the first receipt. A Drive file saved again with the same
//! bytes costs one job; one whose size then changes still makes one. A change
//! of content to the same size after such a job is not seen (as for Tistory
//! and Naver files).
//!
//! A file's `Last-Modified` from the receipt's `GET` is the string a `HEAD`
//! gives, and the bytes' length is the `Content-Length`/`Content-Range`/
//! `attachFileSize`; the ignored real-network test receives the files of a
//! real Drive, Tistory and Naver post and compares them with the recheck's
//! values (2026-10-03), so the first reading after a receipt finds no
//! difference.
//!
//! # What is not rechecked
//!
//! erulabo posts are out of scope until its source exists (ticket 0041): its
//! post's `dateModified` would have to become an `인증 필요` to-do and the
//! Drive ID of an authenticated receipt would have to be observed. A source
//! that implements [`trss_subtitles::Source::recheck`] for erulabo is where
//! that goes; the items of a post no source reads are recorded `unreadable`.
//! The same goes for Tistory's WinPNG images.
//!
//! # A file or post that is gone
//!
//! It is recorded (`missing`) and makes no job, unless the post offers another
//! file in its place: when a received key is missing, the post is read again
//! and opened for the episode as a receipt would (the source's own choice of
//! files); files it offers under keys the item did not receive are a
//! replacement (a Tistory attachment deleted and attached again has a new
//! address, a Naver one a new name, a Blogger or Tistory body a new Drive
//! link), which is a difference and makes the revision job. A Drive file is
//! asked with a `HEAD` only, and the post is read just when that says it is
//! gone: a link swapped to a new file while the old one still exists is not
//! seen. Otherwise it is read again the next day, within the 14 days, since a
//! post may be hidden for a while and a Drive file shared again; the window
//! ends the attempts.
//!
//! # Limits
//!
//! - An episode whose latest item is held or waiting (any item of the episode
//!   that is neither done nor failed) is not read for as long as it is.
//! - A follow job ([`crate::follow`]) and a recheck job can both be made for
//!   the same episode within a short time, one for a new line of Anissia and
//!   one for a difference in the files; the second finds the episode held
//!   only if the first is already in line.
//! - After two works are merged the earlier receipts keep the old work id and
//!   drop out of the recheck.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{params, TransactionBehavior};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use trss_collect::store::anissia::episode_key;
use trss_core::{Clock, Db, DbError, Millis};
use trss_subtitles::{Failure, FailureKind, FileInfo, Opened, PostFile, Sources};
use url::Url;

use crate::{
    follow::{Follow, FollowError, Subscribed},
    store::{JobError, JobStore, NewItem, NewJob, AUTO},
    Created, ItemState,
};

const DAY: Millis = 24 * 60 * 60 * 1000;

/// How long after its receipt an episode is read again.
pub const WINDOW: Millis = 14 * DAY;

/// About how often an item is read: once a day.
pub const INTERVAL: Millis = DAY;

/// How much earlier than [`INTERVAL`] a reading is due. The worker looks for
/// due items hourly, so a strict day would push each reading up to an hour
/// past the last and the 14th beyond the window.
pub const SLACK: Millis = 90 * 60 * 1000;

/// How far ahead of the clock a reading may be dated before the clock is taken
/// to have gone back (a reading another worker's later pass made is a little
/// ahead of this one's clock, and still counts).
pub const CLOCK_BACK: Millis = 60 * 60 * 1000;

/// What the request ID of the job a recheck makes starts with.
pub const COMMAND_PREFIX: &str = "recheck:";

#[derive(Debug, thiserror::Error)]
pub enum RecheckError {
    #[error(transparent)]
    Follow(#[from] FollowError),
    #[error(transparent)]
    Jobs(#[from] JobError),
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

type Result<T> = std::result::Result<T, RecheckError>;

/// How a reading of an item came out (`subtitle_item_rechecks.result`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Every file is as received.
    Same,
    /// A file differs: a job receives the post again.
    Changed,
    /// The post or a file is gone.
    Missing,
    /// The site could not be reached.
    Failed,
    /// No source of this build reads the post, or the site told nothing the
    /// check can use.
    Unreadable,
}

impl Verdict {
    pub fn code(self) -> &'static str {
        match self {
            Verdict::Same => "same",
            Verdict::Changed => "changed",
            Verdict::Missing => "missing",
            Verdict::Failed => "failed",
            Verdict::Unreadable => "unreadable",
        }
    }
}

/// What one pass read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// The items read, by how they came out.
    pub same: usize,
    pub changed: usize,
    pub missing: usize,
    pub failed: usize,
    pub unreadable: usize,
    /// The jobs the pass made, for the runner to be woken for.
    pub jobs: Vec<String>,
}

impl Report {
    pub fn read(&self) -> usize {
        self.same + self.changed + self.missing + self.failed + self.unreadable
    }

    fn count(&mut self, verdict: Verdict) {
        match verdict {
            Verdict::Same => self.same += 1,
            Verdict::Changed => self.changed += 1,
            Verdict::Missing => self.missing += 1,
            Verdict::Failed => self.failed += 1,
            Verdict::Unreadable => self.unreadable += 1,
        }
    }

    fn add(&mut self, other: Report) {
        self.same += other.same;
        self.changed += other.changed;
        self.missing += other.missing;
        self.failed += other.failed;
        self.unreadable += other.unreadable;
        self.jobs.extend(other.jobs);
    }
}

/// A file as it was received.
#[derive(Debug, Clone)]
struct Known {
    key: String,
    name: String,
    size: Option<u64>,
    last_modified: Option<String>,
}

/// A reading as the database keeps it, to give back when a newer one is cut
/// short.
#[derive(Debug, Clone)]
struct Prev {
    checked_at: Millis,
    checks: i64,
    result: Option<String>,
    observed: Option<String>,
    job_id: Option<String>,
    result_at: Option<Millis>,
}

/// A received item to read again.
#[derive(Debug, Clone)]
struct Due {
    id: i64,
    observation: i64,
    episode: String,
    post_url: String,
    found_at: Millis,
    received_at: Millis,
    /// The reading before this one.
    before: Option<Prev>,
    /// For an item received again with the same bytes, the sizes the recheck
    /// that made its job read (the breaker, see the module docs).
    trigger: Option<HashMap<String, Option<u64>>>,
    /// When the claim took this reading: what the reading is stamped with.
    claimed: Millis,
    files: Vec<Known>,
}

/// A difference found in one file.
#[derive(Debug, Clone)]
struct Change {
    name: String,
    key: String,
    now: FileInfo,
    was_size: Option<u64>,
    /// The post offers this file in place of one that is gone.
    replaced: bool,
}

type Answers = Option<HashMap<String, std::result::Result<FileInfo, Failure>>>;

/// The recheck of the received episodes' files. Cheap to clone.
#[derive(Clone)]
pub struct Recheck {
    db: Db,
    jobs: JobStore,
    follow: Follow,
    sources: Sources,
}

impl Recheck {
    /// A recheck on `db` asking `sources` (the runner's, so the pace per host
    /// is shared).
    pub fn new(db: Db, sources: Sources) -> Recheck {
        Recheck {
            jobs: JobStore::new(db.clone()),
            follow: Follow::new(db.clone()),
            db,
            sources,
        }
    }

    /// One pass over every subscription: reads the items that are due and
    /// makes the jobs for the files that differ (see the module docs). The
    /// pass looks at `clock` when it picks the items and again when it claims
    /// each one, and stamps a reading with the time of its claim, so passes of
    /// different workers that start at different times agree on what was
    /// claimed. A subscription that fails is logged and the others go on.
    /// Returns when `cancel` fires, with what it had read.
    pub async fn run(&self, clock: &Clock, cancel: &CancellationToken) -> Result<Report> {
        let mut report = Report::default();
        for sub in self.follow.subscribed().await? {
            if cancel.is_cancelled() {
                break;
            }
            let Some(creator) = sub.creator.clone() else {
                continue;
            };
            match self.subscription(&sub, &creator, clock, cancel).await {
                Ok(read) => report.add(read),
                Err(e) => eprintln!(
                    "Subtitle recheck of work {} season {} (rule {}): {e}",
                    sub.work_id, sub.season, sub.rule_id
                ),
            }
        }
        Ok(report)
    }

    async fn subscription(
        &self,
        sub: &Subscribed,
        creator: &str,
        clock: &Clock,
        cancel: &CancellationToken,
    ) -> Result<Report> {
        let mut report = Report::default();
        if !self.follow.folder_present(&sub.work_id).await? {
            return Ok(report);
        }
        let Some(source_id) = self.follow.creator_source(sub, creator).await? else {
            return Ok(report);
        };
        let due = self.due(sub, &source_id, clock()).await?;

        // The items of a post are read together.
        let mut posts: BTreeMap<String, Vec<Due>> = BTreeMap::new();
        for item in due {
            posts.entry(item.post_url.clone()).or_default().push(item);
        }
        for (post, items) in posts {
            if cancel.is_cancelled() {
                break;
            }
            // Claimed before the first request, by whichever process comes first.
            let mut claimed = Vec::new();
            for mut item in items {
                if let Some(at) = self.claim(item.id, clock).await? {
                    item.claimed = at;
                    claimed.push(item);
                }
            }
            if claimed.is_empty() {
                continue;
            }
            let answers = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    for item in &claimed {
                        self.release(item).await?;
                    }
                    break;
                }
                answers = self.ask(&post, &claimed) => answers,
            };
            for item in claimed {
                let verdict = self
                    .judge(sub, &source_id, creator, &item, &answers, &mut report)
                    .await?;
                report.count(verdict);
            }
        }
        Ok(report)
    }

    /// The items of the subscription to read at `now`: the newest receipt of
    /// each episode of the source (see the module docs).
    async fn due(&self, sub: &Subscribed, source_id: &str, now: Millis) -> Result<Vec<Due>> {
        let (anime_no, work_id, source) = (sub.anime_no, sub.work_id.clone(), source_id.to_owned());
        struct Row {
            id: i64,
            observation: i64,
            episode: String,
            post_url: String,
            found_at: Millis,
            state: ItemState,
            updated_at: Millis,
            checked: Option<Prev>,
            trigger: Option<HashMap<String, Option<u64>>>,
        }
        let rows: Vec<Row> = self
            .db
            .run(move |c| {
                // `trigger`: what the recheck that made the job of an item
                // received again with the same bytes had read.
                let mut stmt = c.prepare(
                    "SELECT i.id, i.observation_id, i.episode, i.post_url, i.found_at, i.state,
                            i.updated_at, r.checked_at, r.checks, r.result, r.observed,
                            r.job_id, r.result_at,
                            CASE WHEN i.unchanged_from IS NOT NULL THEN
                                (SELECT t.observed FROM subtitle_item_rechecks t
                                  WHERE t.job_id = i.job_id LIMIT 1)
                            END
                       FROM subtitle_job_items i
                       JOIN subtitle_jobs j ON j.id = i.job_id
                       LEFT JOIN subtitle_item_rechecks r ON r.item_id = i.id
                      WHERE j.anime_no = ?1 AND j.source_id = ?2 AND j.work_id = ?3
                        AND i.observation_id IS NOT NULL
                      ORDER BY i.id",
                )?;
                let rows = stmt
                    .query_map(params![anime_no, source, work_id], |r| {
                        let checked_at: Option<Millis> = r.get(7)?;
                        let checked = match checked_at {
                            Some(checked_at) => Some(Prev {
                                checked_at,
                                checks: r.get(8)?,
                                result: r.get(9)?,
                                observed: r.get(10)?,
                                job_id: r.get(11)?,
                                result_at: r.get(12)?,
                            }),
                            None => None,
                        };
                        let trigger: Option<String> = r.get(13)?;
                        Ok(Row {
                            id: r.get(0)?,
                            observation: r.get(1)?,
                            episode: r.get(2)?,
                            post_url: r.get(3)?,
                            found_at: r.get(4)?,
                            state: r.get(5)?,
                            updated_at: r.get(6)?,
                            checked,
                            trigger: trigger.as_deref().map(sizes_of),
                        })
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok::<_, RecheckError>(rows)
            })
            .await?;

        // The newest receipt of each episode, unless a job still holds it.
        let mut episodes: BTreeMap<String, Vec<&Row>> = BTreeMap::new();
        for row in &rows {
            episodes
                .entry(episode_key(&row.episode))
                .or_default()
                .push(row);
        }
        let mut due = Vec::new();
        for items in episodes.values() {
            if items
                .iter()
                .any(|i| !matches!(i.state, ItemState::Done | ItemState::Failed))
            {
                continue;
            }
            // The window runs from the receipt that was not the same bytes
            // again: a receipt that replaced nothing does not start a new one.
            let mut anchor = None;
            for item in items.iter().filter(|i| i.state == ItemState::Done) {
                anchor = Some(match (item.trigger.is_some(), anchor) {
                    (true, Some(at)) => at,
                    _ => item.updated_at,
                });
            }
            let (Some(newest), Some(anchor)) = (
                items.iter().rev().find(|i| i.state == ItemState::Done),
                anchor,
            ) else {
                continue;
            };
            if now - anchor > WINDOW {
                continue;
            }
            let last = newest
                .checked
                .as_ref()
                .map_or(newest.updated_at, |p| p.checked_at);
            // A reading dated well after now is from a clock that went back.
            let ripe = now - last >= INTERVAL - SLACK || last - now > CLOCK_BACK;
            if !ripe {
                continue;
            }
            due.push(Due {
                id: newest.id,
                observation: newest.observation,
                episode: newest.episode.clone(),
                post_url: newest.post_url.clone(),
                found_at: newest.found_at,
                received_at: newest.updated_at,
                before: newest.checked.clone(),
                trigger: newest.trigger.clone(),
                claimed: now,
                files: Vec::new(),
            });
        }
        for item in &mut due {
            item.files = self.known(item.id).await?;
            // The breaker: an item received again with the same bytes is
            // compared with what read made it, by size alone.
            if let Some(trigger) = &item.trigger {
                for file in &mut item.files {
                    file.last_modified = None;
                    if let Some(Some(size)) = trigger.get(&file.key) {
                        file.size = Some(*size);
                    }
                }
            }
        }
        due.retain(|item| !item.files.is_empty());
        Ok(due)
    }

    /// The files received for an item, as they were received.
    async fn known(&self, item_id: i64) -> Result<Vec<Known>> {
        self.db
            .run(move |c| {
                let mut stmt = c.prepare(
                    "SELECT file_key, name, size, snapshot FROM subtitle_job_files
                      WHERE item_id = ?1 AND state = 'done' ORDER BY created_at, id",
                )?;
                let rows = stmt.query_map([item_id], |r| {
                    let snapshot: Option<String> = r.get(3)?;
                    Ok(Known {
                        key: r.get(0)?,
                        name: r.get(1)?,
                        size: r
                            .get::<_, Option<i64>>(2)?
                            .and_then(|s| u64::try_from(s).ok()),
                        last_modified: snapshot.as_deref().and_then(last_modified_of),
                    })
                })?;
                let mut files: Vec<Known> = Vec::new();
                for file in rows {
                    let file = file?;
                    if !files.iter().any(|f| f.key == file.key) {
                        files.push(file);
                    }
                }
                Ok::<_, RecheckError>(files)
            })
            .await
    }

    /// Takes the item's reading at the time `clock` shows now: the time it
    /// took it when it is the caller's. The previous reading's result goes, to
    /// come back if the reading is cut short ([`Self::release`]).
    async fn claim(&self, item_id: i64, clock: &Clock) -> Result<Option<Millis>> {
        let clock = clock.clone();
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let now = clock();
                let taken = tx.execute(
                    "INSERT INTO subtitle_item_rechecks (item_id, checked_at, checks)
                     VALUES (?1, ?2, 1)
                     ON CONFLICT (item_id) DO UPDATE
                     SET checked_at = ?2, checks = checks + 1,
                         result = NULL, observed = NULL, job_id = NULL, result_at = NULL
                     WHERE ?2 - checked_at >= ?3 OR checked_at - ?2 > ?4",
                    params![item_id, now, INTERVAL - SLACK, CLOCK_BACK],
                )?;
                tx.commit()?;
                Ok::<_, RecheckError>((taken == 1).then_some(now))
            })
            .await
    }

    /// Gives back a claim whose reading was cut short: the reading before it.
    async fn release(&self, item: &Due) -> Result<()> {
        let (id, claimed, before) = (item.id, item.claimed, item.before.clone());
        self.db
            .run(move |c| {
                match before {
                    Some(p) => c.execute(
                        "UPDATE subtitle_item_rechecks
                         SET checked_at = ?2, checks = ?3, result = ?4, observed = ?5,
                             job_id = ?6, result_at = ?7
                         WHERE item_id = ?1 AND checked_at = ?8",
                        params![
                            id,
                            p.checked_at,
                            p.checks,
                            p.result,
                            p.observed,
                            p.job_id,
                            p.result_at,
                            claimed
                        ],
                    )?,
                    None => c.execute(
                        "DELETE FROM subtitle_item_rechecks
                         WHERE item_id = ?1 AND checked_at = ?2 AND result IS NULL",
                        params![id, claimed],
                    )?,
                };
                Ok::<_, RecheckError>(())
            })
            .await
    }

    /// Asks the source of `post` for the files of `items`, one answer per
    /// key; a post no source reads answers none.
    async fn ask(&self, post: &str, items: &[Due]) -> Answers {
        let url = Url::parse(post).ok()?;
        let source = self.sources.for_post(&url)?;
        let mut keys: Vec<String> = Vec::new();
        for file in items.iter().flat_map(|i| &i.files) {
            if !keys.contains(&file.key) {
                keys.push(file.key.clone());
            }
        }
        Some(source.recheck(&url, &keys).await.into_iter().collect())
    }

    /// The files the post offers for the item's episode now (as a receipt
    /// would choose them) that the item did not receive: what replaced a file
    /// that is gone. Empty when the post cannot be read again.
    async fn replacements(&self, item: &Due) -> Vec<PostFile> {
        let Ok(url) = Url::parse(&item.post_url) else {
            return Vec::new();
        };
        let Some(source) = self.sources.for_post(&url) else {
            return Vec::new();
        };
        match source.open(&url, &item.episode).await {
            Ok(Opened::Files(files)) => files
                .into_iter()
                .filter(|f| !item.files.iter().any(|k| k.key == f.key))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Compares what was read with what was received, records the reading,
    /// and makes the job when a file differs.
    async fn judge(
        &self,
        sub: &Subscribed,
        source_id: &str,
        creator: &str,
        item: &Due,
        answers: &Answers,
        report: &mut Report,
    ) -> Result<Verdict> {
        let now = item.claimed;
        let mut changes: Vec<Change> = Vec::new();
        let mut worst: Option<Verdict> = None;
        let mut observed = Vec::new();
        let mut note = |v: Verdict| {
            let rank = |v: Verdict| match v {
                Verdict::Missing => 3,
                Verdict::Failed => 2,
                Verdict::Unreadable => 1,
                _ => 0,
            };
            if worst.is_none_or(|w| rank(v) > rank(w)) {
                worst = Some(v);
            }
        };
        for file in &item.files {
            let answer = answers.as_ref().and_then(|a| a.get(&file.key));
            match answer {
                None => {
                    note(Verdict::Unreadable);
                    observed.push(json!({"key": file.key, "problem": "no_source"}));
                }
                Some(Err(failure)) => {
                    note(match failure.kind {
                        FailureKind::Missing => Verdict::Missing,
                        FailureKind::Network => Verdict::Failed,
                        _ => Verdict::Unreadable,
                    });
                    observed.push(json!({"key": file.key, "problem": failure.kind.code()}));
                }
                Some(Ok(info)) => {
                    observed.push(json!({
                        "key": file.key,
                        "size": info.size,
                        "last_modified": info.last_modified,
                    }));
                    let size_differs =
                        matches!((info.size, file.size), (Some(a), Some(b)) if a != b);
                    let modified_differs = matches!(
                        (&info.last_modified, &file.last_modified),
                        (Some(a), Some(b)) if a != b
                    );
                    if size_differs || modified_differs {
                        changes.push(Change {
                            name: file.name.clone(),
                            key: file.key.clone(),
                            now: info.clone(),
                            was_size: file.size,
                            replaced: false,
                        });
                    }
                }
            }
        }
        // A file that is gone may have been attached again under another key.
        if worst == Some(Verdict::Missing) {
            for file in self.replacements(item).await {
                observed.push(json!({"key": file.key, "replacement": true}));
                changes.push(Change {
                    name: file.name,
                    key: file.key,
                    now: FileInfo::default(),
                    was_size: None,
                    replaced: true,
                });
            }
        }
        let verdict = match (changes.is_empty(), worst) {
            (false, _) => Verdict::Changed,
            (true, Some(problem)) => problem,
            (true, None) => Verdict::Same,
        };

        let job = match verdict {
            Verdict::Changed => Some(
                self.revision(sub, source_id, creator, item, &changes, now)
                    .await?,
            ),
            _ => None,
        };
        if let Some((id, true)) = &job {
            report.jobs.push(id.clone());
        }
        let (id, observed, job_id) = (
            item.id,
            serde_json::Value::Array(observed).to_string(),
            job.map(|(id, _)| id),
        );
        self.db
            .run(move |c| {
                c.execute(
                    "UPDATE subtitle_item_rechecks
                     SET result = ?2, observed = ?3, job_id = ?4, result_at = ?5
                     WHERE item_id = ?1",
                    params![id, verdict.code(), observed, job_id, now],
                )?;
                Ok::<_, RecheckError>(())
            })
            .await?;
        println!(
            "Subtitle recheck of episode {} (received {} days ago): {}",
            item.episode,
            (now - item.received_at) / DAY,
            verdict.code()
        );
        Ok(verdict)
    }

    /// The job that receives the post of `item` again, as a revision of the
    /// item's receipt. Returns its ID and whether this call made it.
    async fn revision(
        &self,
        sub: &Subscribed,
        source_id: &str,
        creator: &str,
        item: &Due,
        changes: &[Change],
        now: Millis,
    ) -> Result<(String, bool)> {
        // The new values name the change, so the same change is one job.
        let mut lines: Vec<String> = changes
            .iter()
            .map(|c| {
                format!(
                    "{}\t{}\t{}\t{}",
                    c.key,
                    c.replaced,
                    c.now.size.map_or_else(String::new, |s| s.to_string()),
                    c.now.last_modified.as_deref().unwrap_or_default()
                )
            })
            .collect();
        lines.sort();
        let digest = Sha256::digest(lines.join("\n").as_bytes());
        let digest: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
        let request = json!({
            "recheck": item.id,
            "observation": item.observation,
            "files": changes.iter().map(|c| json!({
                "key": c.key,
                "size": c.now.size,
                "last_modified": c.now.last_modified,
                "replaced": c.replaced,
            })).collect::<Vec<_>>(),
        })
        .to_string();
        let job = NewJob {
            command_id: format!("{COMMAND_PREFIX}{}:{digest}", item.id),
            request,
            origin: AUTO.to_owned(),
            work_id: Some(sub.work_id.clone()),
            season: Some(i64::from(sub.season)),
            anime_no: Some(sub.anime_no),
            source_id: Some(source_id.to_owned()),
            creator: Some(creator.to_owned()),
            revision_of: Some(item.observation),
            revises_attributed: false,
            items: vec![NewItem {
                observation_id: Some(item.observation),
                episode: item.episode.clone(),
                post_url: item.post_url.clone(),
                found_at: item.found_at,
            }],
        };
        match self.jobs.create(job, now).await? {
            Created::Created(id) => {
                let detail = changes.iter().map(describe).collect::<Vec<_>>().join(" · ");
                self.jobs
                    .event(
                        &id,
                        "받은 파일의 정보가 받은 때와 달라서 다시 받아요".to_owned(),
                        Some(detail),
                        now,
                    )
                    .await?;
                Ok((id, true))
            }
            Created::Existing(id) | Created::Mismatch(id) => Ok((id, false)),
        }
    }
}

/// `name: 1,000 → 1,200 바이트`, `name: 수정 시각이 달라요`,
/// `name: 다른 파일로 바뀌었어요`.
fn describe(change: &Change) -> String {
    if change.replaced {
        return format!("{}: 다른 파일로 바뀌었어요", change.name);
    }
    let size = match (change.was_size, change.now.size) {
        (Some(was), Some(now)) if was != now => Some(format!("{was} → {now}바이트")),
        _ => None,
    };
    match (size, &change.now.last_modified) {
        (Some(size), _) => format!("{}: {size}", change.name),
        (None, Some(_)) => format!("{}: 수정 시각이 달라요", change.name),
        (None, None) => change.name.clone(),
    }
}

/// The sizes of a reading's `observed` JSON, by file key.
fn sizes_of(observed: &str) -> HashMap<String, Option<u64>> {
    let values: Vec<serde_json::Value> = serde_json::from_str(observed).unwrap_or_default();
    values
        .iter()
        .filter_map(|v| {
            let key = v.get("key")?.as_str()?.to_owned();
            Some((key, v.get("size").and_then(serde_json::Value::as_u64)))
        })
        .collect()
}

/// The `last_modified` of a snapshot (a JSON array of `[name, value]` pairs).
fn last_modified_of(snapshot: &str) -> Option<String> {
    let pairs: Vec<[String; 2]> = serde_json::from_str(snapshot).ok()?;
    pairs
        .into_iter()
        .find(|[name, _]| name == trss_subtitles::http::LAST_MODIFIED)
        .map(|[_, value]| value)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    type Row = (
        i64,
        i64,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
    );

    async fn with_item() -> (tempfile::TempDir, Recheck) {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("app.db")).await.unwrap();
        db.run::<_, DbError, _>(|c| {
            c.execute_batch(
                "INSERT INTO subtitle_jobs (id, command_id, request, origin, state,
                     created_at, updated_at, state_at)
                     VALUES ('j1', 'c1', '{}', 'auto', 'done', 1, 1, 1);
                 INSERT INTO subtitle_job_items (job_id, position, episode, post_url, found_at,
                     state, updated_at)
                     VALUES ('j1', 0, '1', 'https://a.test/1', 6, 'done', 1);
                 INSERT INTO subtitle_item_rechecks
                     (item_id, checked_at, checks, result, observed, job_id, result_at)
                     VALUES (1, 100, 3, 'changed', '[]', 'j1', 100);",
            )?;
            Ok(())
        })
        .await
        .unwrap();
        (dir, Recheck::new(db, Sources::none()))
    }

    async fn row(recheck: &Recheck) -> Row {
        recheck
            .db
            .run::<_, DbError, _>(|c| {
                Ok(c.query_row(
                    "SELECT checked_at, checks, result, observed, job_id, result_at
                       FROM subtitle_item_rechecks",
                    [],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                        ))
                    },
                )?)
            })
            .await
            .unwrap()
    }

    fn at(now: Millis) -> Clock {
        Arc::new(move || now)
    }

    fn due(before: Option<Prev>, claimed: Millis) -> Due {
        Due {
            id: 1,
            observation: 0,
            episode: "1".to_owned(),
            post_url: "https://a.test/1".to_owned(),
            found_at: 6,
            received_at: 1,
            before,
            trigger: None,
            claimed,
            files: Vec::new(),
        }
    }

    const HOUR: Millis = 60 * 60 * 1000;

    #[tokio::test]
    async fn a_claim_clears_the_last_result_and_a_release_gives_it_back() {
        let (_dir, recheck) = with_item().await;
        let before = row(&recheck).await;

        // Too soon (and not from a clock that went back): not the caller's.
        assert_eq!(recheck.claim(1, &at(100 + HOUR)).await.unwrap(), None);
        assert_eq!(row(&recheck).await, before);

        // A day later it is, and what the last reading said is gone while this
        // one is under way.
        let now = 100 + DAY;
        assert_eq!(recheck.claim(1, &at(now)).await.unwrap(), Some(now));
        assert_eq!(row(&recheck).await, (now, 4, None, None, None, None));
        // Nobody else takes it meanwhile, even a pass that began earlier.
        assert_eq!(
            recheck.claim(1, &at(now - 5 * 60 * 1000)).await.unwrap(),
            None
        );

        // A release of a claim someone else replaced changes nothing.
        recheck.release(&due(None, now - 1)).await.unwrap();
        assert_eq!(row(&recheck).await, (now, 4, None, None, None, None));
        // The reading is cut short: the earlier one is given back whole.
        let prev = Prev {
            checked_at: 100,
            checks: 3,
            result: Some("changed".to_owned()),
            observed: Some("[]".to_owned()),
            job_id: Some("j1".to_owned()),
            result_at: Some(100),
        };
        recheck.release(&due(Some(prev), now)).await.unwrap();
        assert_eq!(row(&recheck).await, before);
    }

    #[tokio::test]
    async fn a_first_reading_cut_short_leaves_no_row() {
        let (_dir, recheck) = with_item().await;
        recheck
            .db
            .run::<_, DbError, _>(|c| {
                c.execute("DELETE FROM subtitle_item_rechecks", [])?;
                Ok(())
            })
            .await
            .unwrap();
        assert_eq!(recheck.claim(1, &at(500)).await.unwrap(), Some(500));
        recheck.release(&due(None, 500)).await.unwrap();
        let rows: i64 = recheck
            .db
            .run::<_, DbError, _>(|c| {
                Ok(
                    c.query_row("SELECT count(*) FROM subtitle_item_rechecks", [], |r| {
                        r.get(0)
                    })?,
                )
            })
            .await
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn the_sizes_of_a_reading_are_by_file_key() {
        let sizes = sizes_of(
            r#"[{"key":"a","size":5,"last_modified":null},
                {"key":"b","size":null},{"key":"c","problem":"missing"},{"size":1}]"#,
        );
        assert_eq!(sizes.get("a"), Some(&Some(5)));
        assert_eq!(sizes.get("b"), Some(&None));
        assert_eq!(sizes.get("c"), Some(&None));
        assert_eq!(sizes.len(), 3);
        assert!(sizes_of("not json").is_empty());
    }
}
