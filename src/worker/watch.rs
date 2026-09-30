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

use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use tokio_util::sync::CancellationToken;

use super::{Clock, CycleContext};
use crate::{
    discovery,
    store::library::{Followed, LibraryError, ScanReport, WatchFolder},
};

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
    let result = match tokio::task::spawn_blocking(move || discovery::scan(path.as_ref())).await {
        Ok(result) => result,
        Err(err) => {
            // The scan panicked: nothing is known about the folder.
            eprintln!("Watch folder {}: the scan ended with {err}", folder.path);
            Err(discovery::ScanError {
                message: "폴더를 읽다가 내부 오류가 났어요.".to_owned(),
                detail: err.to_string(),
            })
        }
    };
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

/// Reads every watch folder in turn. A folder that fails, or a database error
/// on one, is logged and the next folder is read.
pub async fn scan_all(ctx: &CycleContext, clock: &Clock, cancel: &CancellationToken) {
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
