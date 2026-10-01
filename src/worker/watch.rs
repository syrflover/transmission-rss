//! The worker's reading of the watch folders (`docs/specs/library.md`, 작품
//! 발견과 감시 폴더).
//!
//! Every cycle, after the RSS work and under the same lock, [`scan_all`] reads
//! each registered watch folder with [`crate::discovery::scan`] and records what
//! it found ([`crate::store::library`]). The `watch_rescan` command
//! ([`super::commands::watch_rescan`]) reads one folder the same way when the
//! user asks for `다시 확인`. The web reads a folder only when it is added, with
//! the same scan.
//!
//! A folder that cannot be read records its reason on the folder and keeps what
//! was known; it does not stop the other folders, the cycle, or the RSS work.
//! A scan only reads: it creates, moves and deletes no file.
//!
//! The duration of every scan is logged, since a large library (hundreds of
//! works, thousands of files) is read in the cycle's own time.
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
//! Every scan runs on a blocking thread, and the cycle waits for it under its
//! lock. A mount that stops answering (NFS, SMB) would hold the lock for good,
//! so a scan gets [`SCAN_TIMEOUT`]; past it the folder is recorded as unreadable
//! and the cycle goes on. The thread cannot be stopped, but a scan only reads, so
//! leaving it to finish (or hang) harms nothing. While it still runs, the folder
//! is not scanned again (the next attempts fail at once with a sentence saying
//! so), so a hung mount costs one thread, not one per cycle.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

use tokio_util::sync::CancellationToken;

use super::{Clock, CycleContext};
use crate::{
    automatic_watch::{self, Wanted},
    discovery,
    store::library::{Followed, LibraryError, ScanReport, WatchFolder},
};

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
async fn read_with_timeout<F>(
    path: &str,
    timeout: Duration,
    read: F,
) -> Result<discovery::Scan, discovery::ScanError>
where
    F: FnOnce() -> Result<discovery::Scan, discovery::ScanError> + Send + 'static,
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
        Ok(Ok(result)) => result,
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
    ctx: &CycleContext,
    folder: &WatchFolder,
    now: crate::store::history::Millis,
) -> Result<Scanned, LibraryError> {
    let started = Instant::now();
    let path = folder.path.clone();
    let result = read_with_timeout(&folder.path, SCAN_TIMEOUT, move || {
        discovery::scan(path.as_ref())
    })
    .await;
    let read_in = started.elapsed();
    let failure = result.as_ref().err().cloned();
    let recorded = ctx.library.record_scan(&folder.id, result, now).await?;
    let total = started.elapsed();

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
            println!(
                "Watch folder {}: {} works, {} new, {} files added, {} removed, {} missing; \
                 read in {} ms, scan took {} ms in all",
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

/// Makes the automatic watch folders the collect and archive folders of the
/// settings (see the module docs). Failures are logged: the folders already
/// registered are read all the same.
pub async fn sync_automatic(ctx: &CycleContext, now: crate::store::history::Millis) {
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

/// Reads every watch folder in turn. A folder that fails, or a database error
/// on one, is logged and the next folder is read.
pub async fn scan_all(ctx: &CycleContext, clock: &Clock, cancel: &CancellationToken) {
    sync_automatic(ctx, clock()).await;
    let folders = match ctx.library.folders().await {
        Ok(folders) => folders,
        Err(err) => {
            eprintln!("Watch folders: cannot list them: {err}");
            return;
        }
    };
    for folder in folders {
        if cancel.is_cancelled() {
            break;
        }
        if let Err(err) = scan_folder(ctx, &folder, clock()).await {
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
    ctx: &CycleContext,
    from_root: PathBuf,
    to_root: PathBuf,
    name: String,
) -> Result<Followed, LibraryError> {
    let folders = ctx.library.folders().await?;
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
    ctx.library.follow_move(&from, &to, &name).await
}

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
            Ok(discovery::Scan::default())
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
        let other = read_with_timeout("/other", timeout, || Ok(discovery::Scan::default())).await;
        assert!(other.is_ok());

        // Once the thread returns, the folder can be scanned again.
        release.send(()).unwrap();
        for _ in 0..100 {
            if !SCANNING.lock().unwrap().contains("/hang") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let after = read_with_timeout("/hang", timeout, || Ok(discovery::Scan::default())).await;
        assert!(after.is_ok(), "{after:?}");
    }

    #[tokio::test]
    async fn a_scan_within_the_time_is_returned_as_it_is() {
        let ok = read_with_timeout("/ok", Duration::from_secs(5), || {
            Ok(discovery::Scan::default())
        })
        .await;
        assert_eq!(ok, Ok(discovery::Scan::default()));
        let failed = read_with_timeout("/failed", Duration::from_secs(5), || {
            Err(discovery::ScanError {
                message: "m".into(),
                detail: "d".into(),
            })
        })
        .await;
        assert_eq!(failed.unwrap_err().message, "m");
    }
}
