//! Subtitles and fonts a person uploads (`docs/specs/subtitles.md`, 직접
//! 찾기와 자막 올리기).
//!
//! An upload makes one job whose files are one subtitle package. It is made
//! in three steps, none of which touches the library:
//!
//! 1. [`Uploads::begin`] takes a turn (at most [`UPLOAD_SLOTS`] uploads are
//!    taken in at once) and a private folder `.tmp/<id>/` of the receive area.
//!    The web streams each file into it with [`Staging::start`],
//!    [`Staging::write`] and [`Staging::end`], which hold the upload to its
//!    [`Limits`] before a byte more is stored, so a file or a total past a
//!    limit leaves nothing behind (dropping the [`Staging`] removes the
//!    folder).
//! 2. [`Uploads::finish`] judges every staged file by its bytes
//!    ([`trss_subtitles::upload::judge`]), keeps subtitles, fonts and ZIPs
//!    under safe names in the job's folder `<job>/` of the receive area, and
//!    records the job with the files it kept and the names and reasons of
//!    those it dropped, `pending` for the worker to unpack
//!    ([`crate::place::unpack`]) and analyse them for the person's
//!    배치 확인 ([`crate::place`]).
//! 3. A job is made once for a command ID: a repeat of the same upload finds
//!    the job it made, and the files of that repeat are removed.
//!
//! A person's file names are not trusted: a name (a folder's relative path
//! included) is cut into parts, each made a safe name
//! ([`crate::area::safe_name`]), with `.`/`..` and empty parts gone, so no
//! name leaves the job's folder.
//!
//! A crash between the files being moved into `<job>/` and the job being
//! recorded leaves a folder no job names, and a kill while a body is read
//! leaves its `.tmp/<id>/`. [`Uploads::finish`] runs to its end even when the
//! request is dropped (a client that goes away after its body was read), so
//! only a killed process leaves these; [`Uploads::sweep`] removes the ones
//! that are old enough not to belong to an upload still going on.

use std::{
    collections::{HashMap, HashSet},
    io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, SystemTime},
};

use sha2::{Digest, Sha256};
use tokio::{
    io::AsyncWriteExt,
    sync::{OwnedSemaphorePermit, Semaphore},
};
use trss_core::Millis;
pub use trss_subtitles::upload::Kind;
use trss_subtitles::upload::{
    is_archive_name, is_vobsub_index, is_zip_signed, judge, volume_anchor, Archive, Kept, Verdict,
    INFLATE_BUDGET, NOT_AN_ARCHIVE, NOT_THEM, NO_FIRST_VOLUME,
};

use crate::{
    area::{self, ReceiveArea},
    store::{Created, JobError, JobStore, NewUpload, UploadedFile},
};

/// How many uploads are taken in at once. Each streams to disk and holds no
/// body in memory, so this bounds the disk and the checking (a ZIP is read to
/// its end) a burst of uploads costs.
pub const UPLOAD_SLOTS: usize = 2;
/// The most files one upload may send (the ones the browser thinks are
/// subtitles, fonts or ZIPs).
pub const MAX_FILES: usize = 500;
/// The most files one upload may name in all, those left out before sending
/// included.
pub const MAX_ENTRIES: usize = 2000;
/// The most bytes the files of one upload may have in all: 1 GiB. A season's
/// pack is subtitles of a few hundred KB and a few fonts of up to some tens of
/// MB; a ZIP of a whole series with its fonts stays within a few hundred MB.
pub const MAX_TOTAL_BYTES: u64 = 1 << 30;
/// The most bytes one file may have: [`trss_subtitles::MAX_FILE_BYTES`],
/// the size the receipts of every source are held to.
pub const MAX_FILE_BYTES: u64 = trss_subtitles::MAX_FILE_BYTES;
/// How long reading one upload's body may take in all: a file of the largest
/// size at 1.5 MB/s.
pub const BODY_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// How long a body may go without a byte before it is dropped.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// The least average speed a body must keep once [`RATE_GRACE`] has passed: a
/// sender that trickles bytes just often enough to dodge the idle timeout
/// would hold one of the [`UPLOAD_SLOTS`] for the whole [`BODY_TIMEOUT`].
pub const MIN_RATE: u64 = 16 * 1024;
/// How long a body is let go below [`MIN_RATE`] while it starts.
pub const RATE_GRACE: Duration = Duration::from_secs(30);
/// How many uploads may wait for a turn while both slots are busy; the next
/// one is told the server is busy at once.
pub const QUEUE_MAX: usize = 4;
/// How long an upload waits for a turn before it is told the server is busy.
pub const QUEUE_WAIT: Duration = Duration::from_secs(60);
/// How many parts of a relative path a name keeps (the last ones).
const MAX_PARTS: usize = 8;

/// The limits an upload is held to (see the constants).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub files: usize,
    pub entries: usize,
    pub total_bytes: u64,
    pub file_bytes: u64,
    pub body_timeout: Duration,
    pub idle_timeout: Duration,
    /// Bytes per second a body must average after `rate_grace`.
    pub min_rate: u64,
    pub rate_grace: Duration,
    /// How many uploads may wait for a turn, and for how long.
    pub queue: usize,
    pub queue_wait: Duration,
    /// Bytes the ZIPs of one upload may inflate in all while they are checked.
    pub inflate_budget: u64,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            files: MAX_FILES,
            entries: MAX_ENTRIES,
            total_bytes: MAX_TOTAL_BYTES,
            file_bytes: MAX_FILE_BYTES,
            body_timeout: BODY_TIMEOUT,
            idle_timeout: IDLE_TIMEOUT,
            min_rate: MIN_RATE,
            rate_grace: RATE_GRACE,
            queue: QUEUE_MAX,
            queue_wait: QUEUE_WAIT,
            inflate_budget: INFLATE_BUDGET,
        }
    }
}

/// Why an upload was refused or failed.
#[derive(Debug, thiserror::Error)]
pub enum UploadError {
    #[error("more than {0} files")]
    TooManyFiles(usize),
    #[error("more than {0} files named in all")]
    TooManyEntries(usize),
    #[error("more than {0} bytes in all")]
    TotalTooLarge(u64),
    #[error("a file of more than {0} bytes")]
    FileTooLarge(u64),
    #[error("no file is being written")]
    NoFile,
    /// Both turns are taken and the queue is full or the wait ran out.
    #[error("the server is taking in other uploads")]
    Busy,
    #[error("cannot store the upload: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Job(#[from] JobError),
}

/// Whether `name` is a folder where a reading of WinPNG images puts its files
/// (`winpng-` and the job's UUID; see [`crate::Runner`]).
fn is_winpng_staging(name: &str) -> bool {
    name.strip_prefix("winpng-")
        .is_some_and(|id| uuid::Uuid::try_parse(id).is_ok_and(|u| u.to_string() == id))
}

/// The item a folder holds the file of a site's check for (`check-`, the
/// job's UUID, `-` and the item's ID; see [`crate::Runner`]).
fn check_staging_item(name: &str) -> Option<i64> {
    let rest = name.strip_prefix("check-")?;
    let (job, item) = (rest.get(..36)?, rest.get(36..)?.strip_prefix('-')?);
    let canonical = uuid::Uuid::try_parse(job).is_ok_and(|u| u.to_string() == job);
    let item_id = item.parse::<i64>().ok()?;
    (canonical && item_id.to_string() == item).then_some(item_id)
}

/// The uploads of a process: the turns, the limits, and where files go.
#[derive(Clone)]
pub struct Uploads {
    store: JobStore,
    area: ReceiveArea,
    slots: Arc<Semaphore>,
    waiting: Arc<AtomicUsize>,
    limits: Limits,
}

/// One upload waiting for a turn, counted while it does.
struct Waiting(Arc<AtomicUsize>);

impl Drop for Waiting {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Uploads {
    pub fn new(store: JobStore, area: ReceiveArea) -> Uploads {
        Uploads {
            store,
            area,
            slots: Arc::new(Semaphore::new(UPLOAD_SLOTS)),
            waiting: Arc::new(AtomicUsize::new(0)),
            limits: Limits::default(),
        }
    }

    /// Holds uploads to `limits` instead (tests).
    pub fn with_limits(mut self, limits: Limits) -> Uploads {
        self.limits = limits;
        self
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Removes what a killed process left in the receive area: the folders
    /// `.tmp/<id>/` and `<id>/` whose name is a UUID (what an upload makes)
    /// that no job or file record names and that nothing wrote for `older_than`
    /// (an upload or a receipt going on is younger, and a receipt's attempt
    /// folder always has its record), and the folders `.tmp/winpng-<job id>/`
    /// (the files a reading of a post's WinPNG images put down, which the
    /// runner removes after each item and a killed process does not) that
    /// nothing wrote for `older_than`: a reading going on writes into its
    /// folder every few seconds; and the folders `.tmp/check-<job id>-<item
    /// id>/` (the file a person's check let the server browser download,
    /// which the runner removes once the item settles) that nothing wrote for
    /// `older_than` and whose item no longer has to receive its file (not
    /// pending, running or waiting; the item's next run takes the file
    /// otherwise). Anything else in the area is left. Returns how many
    /// folders it removed.
    pub async fn sweep(&self, older_than: Duration) -> Result<usize, UploadError> {
        let root = self.area.root().to_owned();
        let mut candidates: Vec<PathBuf> = Vec::new();
        let mut stagings: Vec<PathBuf> = Vec::new();
        let mut checks: Vec<(PathBuf, i64)> = Vec::new();
        for dir in [root.join(".tmp"), root.clone()] {
            let mut entries = match tokio::fs::read_dir(&dir).await {
                Ok(entries) => entries,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.into()),
            };
            while let Some(entry) = entries.next_entry().await? {
                let name = entry.file_name().to_string_lossy().into_owned();
                let old = entry
                    .metadata()
                    .await
                    .ok()
                    .filter(|meta| meta.is_dir())
                    .and_then(|meta| meta.modified().ok())
                    .and_then(|at| SystemTime::now().duration_since(at).ok())
                    .is_some_and(|age| age >= older_than);
                if old && uuid::Uuid::try_parse(&name).is_ok_and(|id| id.to_string() == name) {
                    candidates.push(entry.path());
                } else if old && dir.ends_with(".tmp") && is_winpng_staging(&name) {
                    stagings.push(entry.path());
                } else if let Some(item) =
                    check_staging_item(&name).filter(|_| old && dir.ends_with(".tmp"))
                {
                    checks.push((entry.path(), item));
                }
            }
        }
        let names: Vec<String> = candidates
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        let known = self.store.known_receive_ids(names).await?;
        let open = self
            .store
            .open_items(checks.iter().map(|(_, item)| *item).collect())
            .await?;
        stagings.extend(
            checks
                .into_iter()
                .filter(|(_, item)| !open.contains(item))
                .map(|(path, _)| path),
        );
        let mut removed = 0;
        for path in stagings {
            if tokio::fs::remove_dir_all(&path).await.is_ok() {
                removed += 1;
            }
        }
        for path in candidates {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            if name.is_some_and(|n| known.contains(&n)) {
                continue;
            }
            if tokio::fs::remove_dir_all(&path).await.is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Sweeps at once and then every `every`, in a task of its own, until the
    /// handle is aborted. An upload killed with the process leaves its folder
    /// at the next start, but one whose `finish` was cut short in a running
    /// process leaves it too, so the area is swept as long as the process runs.
    /// Each pass keeps the rules of [`Uploads::sweep`]; a pass that removed
    /// folders or failed is reported to `report`.
    pub fn keep_sweeping(
        &self,
        every: Duration,
        older_than: Duration,
        report: impl Fn(Result<usize, UploadError>) + Send + 'static,
    ) -> tokio::task::JoinHandle<()> {
        let uploads = self.clone();
        tokio::spawn(async move {
            let mut ticks = tokio::time::interval(every);
            ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                // The first tick is immediate: the sweep at start.
                ticks.tick().await;
                match uploads.sweep(older_than).await {
                    Ok(0) => {}
                    result => report(result),
                }
            }
        })
    }

    /// Waits for a turn, then makes the folder the upload's files are
    /// written to. The turn lasts until the [`Staging`] is dropped. When
    /// both turns are taken, at most `limits.queue` uploads wait, each for
    /// at most `limits.queue_wait`; the others get [`UploadError::Busy`].
    pub async fn begin(&self) -> Result<Staging, UploadError> {
        let slot = match Arc::clone(&self.slots).try_acquire_owned() {
            Ok(slot) => slot,
            Err(_) => {
                if self.waiting.fetch_add(1, Ordering::SeqCst) >= self.limits.queue {
                    self.waiting.fetch_sub(1, Ordering::SeqCst);
                    return Err(UploadError::Busy);
                }
                let _waiting = Waiting(Arc::clone(&self.waiting));
                tokio::time::timeout(
                    self.limits.queue_wait,
                    Arc::clone(&self.slots).acquire_owned(),
                )
                .await
                .map_err(|_| UploadError::Busy)?
                .expect("the semaphore is never closed")
            }
        };
        let dir = self
            .area
            .at(&ReceiveArea::temp_dir(&uuid::Uuid::new_v4().to_string()));
        tokio::fs::create_dir_all(&dir).await?;
        Ok(Staging {
            _slot: slot,
            dir,
            limits: self.limits,
            files: Vec::new(),
            skipped: Vec::new(),
            open: None,
            total: 0,
        })
    }

    /// Judges the staged files, stores those kept and records the job. The
    /// staged folder is gone when this returns. It runs as a task of its
    /// own: a caller that is dropped midway (a client that went away) does
    /// not stop it between moving the files and recording the job.
    pub async fn finish(
        &self,
        staging: Staging,
        request: UploadRequest,
        now: Millis,
    ) -> Result<Finished, UploadError> {
        let this = self.clone();
        tokio::spawn(async move { this.finish_now(staging, request, now).await })
            .await
            .map_err(io::Error::other)?
    }

    async fn finish_now(
        &self,
        mut staging: Staging,
        request: UploadRequest,
        now: Millis,
    ) -> Result<Finished, UploadError> {
        if staging.open.is_some() {
            staging.end().await?;
        }
        let digest = request_digest(&staging);
        let canonical = serde_json::json!({
            "upload": {
                "work_id": request.work_id,
                "season": request.season,
                "source_id": request.source_id,
                "files": staging.files.len(),
                "skipped": staging.skipped.len(),
                "digest": digest,
            }
        })
        .to_string();

        let job_id = uuid::Uuid::new_v4().to_string();
        let area = self.area.clone();
        let budget = staging.limits.inflate_budget;
        let (kept, dropped, job_dir) = {
            let (judged, skipped, dir) = (
                std::mem::take(&mut staging.files),
                std::mem::take(&mut staging.skipped),
                staging.dir.clone(),
            );
            let job_id = job_id.clone();
            tokio::task::spawn_blocking(move || {
                keep_the_files(&area, &dir, &job_id, judged, skipped, budget)
            })
            .await
            .map_err(io::Error::other)??
        };
        // The staged folder has served: what is kept moved out of it.
        let _ = tokio::fs::remove_dir_all(&staging.dir).await;

        let removed = |job_dir: Option<PathBuf>| async move {
            if let Some(dir) = job_dir {
                let _ = tokio::fs::remove_dir_all(dir).await;
            }
        };
        if kept.is_empty() {
            removed(job_dir).await;
            return Ok(Finished::Nothing { dropped });
        }
        let counts = Counts::of(&kept);
        let made = self
            .store
            .create_upload(
                NewUpload {
                    id: job_id,
                    command_id: request.command_id,
                    request: canonical,
                    work_id: request.work_id,
                    season: request.season,
                    anime_no: request.anime_no,
                    source_id: request.source_id,
                    creator: request.creator,
                    files: kept,
                    dropped: dropped.clone(),
                },
                now,
            )
            .await;
        match made {
            Ok(Created::Created(id)) => Ok(Finished::Created {
                job_id: id,
                counts,
                dropped,
            }),
            Ok(Created::Existing(id)) => {
                removed(job_dir).await;
                Ok(Finished::Existing(id))
            }
            Ok(Created::Mismatch(id)) => {
                removed(job_dir).await;
                Ok(Finished::Mismatch(id))
            }
            Err(e) => {
                removed(job_dir).await;
                Err(e.into())
            }
        }
    }
}

/// What an upload asks for, besides its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadRequest {
    /// The ID the browser made for the action.
    pub command_id: String,
    pub work_id: String,
    pub season: i64,
    /// The season's Anissia anime, when it is linked.
    pub anime_no: Option<i64>,
    /// The creator the person chose among the season's, or `None` for
    /// `제작자 알 수 없음`, with the creator's name.
    pub source_id: Option<String>,
    pub creator: Option<String>,
}

/// What [`Uploads::finish`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Finished {
    Created {
        job_id: String,
        counts: Counts,
        dropped: Vec<Dropped>,
    },
    /// The command ID had made this job with the same upload before; the
    /// files of this one were removed.
    Existing(String),
    /// The command ID made another upload before.
    Mismatch(String),
    /// No file was a subtitle, a font or a ZIP: no job was made.
    Nothing { dropped: Vec<Dropped> },
}

/// How many files of each kind an upload kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    pub subtitles: usize,
    pub fonts: usize,
    pub archives: usize,
}

impl Counts {
    fn of(files: &[UploadedFile]) -> Counts {
        let mut counts = Counts::default();
        for file in files {
            match file.kind {
                Kind::Subtitle => counts.subtitles += 1,
                Kind::Font => counts.fonts += 1,
                Kind::Archive => counts.archives += 1,
            }
        }
        counts
    }

    /// `자막 2개 · 폰트 1개`: the kinds that are there.
    pub fn sentence(self) -> String {
        [
            (self.subtitles, Kind::Subtitle),
            (self.fonts, Kind::Font),
            (self.archives, Kind::Archive),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, kind)| format!("{} {n}개", kind.label()))
        .collect::<Vec<_>>()
        .join(" · ")
    }
}

/// A file an upload did not keep, with why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
    pub name: String,
    pub reason: String,
}

/// The reason a file left out before it was sent is recorded with.
pub const SKIPPED_REASON: &str = "이름으로 보아 자막이나 폰트가 아니라 올리지 않았어요";

/// The reason an MPEG program stream is dropped with when no VobSub index of
/// the same name came with it: it is a video.
pub const NO_INDEX_REASON: &str =
    "같은 이름의 VobSub 인덱스(.idx)가 없어서 자막이 아니라 영상으로 보고 빼요";

/// Whether a drop is for what the file is (empty, a web page, past the size
/// cap, unreadable, the budget spent) rather than for having no magic: a volume
/// that is one of those is dropped with that reason.
fn has_own_reason(verdict: &Verdict) -> bool {
    matches!(verdict, Verdict::Drop(reason) if reason != NOT_THEM)
}

/// The last extension of a shown name, without the dot.
fn extension_of(name: &str) -> Option<&str> {
    let base = name.rsplit('/').next().unwrap_or(name);
    base.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, ext)| ext)
}

/// A shown name without its last extension, lowercase: what a `.sub` and its
/// `.idx` share.
fn stem_key(name: &str) -> String {
    let cut = name.len() - extension_of(name).map_or(0, |ext| ext.len() + 1);
    name[..cut].to_lowercase()
}

struct Staged {
    /// The name the browser gave the file (a folder's relative path), not yet
    /// made safe.
    name: String,
    /// The staged file, within the staging folder.
    temp: String,
    size: u64,
    sha256: String,
}

struct Open {
    name: String,
    temp: String,
    file: tokio::fs::File,
    size: u64,
    hasher: Sha256,
}

/// The folder an upload's files are written to, and its turn. Dropping it
/// removes the folder.
pub struct Staging {
    _slot: OwnedSemaphorePermit,
    dir: PathBuf,
    limits: Limits,
    files: Vec<Staged>,
    skipped: Vec<String>,
    open: Option<Open>,
    total: u64,
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Staging {
    /// How many files were written (the one being written not counted).
    pub fn files(&self) -> usize {
        self.files.len()
    }

    /// Notes a file the browser left out by its name, so the job shows it.
    pub fn skip(&mut self, name: &str) -> Result<(), UploadError> {
        if self.files.len() + self.skipped.len() + usize::from(self.open.is_some())
            >= self.limits.entries
        {
            return Err(UploadError::TooManyEntries(self.limits.entries));
        }
        self.skipped.push(name.to_owned());
        Ok(())
    }

    /// Starts the next file under `name`. The one before is ended first.
    pub async fn start(&mut self, name: &str) -> Result<(), UploadError> {
        if self.open.is_some() {
            self.end().await?;
        }
        if self.files.len() >= self.limits.files {
            return Err(UploadError::TooManyFiles(self.limits.files));
        }
        if self.files.len() + self.skipped.len() >= self.limits.entries {
            return Err(UploadError::TooManyEntries(self.limits.entries));
        }
        let temp = format!("{}.part", self.files.len());
        let file = tokio::fs::File::create(self.dir.join(&temp)).await?;
        self.open = Some(Open {
            name: name.to_owned(),
            temp,
            file,
            size: 0,
            hasher: Sha256::new(),
        });
        Ok(())
    }

    /// Writes the next bytes of the file being written, if they keep it and
    /// the upload within the limits.
    pub async fn write(&mut self, bytes: &[u8]) -> Result<(), UploadError> {
        let open = self.open.as_mut().ok_or(UploadError::NoFile)?;
        let size = open.size + bytes.len() as u64;
        if size > self.limits.file_bytes {
            return Err(UploadError::FileTooLarge(self.limits.file_bytes));
        }
        if self.total + bytes.len() as u64 > self.limits.total_bytes {
            return Err(UploadError::TotalTooLarge(self.limits.total_bytes));
        }
        open.file.write_all(bytes).await?;
        open.hasher.update(bytes);
        open.size = size;
        self.total += bytes.len() as u64;
        Ok(())
    }

    /// Ends the file being written: its bytes are on the disk.
    pub async fn end(&mut self) -> Result<(), UploadError> {
        let Open {
            name,
            temp,
            mut file,
            size,
            hasher,
        } = self.open.take().ok_or(UploadError::NoFile)?;
        file.flush().await?;
        file.sync_all().await?;
        self.files.push(Staged {
            name,
            temp,
            size,
            sha256: area::hex(&hasher.finalize()),
        });
        Ok(())
    }
}

/// Everything about the files, in order, that makes one upload the same as a
/// repeat of it.
fn request_digest(staging: &Staging) -> String {
    let mut hasher = Sha256::new();
    for file in &staging.files {
        hasher.update(file.name.as_bytes());
        hasher.update([0]);
        hasher.update(file.size.to_be_bytes());
        hasher.update(file.sha256.as_bytes());
        hasher.update([0]);
    }
    hasher.update([1]);
    for name in &staging.skipped {
        hasher.update(name.as_bytes());
        hasher.update([0]);
    }
    area::hex(&hasher.finalize())
}

/// A name as shown and kept: the parts of the relative path made safe names,
/// the last [`MAX_PARTS`] of them, joined by `/`.
pub fn display_name(raw: &str) -> String {
    let parts: Vec<String> = raw
        .split(['/', '\\'])
        .filter(|part| !part.is_empty() && *part != "." && *part != "..")
        .map(area::safe_name)
        .collect();
    let first = parts.len().saturating_sub(MAX_PARTS);
    match parts[first..].is_empty() {
        true => area::safe_name(""),
        false => parts[first..].join("/"),
    }
}

/// The first of `name`'s candidates ([`area::name_candidates`]) not in `taken`,
/// which then holds it.
fn first_free(name: &str, taken: &mut HashSet<String>) -> String {
    area::name_candidates(name)
        .find(|candidate| taken.insert(candidate.clone()))
        .expect("the names go on")
}

/// What the files of one package tell about each other, which the bytes of
/// one file alone cannot: the later volumes of a split archive waiting on a
/// name, the first volumes kept, and the VobSub indexes kept. An upload sees
/// all its files at once ([`keep_the_files`]); a find job's browser run
/// brings them one at a time, each judged against those before it
/// ([`sort_arrival`]).
#[derive(Debug, Default)]
struct Siblings {
    /// The lowercased names later volumes are vouched for by
    /// ([`volume_anchor`]).
    wanted: HashSet<String>,
    /// The archives kept, by lowercased name.
    first_volumes: HashMap<String, Archive>,
    /// The stems ([`stem_key`]) of the VobSub indexes kept.
    indexes: HashSet<String>,
}

impl Siblings {
    /// `name` is one of the package's files.
    fn want(&mut self, name: &str) {
        if let Some(anchor) = volume_anchor(name) {
            self.wanted.insert(anchor);
        }
    }

    /// A ZIP that cannot be read alone but starts with `PK` and has volumes
    /// waiting on it (`pack.z01` for `pack.zip`, `pack.zip.002` for
    /// `pack.zip.001`) is the set's spanned ZIP: it is kept without the
    /// structural check a whole ZIP gets.
    fn spanned(&self, name: &str, path: &Path, verdict: &mut Verdict) {
        if matches!(verdict, Verdict::BadZip(_))
            && self.wanted.contains(&name.to_lowercase())
            && is_zip_signed(path)
        {
            *verdict = Verdict::Keep(Kept {
                kind: Kind::Archive,
                format: trss_subtitles::verify::Format::Other,
                archive: Some(Archive::Zip),
            });
        }
    }

    /// Notes a file judged `verdict` (after [`Siblings::spanned`]): a kept
    /// archive vouches for its set's later volumes, a kept VobSub index for
    /// its program stream.
    fn note(&mut self, name: &str, path: &Path, verdict: &Verdict) {
        let Verdict::Keep(kept) = verdict else {
            return;
        };
        if let (Kind::Archive, Some(archive)) = (kept.kind, kept.archive) {
            self.first_volumes.insert(name.to_lowercase(), archive);
        }
        if extension_of(name).is_some_and(|ext| ext.eq_ignore_ascii_case("idx"))
            && is_vobsub_index(path)
        {
            self.indexes.insert(stem_key(name));
        }
    }

    /// What becomes of the file `name` judged `verdict`: kept as what, or
    /// dropped for why.
    fn resolve(&self, name: &str, verdict: Verdict) -> Result<Kept, String> {
        match verdict {
            Verdict::Keep(kept) => Ok(kept),
            Verdict::Stream if self.indexes.contains(&stem_key(name)) => Ok(Kept {
                kind: Kind::Subtitle,
                format: trss_subtitles::verify::Format::Other,
                archive: None,
            }),
            Verdict::Stream => Err(NO_INDEX_REASON.to_owned()),
            // A later volume of a split archive is the set's, whatever its bytes are.
            Verdict::Drop(_) | Verdict::BadZip(_)
                if volume_anchor(name).is_some() && !has_own_reason(&verdict) =>
            {
                match volume_anchor(name).and_then(|anchor| self.first_volumes.get(&anchor)) {
                    Some(archive) => Ok(Kept {
                        kind: Kind::Archive,
                        format: trss_subtitles::verify::Format::Other,
                        archive: Some(*archive),
                    }),
                    None => Err(NO_FIRST_VOLUME.to_owned()),
                }
            }
            Verdict::BadZip(reason) => Err(reason),
            // A file named as an archive that is none says so, not that it is no subtitle.
            Verdict::Drop(reason) if reason == NOT_THEM && is_archive_name(name) => {
                Err(NOT_AN_ARCHIVE.to_owned())
            }
            Verdict::Drop(reason) => Err(reason),
        }
    }
}

/// A file a find job's browser run downloaded, judged
/// ([`sort_arrival`]): kept in the job's folder, or dropped (its bytes gone).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arrived {
    Kept(UploadedFile),
    Dropped(Dropped),
}

/// What a find job kept before a file arrives: what [`sort_arrival`] judges
/// it against and names it apart from. A file the job dropped is no part of
/// its package (its bytes are gone), so it vouches for nothing.
#[derive(Debug, Clone, Default)]
pub struct Earlier {
    /// The files kept: shown name, path relative to the receive area, kind
    /// and archive format.
    pub kept: Vec<(String, String, Kind, Option<Archive>)>,
}

impl Earlier {
    /// What the job's records `found` say it kept.
    pub fn of(found: &crate::store::Found) -> Earlier {
        Earlier {
            kept: found
                .kept
                .iter()
                .filter_map(|f| Some((f.name.clone(), f.path.clone()?, f.kind?, f.archive)))
                .collect(),
        }
    }
}

/// Judges a file a find job's browser run downloaded to `staged` (in a
/// folder of the receive area), named `raw` by the site, as an upload's file
/// is judged ([`judge`] and the rules between files, [`Siblings`]) against
/// the files the job kept `earlier`. A kept file is moved into the job's
/// folder under a free safe name, `n` making its receipt's ID; a dropped one
/// is removed. ZIPs inflate within `budget`. The rules between files see
/// only the files kept before: a VobSub stream before its index, or a later
/// volume of a split archive before its first, is dropped, and is kept if it
/// is downloaded again after. A ZIP that cannot be read alone is kept, as an
/// upload keeps it, only with a later volume of its set among the files, so
/// a ZIP split into volumes (`.z01`, `.zip.001`) is never kept: each of its
/// parts comes alone.
pub fn sort_arrival(
    area: &ReceiveArea,
    job_id: &str,
    staged: &Path,
    raw: &str,
    earlier: &Earlier,
    n: usize,
    budget: &mut u64,
) -> Result<Arrived, UploadError> {
    // Named apart from the files kept only: a dropped file holds no name, so
    // a stream downloaded again after its index keeps its stem.
    let mut shown: HashSet<String> = earlier.kept.iter().map(|k| k.0.clone()).collect();
    let name = first_free(&display_name(raw), &mut shown);
    let mut verdict = judge(staged, budget);
    let mut siblings = Siblings::default();
    for (kept_name, ..) in &earlier.kept {
        siblings.want(kept_name);
    }
    siblings.want(&name);
    siblings.spanned(&name, staged, &mut verdict);
    for (kept_name, path, kind, archive) in &earlier.kept {
        let verdict = Verdict::Keep(Kept {
            kind: *kind,
            format: trss_subtitles::verify::Format::Other,
            archive: *archive,
        });
        siblings.note(kept_name, &area.at(path), &verdict);
    }
    let kept = match siblings.resolve(&name, verdict) {
        Ok(kept) => kept,
        Err(reason) => {
            std::fs::remove_file(staged)?;
            return Ok(Arrived::Dropped(Dropped { name, reason }));
        }
    };

    let (size, sha256, _) = area::read_facts(staged)?;
    let rel_dir = ReceiveArea::job_dir(job_id);
    let job_dir = area.at(&rel_dir);
    std::fs::create_dir_all(&job_dir)?;
    // The names in the folder now: a file a crash left there unrecorded keeps
    // its name.
    let mut used: HashSet<String> = std::fs::read_dir(&job_dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    let last = name.rsplit('/').next().unwrap_or(&name);
    let flat = first_free(last, &mut used);
    let target = job_dir.join(&flat);
    trss_core::files::rename_noreplace(staged, &target)?;
    trss_core::files::sync_dir(&job_dir)?;
    if let Some(folder) = job_dir.parent() {
        trss_core::files::sync_dir(folder)?;
    }
    let meta = std::fs::symlink_metadata(&target)?;
    Ok(Arrived::Kept(UploadedFile {
        id: format!("{job_id}.{n:04}"),
        file_key: name.clone(),
        name,
        path: format!("{rel_dir}/{flat}"),
        size,
        sha256,
        object: area::object_of(&meta),
        format: kept.format,
        kind: kept.kind,
        archive: kept.archive,
    }))
}

/// What judging an upload's files came to: the kept files, the dropped ones,
/// and the job's folder when one was made.
type Placed = (Vec<UploadedFile>, Vec<Dropped>, Option<PathBuf>);

/// Judges the staged files and moves the kept ones into the job's folder,
/// under free safe names. Returns the kept files, the dropped ones (those left
/// out by name first), and the job's folder when one was made.
fn keep_the_files(
    area: &ReceiveArea,
    staged_dir: &Path,
    job_id: &str,
    files: Vec<Staged>,
    skipped: Vec<String>,
    mut budget: u64,
) -> Result<Placed, UploadError> {
    let mut shown: HashSet<String> = HashSet::new();
    let mut unique_shown = |raw: &str| -> String { first_free(&display_name(raw), &mut shown) };

    let mut dropped: Vec<Dropped> = skipped
        .iter()
        .map(|raw| Dropped {
            name: unique_shown(raw),
            reason: SKIPPED_REASON.to_owned(),
        })
        .collect();
    // Every file is judged first: a program stream is VobSub's only if its
    // index is among them, and a later volume of a split archive (which has
    // no magic) is kept only with the volume that vouches for its set.
    let mut judged: Vec<(Staged, String, Verdict)> = files
        .into_iter()
        .map(|file| {
            let name = unique_shown(&file.name);
            let verdict = judge(&staged_dir.join(&file.temp), &mut budget);
            (file, name, verdict)
        })
        .collect();
    let mut siblings = Siblings::default();
    for (_, name, _) in &judged {
        siblings.want(name);
    }
    for (file, name, verdict) in &mut judged {
        siblings.spanned(name, &staged_dir.join(&file.temp), verdict);
    }
    for (file, name, verdict) in &judged {
        siblings.note(name, &staged_dir.join(&file.temp), verdict);
    }
    let mut keepers: Vec<(Staged, String, Kept)> = Vec::new();
    for (file, name, verdict) in judged {
        match siblings.resolve(&name, verdict) {
            Ok(kept) => keepers.push((file, name, kept)),
            Err(reason) => dropped.push(Dropped { name, reason }),
        }
    }
    if keepers.is_empty() {
        return Ok((Vec::new(), dropped, None));
    }

    let rel_dir = ReceiveArea::job_dir(job_id);
    let job_dir = area.at(&rel_dir);
    let folder = job_dir.parent().expect("a folder in the area").to_owned();
    std::fs::create_dir_all(&folder)?;
    std::fs::create_dir(&job_dir)?;
    let moved = (|| -> Result<Vec<UploadedFile>, UploadError> {
        let mut used: HashSet<String> = HashSet::new();
        let mut out = Vec::with_capacity(keepers.len());
        for (n, (file, name, kept)) in keepers.into_iter().enumerate() {
            // The flat name is the last part of the shown one.
            let last = name.rsplit('/').next().unwrap_or(&name);
            let flat = first_free(last, &mut used);
            let target = job_dir.join(&flat);
            trss_core::files::rename_noreplace(&staged_dir.join(&file.temp), &target)?;
            let meta = std::fs::symlink_metadata(&target)?;
            out.push(UploadedFile {
                // Sorts in the order the files came: the job lists them so.
                id: format!("{job_id}.{n:04}"),
                file_key: name.clone(),
                name,
                path: format!("{rel_dir}/{flat}"),
                size: file.size,
                sha256: file.sha256,
                object: area::object_of(&meta),
                format: kept.format,
                kind: kept.kind,
                archive: kept.archive,
            });
        }
        trss_core::files::sync_dir(&job_dir)?;
        trss_core::files::sync_dir(&folder)?;
        Ok(out)
    })();
    match moved {
        Ok(kept) => Ok((kept, dropped, Some(job_dir))),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&job_dir);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_cut_into_safe_parts_that_stay_in_the_folder() {
        assert_eq!(display_name("01.ass"), "01.ass");
        assert_eq!(display_name("Show/Season 1/01.ass"), "Show/Season 1/01.ass");
        assert_eq!(display_name("../../etc/passwd"), "etc/passwd");
        assert_eq!(display_name("/abs/./path//x.srt"), "abs/path/x.srt");
        assert_eq!(display_name("C:\\subs\\..\\01.ass"), "C:/subs/01.ass");
        assert_eq!(display_name(".hidden/.x.ass"), "hidden/x.ass");
        assert_eq!(display_name("13화\u{202E}exe.ass"), "13화exe.ass");
        assert_eq!(display_name(""), "file");
        assert_eq!(display_name("../.."), "file");
        let deep = (1..=12)
            .map(|n| format!("d{n}"))
            .collect::<Vec<_>>()
            .join("/");
        assert_eq!(
            display_name(&format!("{deep}/f.ass")),
            "d6/d7/d8/d9/d10/d11/d12/f.ass"
        );
    }

    #[test]
    fn counts_are_said_by_kind() {
        let counts = Counts {
            subtitles: 2,
            fonts: 1,
            archives: 0,
        };
        assert_eq!(counts.sentence(), "자막 2개 · 폰트 1개");
        assert_eq!(
            Counts {
                subtitles: 0,
                fonts: 0,
                archives: 1
            }
            .sentence(),
            "압축 파일 1개"
        );
    }

    fn program_stream() -> Vec<u8> {
        let mut bytes = b"\x00\x00\x01\xBA\x44\x00\x04\x00\x04\x01".to_vec();
        bytes.resize(64, 0);
        bytes
    }

    const IDX: &[u8] = b"# VobSub index file, v7 (do not modify this line!)\nlangidx: 0\n";

    /// Sorts `bytes` arriving as `name` after `earlier`, as the `n`th file.
    fn arrive(
        area: &ReceiveArea,
        name: &str,
        bytes: &[u8],
        earlier: &mut Earlier,
        n: usize,
    ) -> Arrived {
        let staged = area.at(".tmp/check-j-1");
        std::fs::create_dir_all(&staged).unwrap();
        let staged = staged.join(name);
        std::fs::write(&staged, bytes).unwrap();
        let mut budget = INFLATE_BUDGET;
        let arrived = sort_arrival(area, "j", &staged, name, earlier, n, &mut budget).unwrap();
        assert!(!staged.exists(), "the staged bytes are moved or removed");
        match &arrived {
            Arrived::Kept(file) => {
                earlier.kept.push((
                    file.name.clone(),
                    file.path.clone(),
                    file.kind,
                    file.archive,
                ));
            }
            Arrived::Dropped(_) => {}
        }
        arrived
    }

    #[test]
    fn an_arrival_is_kept_in_the_jobs_folder_under_a_name_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let area = ReceiveArea::in_app_data(dir.path());
        let mut earlier = Earlier::default();
        let srt = b"1\n00:00:01,000 --> 00:00:02,000\nhi\n\n";
        let Arrived::Kept(first) = arrive(&area, "ep1.srt", srt, &mut earlier, 0) else {
            panic!("kept")
        };
        assert_eq!(
            (first.id.as_str(), first.path.as_str()),
            ("j.0000", "j/ep1.srt")
        );
        assert_eq!(first.kind, Kind::Subtitle);
        // The same name again is a second file, named apart.
        let Arrived::Kept(second) = arrive(&area, "ep1.srt", srt, &mut earlier, 1) else {
            panic!("kept")
        };
        assert_eq!(second.id, "j.0001");
        assert_ne!(second.name, first.name);
        assert_ne!(second.path, first.path);
        assert!(area.at(&first.path).exists() && area.at(&second.path).exists());

        let Arrived::Dropped(dropped) = arrive(&area, "readme.txt", b"hello", &mut earlier, 2)
        else {
            panic!("dropped")
        };
        assert_eq!(dropped.reason, NOT_THEM);
    }

    #[test]
    fn the_rules_between_files_see_the_files_that_came_before() {
        let dir = tempfile::tempdir().unwrap();
        let area = ReceiveArea::in_app_data(dir.path());
        let mut earlier = Earlier::default();
        // A program stream before its index is dropped; after it, kept.
        let Arrived::Dropped(early) =
            arrive(&area, "movie.sub", &program_stream(), &mut earlier, 0)
        else {
            panic!("dropped")
        };
        assert_eq!(early.reason, NO_INDEX_REASON);
        assert!(matches!(
            arrive(&area, "movie.idx", IDX, &mut earlier, 0),
            Arrived::Kept(_)
        ));
        let Arrived::Kept(stream) = arrive(&area, "movie.sub", &program_stream(), &mut earlier, 1)
        else {
            panic!("kept")
        };
        assert_eq!(stream.kind, Kind::Subtitle);
        // A later volume with no first volume before it is dropped.
        let Arrived::Dropped(volume) = arrive(&area, "pack.z01", b"\x00\x01", &mut earlier, 2)
        else {
            panic!("dropped")
        };
        assert_eq!(volume.reason, NO_FIRST_VOLUME);
        // Nor does the dropped volume vouch for a ZIP after it: that ZIP,
        // unreadable alone, would be a set with a part missing.
        let Arrived::Dropped(zip) = arrive(&area, "pack.zip", b"PK\x03\x04broken", &mut earlier, 2)
        else {
            panic!("dropped")
        };
        assert_eq!(zip.name, "pack.zip");
        assert!(earlier.kept.iter().all(|k| k.0 != "pack.zip"));
    }
}
