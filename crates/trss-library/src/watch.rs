//! The worker's reading of the watch folders (`docs/specs/library.md`, 작품
//! 발견과 감시 폴더).
//!
//! Every cycle, after the RSS work, [`scan_all`] reads
//! the registered watch folders that need it with [`crate::discovery::scan`]
//! and records what it found ([`crate::store::library`]); which ones need it is
//! decided by [`crate::live`]: with the kernel's inotify alerts doing the
//! reading of changes as they happen, a cycle reads a folder only when the
//! worker has just started, when alerts could not be relied on, and for the
//! hourly safety net (when the worker is not watching at all, every folder every
//! cycle, as it always did). The same alerts read single works with
//! [`scan_works`]. The `watch_rescan` command
//! ([`crate::watch_rescan`]) reads one folder the same way when the
//! user asks for `다시 확인`. The web reads a folder only when it is added, with
//! the same scan.
//!
//! A folder that cannot be read records its reason on the folder and keeps what
//! was known; it does not stop the other folders, the cycle, or the RSS work.
//! A scan only reads: it creates, moves and deletes no file.
//!
//! # Turns
//!
//! Every reading of a watch folder, the cycle's, an alert's or `다시 확인`'s,
//! takes its turn at the folder first ([`reading_section`], with
//! [`WatchContext::folders`]): one reading of a folder at a time, so a reading
//! that ended later never records an older picture over a newer one, and none
//! while a command moves or renames what it reads (a work folder going to the
//! archive folder, an undo's renames). The readings of other folders
//! go on beside it. The cycle's and an alert's readings do not wait in line:
//! when the turn is taken they are left for later (the cycle reads the folder
//! at its next run), since a reading waiting behind a long move would hold
//! back every command after it in the folder. `다시 확인` is a command and
//! waits for its turn in the order it was accepted.
//!
//! The duration of every scan is logged, since a large library (hundreds of
//! works, thousands of files) is read in the cycle's own time.
//!
//! # Directories that have not changed
//!
//! A periodic scan lists only the directories whose modification time or
//! identity changed since the worker last read them
//! ([`discovery::scan_incremental`]); the rest of the tree costs one `stat` per
//! directory. What the worker remembers lives in memory, per watch folder
//! ([`ScanCaches`]), so the first scan after the worker starts, `다시 확인`
//! ([`ScanMode::Full`]) and a folder that could not be read are full reads. A
//! change that leaves a directory's modification time as it was (a file system
//! with coarse times) is found by `다시 확인` or after a restart. The time only
//! decides what to list again: it never becomes an added time.
//!
//! # The collect and archive folders
//!
//! The collect folder and the archive folder are always watch folders. The web
//! registers them when the settings are saved; [`sync_automatic`] does the same
//! at the start of every cycle for a database whose settings were saved before
//! that (a folder it registers is read, as the first reading, by the same
//! cycle). It changes nothing when the watch folders already match.
//!
//! # A folder that hangs
//!
//! Every scan runs on a blocking thread, and the cycle waits for it under the
//! worker's lock. A mount that stops answering (NFS, SMB) would hold the lock for good,
//! so a scan gets [`SCAN_TIMEOUT`]; past it the folder is recorded as unreadable
//! and the cycle goes on. The thread cannot be stopped, but a scan only reads, so
//! leaving it to finish (or hang) harms nothing. While it still runs, the folder
//! is not scanned again (the next attempts fail at once with a sentence saying
//! so), so a hung mount costs one thread, not one per cycle.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};

use tokio_util::sync::CancellationToken;

use trss_core::{
    folder_locks::{FolderLocks, Section},
    settings::SettingsStore,
    Clock,
};

use crate::{
    automatic_watch::{self, Wanted},
    discovery,
    live::{LiveWatch, Poll},
    store::library::{Followed, LibraryError, LibraryStore, ScanReport, WatchFolder},
};

/// What the reading of the watch folders needs, in the cycle, in a command and
/// in the folders' watches. Cheap to clone.
#[derive(Clone)]
pub struct WatchContext {
    pub library: LibraryStore,
    /// Where the collect and archive folders are read from.
    pub settings: SettingsStore,
    /// The worker's turns at its folders (see the module docs), shared with
    /// the collection work.
    pub folders: FolderLocks,
    /// What the worker remembers of each watch folder's directories between
    /// scans.
    pub scan_cache: ScanCaches,
    /// The inotify watches of the watch folders, which say what the cycle has
    /// to read of them (see [`crate::live`]).
    pub live: LiveWatch,
}

/// What the worker remembers of each watch folder's directories (by folder
/// ID) to skip the unchanged ones the next time. Cheap to clone.
pub type ScanCaches = Arc<Mutex<HashMap<String, discovery::DirCache>>>;

/// How much of a watch folder a scan reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanMode {
    /// The periodic scan: directories that have not changed are skipped.
    Periodic,
    /// Every directory is listed (`다시 확인`).
    Full,
}

/// How [`scan_works`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorksMode {
    /// Every directory of the works is listed (what an alert asks for).
    Fresh,
    /// Directories that have not changed are skipped, as in the periodic scan.
    Incremental,
}

/// How long reading one watch folder may take before it is given up on.
pub const SCAN_TIMEOUT: Duration = Duration::from_secs(60);

/// The folders (by path) whose scan thread has not returned yet.
static SCANNING: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Mutex::default);

/// Takes `path` out of [`SCANNING`] when the scan thread returns.
struct Scanning(String);

impl Drop for Scanning {
    fn drop(&mut self) {
        SCANNING
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.0);
    }
}

/// Runs `read` for the folder at `path` on a blocking thread and waits at most
/// `timeout`. A panic, a timeout, and a scan of the folder that is still running
/// from an earlier timeout are each a [`discovery::ScanError`] with a sentence.
async fn read_with_timeout<T, F>(
    path: &str,
    timeout: Duration,
    read: F,
) -> Result<T, discovery::ScanError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let claimed = SCANNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(path.to_owned());
    if !claimed {
        return Err(discovery::ScanError {
            message: "이전 확인이 아직 끝나지 않았어요. 마운트가 응답하지 않는 것 같으니 연결을 확인해 주세요."
                .to_owned(),
            detail: "an earlier scan of the folder has not returned".to_owned(),
        });
    }
    let guard = Scanning(path.to_owned());
    let task = tokio::task::spawn_blocking(move || {
        let _guard = guard;
        read()
    });
    match tokio::time::timeout(timeout, task).await {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(err)) => {
            // The scan panicked: nothing is known about the folder.
            eprintln!("Watch folder {path}: the scan ended with {err}");
            Err(discovery::ScanError {
                message: "폴더를 읽다가 내부 오류가 났어요.".to_owned(),
                detail: err.to_string(),
            })
        }
        Err(_) => Err(discovery::ScanError {
            message: format!(
                "폴더를 읽는 데 {}초가 넘게 걸려서 멈췄어요. 마운트(NFS·SMB)가 응답하지 않는지 확인해 주세요.",
                timeout.as_secs()
            ),
            detail: format!("timed out after {} s", timeout.as_secs()),
        }),
    }
}

/// What reading one folder came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scanned {
    /// The folder was read (the report may still carry an error about some of
    /// its work folders).
    Read(ScanReport),
    /// The folder could not be read; its error is recorded.
    Failed(String),
    /// The folder was unregistered meanwhile.
    Gone,
}

/// Reads `folder` and records the outcome, logging how long it took.
pub async fn scan_folder(
    ctx: &WatchContext,
    folder: &WatchFolder,
    now: trss_core::Millis,
    mode: ScanMode,
) -> Result<Scanned, LibraryError> {
    let started = Instant::now();
    ctx.live.folder_scan_started(&folder.id);
    let path = folder.path.clone();
    // What the last scan saw goes with the scan; a scan that does not finish
    // leaves nothing behind, so the next one reads everything.
    let previous = {
        let mut caches = ctx.scan_cache.lock().unwrap_or_else(|e| e.into_inner());
        let remembered = caches.remove(&folder.id);
        match mode {
            ScanMode::Periodic => remembered,
            ScanMode::Full => None,
        }
    };
    let scanned = read_with_timeout(&folder.path, SCAN_TIMEOUT, move || {
        discovery::scan_incremental(path.as_ref(), previous)
    })
    .await;
    let (result, stats) = match scanned {
        Ok(scanned) => {
            if scanned.result.is_ok() {
                ctx.scan_cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(folder.id.clone(), scanned.cache);
            }
            (scanned.result, Some(scanned.stats))
        }
        Err(error) => (Err(error), None),
    };
    let read_in = started.elapsed();
    let failure = result.as_ref().err().cloned();
    let recorded = ctx.library.record_scan(&folder.id, result, now).await?;
    let total = started.elapsed();
    let clean = failure.is_none() && recorded.as_ref().is_some_and(|r| r.error.is_none());
    ctx.live.folder_scan_done(&folder.id, now, clean);

    Ok(match (recorded, failure) {
        (None, _) => Scanned::Gone,
        (Some(_), Some(error)) => {
            eprintln!(
                "Watch folder {}: cannot read it: {error} ({} ms)",
                folder.path,
                total.as_millis()
            );
            Scanned::Failed(error.message)
        }
        (Some(report), None) => {
            let listed = stats.map_or(String::new(), |s| {
                format!(
                    "{} directories listed, {} unchanged; ",
                    s.dirs_read, s.dirs_reused
                )
            });
            println!(
                "Watch folder {}: {} works, {} new, {} files added, {} removed, {} missing; \
                 {listed}read in {} ms, scan took {} ms in all",
                folder.path,
                report.works_found,
                report.works_added,
                report.files_added,
                report.files_removed,
                report.works_missing,
                read_in.as_millis(),
                total.as_millis()
            );
            if let Some(error) = &report.error {
                eprintln!("Watch folder {}: {error}", folder.path);
            }
            Scanned::Read(report)
        }
    })
}

/// Reads only the works called `names` of `folder` and records what it found
/// (see [`LibraryStore::record_works`](crate::store::library::LibraryStore::record_works)),
/// logging how long it took. A work whose folder is gone is marked missing; a
/// folder that cannot be read at all records its error as a whole scan does.
pub async fn scan_works(
    ctx: &WatchContext,
    folder: &WatchFolder,
    names: Vec<String>,
    now: trss_core::Millis,
    mode: WorksMode,
) -> Result<Scanned, LibraryError> {
    let started = Instant::now();
    ctx.live.works_scan_started(&folder.id, &names);
    let previous = match mode {
        WorksMode::Fresh => None,
        WorksMode::Incremental => ctx
            .scan_cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&folder.id)
            .cloned(),
    };
    let (path, wanted) = (folder.path.clone(), names.clone());
    let scanned = read_with_timeout(&folder.path, SCAN_TIMEOUT, move || {
        discovery::scan_works(path.as_ref(), &wanted, previous)
    })
    .await;
    let result = match scanned {
        Ok(scanned) => {
            if scanned.result.is_ok() {
                ctx.scan_cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .entry(folder.id.clone())
                    .or_default()
                    .merge(scanned.cache);
            }
            scanned.result
        }
        Err(error) => Err(error),
    };
    let read_in = started.elapsed();
    let recorded = match result {
        Ok(scan) => {
            ctx.library
                .record_works(&folder.id, names.clone(), scan, now)
                .await?
        }
        Err(error) => {
            eprintln!(
                "Watch folder {}: cannot read it: {error} ({} ms)",
                folder.path,
                read_in.as_millis()
            );
            let message = error.message.clone();
            let recorded = ctx.library.record_scan(&folder.id, Err(error), now).await?;
            ctx.live.works_scan_done(&folder.id, false);
            return Ok(match recorded {
                Some(_) => Scanned::Failed(message),
                None => Scanned::Gone,
            });
        }
    };
    let Some(report) = recorded else {
        return Ok(Scanned::Gone);
    };
    ctx.live.works_scan_done(&folder.id, report.error.is_none());
    println!(
        "Watch folder {}: {} works read ({}): {} found, {} new, {} files added, {} removed, \
         {} missing; read in {} ms, took {} ms in all",
        folder.path,
        names.len(),
        names.join(", "),
        report.works_found,
        report.works_added,
        report.files_added,
        report.files_removed,
        report.works_missing,
        read_in.as_millis(),
        started.elapsed().as_millis()
    );
    if let Some(error) = &report.error {
        eprintln!("Watch folder {}: {error}", folder.path);
    }
    Ok(Scanned::Read(report))
}

/// Makes the automatic watch folders the collect and archive folders of the
/// settings (see the module docs). Failures are logged: the folders already
/// registered are read all the same.
pub async fn sync_automatic(ctx: &WatchContext, now: trss_core::Millis) {
    let settings = match ctx.settings.collection().await {
        Ok(Some(settings)) => settings,
        Ok(None) => return,
        Err(err) => {
            eprintln!("Watch folders: cannot read the collection settings: {err}");
            return;
        }
    };
    let registered = match ctx.library.folders().await {
        Ok(folders) => folders,
        Err(err) => {
            eprintln!("Watch folders: cannot list them: {err}");
            return;
        }
    };
    let mut wanted = vec![Wanted {
        what: "수집 폴더",
        path: settings.folder.clone(),
    }];
    if let Some(archive) = &settings.archive_folder {
        wanted.push(Wanted {
            what: "보관 폴더",
            path: archive.clone(),
        });
    }
    let plan = match tokio::task::spawn_blocking(move || {
        automatic_watch::plan(&wanted, &registered)
    })
    .await
    {
        Ok(Ok(plan)) => plan,
        Ok(Err(reason)) => {
            eprintln!("Watch folders: the collect and archive folders cannot be watched: {reason}");
            return;
        }
        Err(err) => {
            eprintln!("Watch folders: planning the collect and archive folders failed: {err}");
            return;
        }
    };
    if plan.is_empty() {
        return;
    }
    match ctx
        .library
        .sync_automatic(plan, settings.version, now)
        .await
    {
        Ok(applied) => println!(
            "Watch folders: the collect and archive folders: {} registered, {} turned automatic, \
             {} removed",
            applied.added, applied.converted, applied.removed
        ),
        // The settings or the folders changed meanwhile; the next cycle plans again.
        Err(LibraryError::Changed) => {}
        Err(err) => {
            eprintln!("Watch folders: cannot register the collect and archive folders: {err}")
        }
    }
}

/// The turn a reading of the watch folder at `folder` takes: the reading's
/// own, and a read of `works` (the work folders it reads) or, with none, of
/// the whole folder.
pub fn reading_section(folder: &str, works: &[String]) -> Section {
    let section = Section::new().reading(folder);
    if works.is_empty() {
        return section.read(folder);
    }
    works.iter().fold(section, |section, work| {
        section.read(Path::new(folder).join(work))
    })
}

/// Reads the watch folders that need it, in turn: all of them when the worker
/// is not watching (see the module docs), otherwise what
/// [`super::live::LiveWatch::poll_for`] says. A folder that fails, or a
/// database error on one, is logged and the next folder is read.
pub async fn scan_all(ctx: &WatchContext, clock: &Clock, cancel: &CancellationToken) {
    sync_automatic(ctx, clock()).await;
    let folders = match ctx.library.folders().await {
        Ok(folders) => folders,
        Err(err) => {
            eprintln!("Watch folders: cannot list them: {err}");
            return;
        }
    };
    // Nothing is remembered of a folder that is not registered any more.
    ctx.scan_cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|id, _| folders.iter().any(|f| &f.id == id));
    // A folder registered since the last look is watched from now on.
    ctx.live.sync_folders().await;
    for folder in folders {
        if cancel.is_cancelled() {
            break;
        }
        let now = clock();
        let poll = ctx.live.poll_for(&folder.id, now);
        let listed = match &poll {
            Poll::Nothing => continue,
            Poll::Works(names) => &names[..],
            Poll::Folder => &[][..],
        };
        // Not waited for: a reading in line behind a move (which waits minutes
        // for Transmission) would hold back every command after it in the
        // folder. The folder stays due, so the next cycle reads it.
        let Some(_turn) = ctx.folders.try_lock(reading_section(&folder.path, listed)) else {
            println!(
                "Watch folder {}: a command is moving or renaming in it; \
                 reading it at the next cycle",
                folder.path
            );
            continue;
        };
        let read = match poll {
            Poll::Nothing => continue,
            // What the watches could not place is tried again each time it is read.
            Poll::Works(names) => {
                for name in &names {
                    ctx.live.resync(&folder.id, Some(name));
                }
                scan_works(ctx, &folder, names, clock(), WorksMode::Incremental)
                    .await
                    .map(|_| ())
            }
            Poll::Folder => {
                if ctx.live.wants_resync(&folder.id) {
                    ctx.live.resync(&folder.id, None);
                }
                scan_folder(ctx, &folder, clock(), ScanMode::Periodic)
                    .await
                    .map(|_| ())
            }
        };
        if let Err(err) = read {
            eprintln!(
                "Watch folder {}: cannot record the scan: {err}",
                folder.path
            );
        }
    }
}

/// The watch folder at `root`: the one registered with that text, or else the
/// one that is the same folder once links are resolved.
fn folder_at<'a>(folders: &'a [WatchFolder], root: &Path) -> Option<&'a WatchFolder> {
    if let Some(found) = folders.iter().find(|f| Path::new(&f.path) == root) {
        return Some(found);
    }
    let real = std::fs::canonicalize(root).ok()?;
    folders
        .iter()
        .find(|f| std::fs::canonicalize(&f.path).is_ok_and(|other| other == real))
}

/// The work folder `name` was moved from `from_root` to `to_root` (by the
/// archive move): its work, if the library has one under a watch folder at
/// `from_root`, now belongs to the watch folder at `to_root` and keeps its
/// ID; when that folder already had a work of the name, the moved work's
/// records are merged into it. Nothing happens when either folder is not a
/// watch folder: the work is then found anew, or not, by a later scan.
///
/// Doing it again changes nothing more, so a command that stops after the move
/// and before this may run it on its next start.
pub async fn follow_move(
    library: &LibraryStore,
    live: &LiveWatch,
    from_root: PathBuf,
    to_root: PathBuf,
    name: String,
) -> Result<Followed, LibraryError> {
    let folders = library.folders().await?;
    let ids = tokio::task::spawn_blocking(move || {
        let from = folder_at(&folders, &from_root)?.id.clone();
        let to = folder_at(&folders, &to_root)?.id.clone();
        Some((from, to))
    })
    .await
    .ok()
    .flatten();
    let Some((from, to)) = ids else {
        return Ok(Followed::NotTracked);
    };
    let followed = library.follow_move(&from, &to, &name).await?;
    // The work folder's watches follow it, though the alerts of the move say so too.
    live.resync(&from, Some(&name));
    live.resync(&to, Some(&name));
    Ok(followed)
}

#[cfg(test)]
pub(crate) mod fixture;
#[cfg(test)]
mod scan_tests;

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[tokio::test]
    async fn a_scan_that_hangs_times_out_and_is_not_started_again_until_it_returns() {
        let (release, wait) = mpsc::channel::<()>();
        let timeout = Duration::from_millis(50);

        let hung = read_with_timeout("/hang", timeout, move || {
            let _ = wait.recv();
        })
        .await;
        let error = hung.unwrap_err();
        assert!(error.message.contains("멈췄어요"), "{error}");

        // The first thread is still running: no second scan of the folder starts.
        let started = Instant::now();
        let again = read_with_timeout("/hang", timeout, || {
            panic!("a second scan of a folder whose scan has not returned must not start")
        })
        .await;
        assert!(again.unwrap_err().message.contains("아직 끝나지 않았어요"));
        assert!(started.elapsed() < timeout);

        // Another folder is not held back.
        let other = read_with_timeout("/other", timeout, || ()).await;
        assert!(other.is_ok());

        // Once the thread returns, the folder can be scanned again.
        release.send(()).unwrap();
        for _ in 0..100 {
            if !SCANNING.lock().unwrap().contains("/hang") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let after = read_with_timeout("/hang", timeout, || ()).await;
        assert!(after.is_ok(), "{after:?}");
    }

    #[tokio::test]
    async fn what_a_scan_within_the_time_returns_is_returned_as_it_is() {
        let ok = read_with_timeout("/ok", Duration::from_secs(5), || 7).await;
        assert_eq!(ok.unwrap(), 7);
    }
}
