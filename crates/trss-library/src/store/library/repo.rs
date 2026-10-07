//! Synchronous SQL of the library. Every write is one `BEGIN IMMEDIATE`
//! transaction, so a scan recorded by the web and one recorded by the worker
//! never interleave, and each reads the folder's state inside its own
//! transaction (which decides what is new).

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use uuid::Uuid;

use crate::{
    discovery::{EpisodeFile, FileKind, Reason, Scan, ScanError, ScannedWork, WorkRead},
    store::library::{
        AutomaticApplied, AutomaticPlan, EpisodeRecord, FileRecord, FolderSummary, Followed,
        LibraryError, ScanReport, UnrecognizedRecord, WatchFolder, WorkRecord,
    },
};
use trss_core::Millis;

fn begin(conn: &mut Connection) -> rusqlite::Result<Transaction<'_>> {
    conn.transaction_with_behavior(TransactionBehavior::Immediate)
}

fn new_id() -> String {
    Uuid::new_v4().to_string()
}

const FOLDER_COLUMNS: &str =
    "id, path, created_at, baselined, checked_at, error, automatic, watch_note";

fn folder_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WatchFolder> {
    Ok(WatchFolder {
        id: row.get(0)?,
        path: row.get(1)?,
        created_at: row.get(2)?,
        baselined: row.get::<_, i64>(3)? != 0,
        checked_at: row.get(4)?,
        error: row.get(5)?,
        automatic: row.get::<_, i64>(6)? != 0,
        watch_note: row.get(7)?,
    })
}

pub(super) fn folders(conn: &Connection) -> rusqlite::Result<Vec<WatchFolder>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {FOLDER_COLUMNS} FROM watch_folders WHERE unregistered_at IS NULL ORDER BY rowid"
    ))?;
    let rows = stmt.query_map([], folder_from_row)?;
    rows.collect()
}

pub(super) fn folder(conn: &Connection, id: &str) -> rusqlite::Result<Option<WatchFolder>> {
    conn.query_row(
        &format!(
            "SELECT {FOLDER_COLUMNS} FROM watch_folders WHERE id = ?1 AND unregistered_at IS NULL"
        ),
        [id],
        folder_from_row,
    )
    .optional()
}

pub(super) fn summaries(
    conn: &Connection,
    new_since: Millis,
) -> rusqlite::Result<Vec<FolderSummary>> {
    let mut stmt = conn.prepare(
        "SELECT f.id, f.path, f.created_at, f.baselined, f.checked_at, f.error, f.automatic, f.watch_note,
                (SELECT count(*) FROM works w WHERE w.watch_folder_id = f.id),
                (SELECT count(*) FROM works w WHERE w.watch_folder_id = f.id AND w.missing = 1),
                (SELECT count(*) FROM works w
                  WHERE w.watch_folder_id = f.id AND w.missing = 0
                    AND w.first_seen_at IS NOT NULL AND w.first_seen_at >= ?1)
           FROM watch_folders f WHERE f.unregistered_at IS NULL ORDER BY f.rowid",
    )?;
    let rows = stmt.query_map([new_since], |row| {
        Ok(FolderSummary {
            folder: folder_from_row(row)?,
            works: row.get::<_, i64>(8)? as usize,
            missing_works: row.get::<_, i64>(9)? as usize,
            new_works: row.get::<_, i64>(10)? as usize,
        })
    })?;
    rows.collect()
}

/// Fails with [`LibraryError::Changed`] unless the registered folders are
/// `expected`.
fn require_folders(
    tx: &Transaction<'_>,
    expected: &[(String, String, bool)],
) -> Result<(), LibraryError> {
    let current = AutomaticPlan::over(&folders(tx)?).based_on;
    if current == expected {
        Ok(())
    } else {
        Err(LibraryError::Changed)
    }
}

pub(super) fn add_folder(
    conn: &mut Connection,
    path: &str,
    scan: &Scan,
    now: Millis,
    checked_against: &[(String, String, bool)],
) -> Result<(WatchFolder, ScanReport), LibraryError> {
    let tx = begin(conn)?;
    // The caller checked overlaps against these folders outside the
    // transaction; a folder registered meanwhile may overlap the new one.
    let registered = folders(&tx)?;
    if registered.iter().any(|f| f.path == path) {
        return Err(LibraryError::Duplicate);
    }
    require_folders(&tx, checked_against)?;
    let id = insert_folder(&tx, path, false, now)?;
    let report = apply(&tx, &id, &Ok(scan.clone()), now, None)?.expect("the folder was just added");
    let folder = folder(&tx, &id)?.expect("the folder was just added");
    tx.commit()?;
    Ok((folder, report))
}

/// Registers the folder at `path`. A folder unregistered at the same path comes
/// back with its ID and its works (see [`detach_folder`]); it reads as never
/// read, so what its next reading finds that was not recorded has no added
/// time, like a new folder's first reading.
fn insert_folder(
    tx: &Transaction<'_>,
    path: &str,
    automatic: bool,
    now: Millis,
) -> Result<String, LibraryError> {
    let back: Option<String> = tx
        .query_row(
            "UPDATE watch_folders SET unregistered_at = NULL, automatic = ?2, created_at = ?3,
                    baselined = 0, error = NULL, watch_note = NULL
              WHERE path = ?1 AND unregistered_at IS NOT NULL
             RETURNING id",
            params![path, automatic, now],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(id) = back {
        return Ok(id);
    }
    let id = new_id();
    let inserted = tx.execute(
        "INSERT INTO watch_folders (id, path, created_at, automatic) VALUES (?1, ?2, ?3, ?4)",
        params![id, path, now, automatic],
    );
    match inserted {
        Ok(_) => Ok(id),
        Err(rusqlite::Error::SqliteFailure(e, _))
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Err(LibraryError::Duplicate)
        }
        Err(e) => Err(e.into()),
    }
}

/// Unregisters folder `id`: it stops being a watch folder, and its works leave
/// the library, but the folder's row stays with every work and what is linked
/// to them, so registering the same path again finds them under the same IDs.
/// How many works left the library; `None` when no such folder is registered.
fn detach_folder(tx: &Transaction<'_>, id: &str, now: Millis) -> rusqlite::Result<Option<usize>> {
    let works: i64 = tx.query_row(
        "SELECT count(*) FROM works WHERE watch_folder_id = ?1",
        [id],
        |row| row.get(0),
    )?;
    let detached = tx.execute(
        "UPDATE watch_folders SET unregistered_at = ?2, automatic = 0, watch_note = NULL
          WHERE id = ?1 AND unregistered_at IS NULL",
        params![id, now],
    )?;
    if detached == 0 {
        return Ok(None);
    }
    // Work folders waiting to be readable are a reading's state, not a record.
    tx.execute("DELETE FROM unread_works WHERE watch_folder_id = ?1", [id])?;
    Ok(Some(works as usize))
}

pub(super) fn remove_folder(
    conn: &mut Connection,
    id: &str,
    now: Millis,
) -> Result<Option<usize>, LibraryError> {
    let tx = begin(conn)?;
    if folder(&tx, id)?.is_some_and(|f| f.automatic) {
        return Err(LibraryError::Automatic);
    }
    let removed = detach_folder(&tx, id, now)?;
    tx.commit()?;
    Ok(removed)
}

pub(super) fn apply_automatic(
    tx: &Transaction<'_>,
    plan: &AutomaticPlan,
    now: Millis,
) -> Result<AutomaticApplied, LibraryError> {
    require_folders(tx, &plan.based_on)?;
    let mut applied = AutomaticApplied::default();
    // Removals first, so that a path given up can be taken by another folder.
    for id in &plan.remove {
        if let Some(works) = detach_folder(tx, id, now)? {
            applied.removed += 1;
            applied.removed_works += works;
        }
    }
    for (id, path) in &plan.keep {
        // An unregistered folder holding the path keeps it (it is that path's
        // to come back to); this folder then keeps the path it has, which is
        // the same place written otherwise.
        tx.execute(
            "UPDATE watch_folders SET automatic = 1,
                    path = CASE WHEN EXISTS (SELECT 1 FROM watch_folders o
                                              WHERE o.path = ?2 AND o.id <> ?1)
                                THEN path ELSE ?2 END,
                    watch_note = CASE WHEN path = ?2
                                       OR EXISTS (SELECT 1 FROM watch_folders o
                                                   WHERE o.path = ?2 AND o.id <> ?1)
                                      THEN watch_note END
              WHERE id = ?1",
            params![id, path],
        )?;
        applied.converted += 1;
    }
    for new in &plan.add {
        let id = insert_folder(tx, &new.path, true, now)?;
        if let Some(scan) = &new.scan {
            apply(tx, &id, &Ok(scan.clone()), now, None)?;
        }
        applied.added += 1;
    }
    Ok(applied)
}

pub(super) fn sync_automatic(
    conn: &mut Connection,
    plan: &AutomaticPlan,
    settings_version: i64,
    now: Millis,
) -> Result<AutomaticApplied, LibraryError> {
    let tx = begin(conn)?;
    let version: Option<i64> = tx
        .query_row(
            "SELECT version FROM collection_settings WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if version.unwrap_or(0) != settings_version {
        return Err(LibraryError::Changed);
    }
    let applied = apply_automatic(&tx, plan, now)?;
    tx.commit()?;
    Ok(applied)
}

/// See [`super::ensure_automatic_in`].
pub(super) fn ensure_automatic(
    tx: &Transaction<'_>,
    path: &str,
    now: Millis,
) -> rusqlite::Result<()> {
    // A folder unregistered at the path comes back with its works, read anew.
    let updated = tx.execute(
        "UPDATE watch_folders SET automatic = 1,
                baselined = CASE WHEN unregistered_at IS NULL THEN baselined ELSE 0 END,
                error = CASE WHEN unregistered_at IS NULL THEN error END,
                created_at = CASE WHEN unregistered_at IS NULL THEN created_at ELSE ?2 END,
                unregistered_at = NULL
          WHERE path = ?1",
        params![path, now],
    )?;
    if updated == 0 {
        tx.execute(
            "INSERT INTO watch_folders (id, path, created_at, automatic) VALUES (?1, ?2, ?3, 1)",
            params![new_id(), path, now],
        )?;
    }
    Ok(())
}

pub(super) fn record_scan(
    conn: &mut Connection,
    id: &str,
    scan: &Result<Scan, ScanError>,
    now: Millis,
) -> rusqlite::Result<Option<ScanReport>> {
    let tx = begin(conn)?;
    let report = apply(&tx, id, scan, now, None)?;
    tx.commit()?;
    Ok(report)
}

/// Records the reading of the works called `names` only.
pub(super) fn record_works(
    conn: &mut Connection,
    id: &str,
    names: &[String],
    scan: &Scan,
    now: Millis,
) -> rusqlite::Result<Option<ScanReport>> {
    let tx = begin(conn)?;
    let scope: HashSet<&str> = names.iter().map(String::as_str).collect();
    let report = apply(&tx, id, &Ok(scan.clone()), now, Some(&scope))?;
    tx.commit()?;
    Ok(report)
}

pub(super) fn set_watch_note(
    conn: &Connection,
    id: &str,
    note: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE watch_folders SET watch_note = ?2 WHERE id = ?1 AND unregistered_at IS NULL",
        params![id, note],
    )?;
    Ok(())
}

/// A work as the store has it, before a scan.
struct KnownWork {
    id: String,
    missing: bool,
}

/// Records `scan` for folder `folder_id`. With a `scope`, the scan covers only
/// the works of those names: it is the reading of just those work folders (a
/// name in the scope that the scan does not have is a work folder that is gone),
/// every other work and the folder's own error are left as they are, and the
/// folder is not baselined by it.
fn apply(
    tx: &Transaction<'_>,
    folder_id: &str,
    scan: &Result<Scan, ScanError>,
    now: Millis,
    scope: Option<&HashSet<&str>>,
) -> rusqlite::Result<Option<ScanReport>> {
    let in_scope = |name: &str| scope.is_none_or(|names| names.contains(name));
    let baselined: Option<i64> = tx
        .query_row(
            "SELECT baselined FROM watch_folders WHERE id = ?1 AND unregistered_at IS NULL",
            [folder_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(baselined) = baselined else {
        return Ok(None);
    };

    let scan = match scan {
        Err(error) => {
            // What was known stays: a folder that cannot be read says nothing
            // about what is in it.
            tx.execute(
                "UPDATE watch_folders SET checked_at = ?2, error = ?3 WHERE id = ?1",
                params![folder_id, now, error.message],
            )?;
            return Ok(Some(ScanReport {
                error: Some(error.message.clone()),
                ..ScanReport::default()
            }));
        }
        Ok(scan) => scan,
    };

    // The folder's first reading: what it finds was there before the app
    // looked (unknown age), not new. What a later scan finds first is stamped
    // with that scan's time.
    let baseline = baselined == 0;
    let stamp: Option<Millis> = if baseline { None } else { Some(now) };

    // Work folders an earlier scan saw and could not read (and the time that
    // scan was, `None` for the first one).
    let mut pending: HashMap<String, Option<Millis>> = HashMap::new();
    {
        let mut stmt =
            tx.prepare("SELECT dir_name, seen_at FROM unread_works WHERE watch_folder_id = ?1")?;
        let rows = stmt.query_map([folder_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
        for row in rows {
            let (name, seen_at): (String, Option<Millis>) = row?;
            if in_scope(&name) {
                pending.insert(name, seen_at);
            }
        }
    }

    let mut known: HashMap<String, KnownWork> = HashMap::new();
    {
        let mut stmt =
            tx.prepare("SELECT dir_name, id, missing FROM works WHERE watch_folder_id = ?1")?;
        let rows = stmt.query_map([folder_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                KnownWork {
                    id: row.get(1)?,
                    missing: row.get::<_, i64>(2)? != 0,
                },
            ))
        })?;
        for row in rows {
            let (name, work) = row?;
            known.insert(name, work);
        }
    }

    let mut report = ScanReport {
        baseline,
        ..ScanReport::default()
    };
    let mut seen: HashSet<&str> = HashSet::new();
    let mut unreadable: Vec<(&str, &str)> = Vec::new();

    for read in &scan.works {
        seen.insert(read.dir_name());
        match read {
            WorkRead::Unreadable { dir_name, reason } => {
                // Its records stay; a work not recorded yet waits for a scan
                // that can read it, and is dated by this one.
                if !known.contains_key(dir_name) && !pending.contains_key(dir_name) {
                    tx.execute(
                        "INSERT INTO unread_works (watch_folder_id, dir_name, seen_at)
                         VALUES (?1, ?2, ?3)",
                        params![folder_id, dir_name, stamp],
                    )?;
                }
                unreadable.push((dir_name, reason));
                report.works_unreadable += 1;
            }
            WorkRead::Read(work) => {
                let mut files_stamp = stamp;
                let id = match known.get(&work.dir_name) {
                    Some(known) => {
                        if known.missing {
                            tx.execute("UPDATE works SET missing = 0 WHERE id = ?1", [&known.id])?;
                        }
                        known.id.clone()
                    }
                    None => {
                        let id = new_id();
                        // A folder first read after a scan that could not read
                        // it is dated by that scan, and its files may have been
                        // there then: unknown.
                        let first_seen = match pending.get(&work.dir_name) {
                            Some(seen_at) => {
                                files_stamp = None;
                                *seen_at
                            }
                            None => stamp,
                        };
                        tx.execute(
                            "INSERT INTO works (id, watch_folder_id, dir_name, first_seen_at)
                             VALUES (?1, ?2, ?3, ?4)",
                            params![id, folder_id, work.dir_name, first_seen],
                        )?;
                        report.works_added += 1;
                        id
                    }
                };
                report.works_found += 1;
                sync_work(tx, &id, work, files_stamp, now, &mut report)?;
            }
        }
    }

    for (name, work) in &known {
        if seen.contains(name.as_str()) || !in_scope(name) {
            continue;
        }
        if !work.missing {
            tx.execute("UPDATE works SET missing = 1 WHERE id = ?1", [&work.id])?;
        }
    }
    // A pending folder that was read is a work now, and one that is gone is
    // not waiting for anything.
    for name in pending.keys() {
        if !unreadable.iter().any(|(n, _)| n == name) {
            tx.execute(
                "DELETE FROM unread_works WHERE watch_folder_id = ?1 AND dir_name = ?2",
                params![folder_id, name],
            )?;
        }
    }
    report.works_missing = tx.query_row(
        "SELECT count(*) FROM works WHERE watch_folder_id = ?1 AND missing = 1",
        [folder_id],
        |row| row.get::<_, i64>(0),
    )? as usize;

    let error = unreadable_sentence(&unreadable);
    if scope.is_some() {
        // Only some works were read: that can add to the folder's error, not
        // clear what other works or an earlier folder-wide read left there.
        match &error {
            Some(error) => tx.execute(
                "UPDATE watch_folders SET checked_at = ?2, error = ?3 WHERE id = ?1",
                params![folder_id, now, error],
            )?,
            None => tx.execute(
                "UPDATE watch_folders SET checked_at = ?2 WHERE id = ?1",
                params![folder_id, now],
            )?,
        };
    } else {
        tx.execute(
            "UPDATE watch_folders SET checked_at = ?2, error = ?3, baselined = 1 WHERE id = ?1",
            params![folder_id, now, error],
        )?;
    }
    report.error = error;
    Ok(Some(report))
}

/// The folder's error for work folders that could not be read.
fn unreadable_sentence(unreadable: &[(&str, &str)]) -> Option<String> {
    let (first, reason) = unreadable.first()?;
    let more = match unreadable.len() {
        1 => String::new(),
        n => format!(" 외 {}개", n - 1),
    };
    Some(format!(
        "작품 폴더 `{first}`{more}를 읽지 못해서 그 작품의 기록은 그대로 두었어요. {reason}"
    ))
}

struct KnownFile {
    season: u32,
    episode: String,
    kind: String,
    added_at: Option<Millis>,
    /// The subtitle source the user named for the file, and its version.
    creator: Option<String>,
    creator_version: i64,
    creator_set_at: Option<Millis>,
}

/// What a file recorded again keeps of the row it had: the time it was added
/// and its creator.
struct Carried {
    added_at: Option<Millis>,
    creator: Option<String>,
    creator_version: i64,
    creator_set_at: Option<Millis>,
}

/// Makes the records of one work the scan's: the files that are gone are
/// dropped, the new ones get `stamp`, and the seasons and unrecognized files
/// are the scan's.
fn sync_work(
    tx: &Transaction<'_>,
    work_id: &str,
    scanned: &ScannedWork,
    stamp: Option<Millis>,
    now: Millis,
    report: &mut ScanReport,
) -> rusqlite::Result<()> {
    let mut known: HashMap<String, KnownFile> = HashMap::new();
    {
        let mut stmt = tx.prepare(
            "SELECT path, season, episode, kind, added_at, creator_source_id, creator_version,
                    creator_set_at
               FROM media_files WHERE work_id = ?1",
        )?;
        let rows = stmt.query_map([work_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                KnownFile {
                    season: row.get(1)?,
                    episode: row.get(2)?,
                    kind: row.get(3)?,
                    added_at: row.get(4)?,
                    creator: row.get(5)?,
                    creator_version: row.get(6)?,
                    creator_set_at: row.get(7)?,
                },
            ))
        })?;
        for row in rows {
            let (path, file) = row?;
            known.insert(path, file);
        }
    }

    let scanned_files: HashMap<&str, &EpisodeFile> =
        scanned.files.iter().map(|f| (f.path.as_str(), f)).collect();
    // A file whose place in the episodes changed (its name is read differently
    // now) is dropped and recorded again with the time it had and the creator
    // the user named for it.
    let mut carried: HashMap<&str, Carried> = HashMap::new();
    let mut removed = 0;
    for (path, file) in &known {
        let now = scanned_files.get(path.as_str());
        let unchanged = now.is_some_and(|now| {
            now.season == file.season && now.episode == file.episode && now.kind.code() == file.kind
        });
        if unchanged {
            continue;
        }
        tx.execute(
            "DELETE FROM media_files WHERE work_id = ?1 AND path = ?2",
            params![work_id, path],
        )?;
        if now.is_some() {
            carried.insert(
                path,
                Carried {
                    added_at: file.added_at,
                    creator: file.creator.clone(),
                    creator_version: file.creator_version,
                    creator_set_at: file.creator_set_at,
                },
            );
        } else {
            removed += 1;
        }
    }
    report.files_removed += removed;
    if removed > 0 || !carried.is_empty() {
        // An episode lives only while a file of it does.
        tx.execute(
            "DELETE FROM episodes WHERE work_id = ?1 AND NOT EXISTS (
                 SELECT 1 FROM media_files m
                  WHERE m.work_id = episodes.work_id AND m.season = episodes.season
                    AND m.episode = episodes.episode)",
            [work_id],
        )?;
    }

    // Seasons that have a folder.
    let recorded: HashSet<u32> = {
        let mut stmt = tx.prepare("SELECT number FROM seasons WHERE work_id = ?1")?;
        let rows = stmt.query_map([work_id], |row| row.get::<_, u32>(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for season in recorded.difference(&scanned.seasons.iter().copied().collect()) {
        tx.execute(
            "DELETE FROM seasons WHERE work_id = ?1 AND number = ?2",
            params![work_id, season],
        )?;
    }
    for season in &scanned.seasons {
        if !recorded.contains(season) {
            tx.execute(
                "INSERT INTO seasons (work_id, number) VALUES (?1, ?2)",
                params![work_id, season],
            )?;
        }
    }

    // New files, and the ones recorded again.
    for file in &scanned.files {
        let (added_at, creator, creator_version, creator_set_at) = match (
            known.contains_key(&file.path),
            carried.get(file.path.as_str()),
        ) {
            (true, None) => continue,
            // A creator is a subtitle's: a file read as a video now has none.
            (_, Some(old)) if file.kind == FileKind::Subtitle => (
                old.added_at,
                old.creator.as_deref(),
                old.creator_version,
                old.creator_set_at,
            ),
            // The creator the file loses is a change: the version goes up, so a
            // screen that read the file with it cannot name over the loss.
            (_, Some(old)) => (
                old.added_at,
                None,
                old.creator_version + i64::from(old.creator.is_some()),
                None,
            ),
            // A new file starts at the scan's time, a version above any the
            // path had before: a screen that read an earlier file of this name
            // (one that was removed) cannot change this one.
            (false, None) => {
                report.files_added += 1;
                (stamp, None, now, None)
            }
        };
        tx.execute(
            "INSERT OR IGNORE INTO episodes (work_id, season, episode) VALUES (?1, ?2, ?3)",
            params![work_id, file.season, file.episode],
        )?;
        tx.execute(
            "INSERT INTO media_files
                 (work_id, path, season, episode, kind, added_at, creator_source_id,
                  creator_version, creator_set_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                work_id,
                file.path,
                file.season,
                file.episode,
                file.kind.code(),
                added_at,
                creator,
                creator_version,
                creator_set_at
            ],
        )?;
    }

    tx.execute(
        "DELETE FROM unrecognized_files WHERE work_id = ?1",
        [work_id],
    )?;
    for file in &scanned.unrecognized {
        // A size past `i64` is no file a disk holds; it is recorded as unread.
        let check = file
            .check
            .and_then(|c| Some((i64::try_from(c.size).ok()?, c.mtime_ns)));
        tx.execute(
            "INSERT OR REPLACE INTO unrecognized_files (work_id, path, reason, size, mtime_ns)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                work_id,
                file.path,
                file.reason.code(),
                check.map(|c| c.0),
                check.map(|c| c.1)
            ],
        )?;
    }
    // A person's `확인함` lasts while the scan finds the same video at its
    // path; one it could not read this time keeps it (`checks.rs`).
    tx.execute(
        "DELETE FROM unrecognized_checks
          WHERE work_id = ?1
            AND NOT EXISTS (
                SELECT 1 FROM unrecognized_files u
                 WHERE u.work_id = ?1 AND u.path = unrecognized_checks.path
                   AND (u.size IS NULL
                        OR (u.size = unrecognized_checks.size
                            AND u.mtime_ns = unrecognized_checks.mtime_ns)))",
        [work_id],
    )?;
    Ok(())
}

pub(super) fn follow_move(
    conn: &mut Connection,
    from: &str,
    to: &str,
    name: &str,
) -> rusqlite::Result<Followed> {
    let tx = begin(conn)?;
    let find = |folder: &str| -> rusqlite::Result<Option<(String, Option<Millis>)>> {
        tx.query_row(
            "SELECT id, first_seen_at FROM works WHERE watch_folder_id = ?1 AND dir_name = ?2",
            params![folder, name],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
    };
    let Some((moved, moved_seen)) = find(from)? else {
        return Ok(Followed::NotTracked);
    };
    let followed = match find(to)? {
        None => {
            tx.execute(
                "UPDATE works SET watch_folder_id = ?2, missing = 0 WHERE id = ?1",
                params![moved, to],
            )?;
            Followed::Moved
        }
        Some((kept, kept_seen)) => {
            merge_work(&tx, &moved, &kept)?;
            // What the user chose for the moved work is not lost with its row.
            crate::store::artwork::merge_selection(&tx, &moved, &kept)?;
            crate::store::seasons::merge_links(&tx, &moved, &kept)?;
            crate::store::seasons::merge_anissia_links(&tx, &moved, &kept)?;
            // Subscriptions connected to the moved work's seasons follow them.
            crate::store::seasons::follow_subscriptions(&tx, &moved, &kept)?;
            merge_subtitle_mappings(&tx, &moved, &kept)?;
            // Unknown is the older: the work was there before the app looked.
            let seen = match (moved_seen, kept_seen) {
                (Some(a), Some(b)) => Some(a.min(b)),
                _ => None,
            };
            tx.execute(
                "UPDATE works SET first_seen_at = ?2, missing = 0 WHERE id = ?1",
                params![kept, seen],
            )?;
            tx.execute("DELETE FROM works WHERE id = ?1", [&moved])?;
            Followed::Merged
        }
    };
    tx.commit()?;
    Ok(followed)
}

/// Copies what is recorded for work `from` into work `into`; what `into`
/// already has of the same key stays as it is.
fn merge_work(tx: &Transaction<'_>, from: &str, into: &str) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO seasons (work_id, number)
         SELECT ?2, number FROM seasons WHERE work_id = ?1",
        params![from, into],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO episodes (work_id, season, episode)
         SELECT ?2, season, episode FROM episodes WHERE work_id = ?1",
        params![from, into],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO media_files
             (work_id, path, season, episode, kind, added_at, creator_source_id, creator_version,
              creator_set_at)
         SELECT ?2, path, season, episode, kind, added_at, creator_source_id, creator_version,
                creator_set_at
           FROM media_files WHERE work_id = ?1",
        params![from, into],
    )?;
    // A subtitle both works have: the one with a creator named wins over one
    // with none (`into`'s creator stays when both are named), and the version
    // goes past both so no screen that read either can change it.
    tx.execute(
        "UPDATE media_files
            SET creator_source_id = (SELECT f.creator_source_id FROM media_files f
                                      WHERE f.work_id = ?1 AND f.path = media_files.path),
                creator_set_at = (SELECT f.creator_set_at FROM media_files f
                                   WHERE f.work_id = ?1 AND f.path = media_files.path),
                creator_version = MAX(creator_version,
                                      (SELECT f.creator_version FROM media_files f
                                        WHERE f.work_id = ?1 AND f.path = media_files.path)) + 1
          WHERE work_id = ?2 AND kind = 'subtitle' AND creator_source_id IS NULL
            AND EXISTS (SELECT 1 FROM media_files f
                         WHERE f.work_id = ?1 AND f.path = media_files.path
                           AND f.kind = 'subtitle' AND f.creator_source_id IS NOT NULL)",
        params![from, into],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO unrecognized_files (work_id, path, reason, size, mtime_ns)
         SELECT ?2, path, reason, size, mtime_ns FROM unrecognized_files WHERE work_id = ?1",
        params![from, into],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO unrecognized_checks (work_id, path, size, mtime_ns, checked_at)
         SELECT ?2, path, size, mtime_ns, checked_at FROM unrecognized_checks WHERE work_id = ?1",
        params![from, into],
    )?;
    Ok(())
}

/// Moves the subtitle sources' episode mappings of work `from`
/// (`subtitle_episode_mappings`, which the subscribed creator's receipts
/// keep) to work `into`. Where both have one for a season and source, the kept
/// work's stays, unless the moved one is the user's.
///
/// The conflicts of a source's episodes (`subtitle_mapping_conflicts`) belong
/// to the mapping they were found against, so they go where that mapping
/// does: the moved work's replace the kept work's for each (season, source)
/// whose mapping `into` now has from `from`, and the kept work's stay for the
/// others. The follower rewrites them at its next look anyway. The user's
/// exceptions (`subtitle_episode_exceptions`) go the same way, with the user's
/// mapping they belong to. A mapping written here has a new version (the next
/// of `subtitle_mapping_clock`), since it is not the row a screen read.
fn merge_subtitle_mappings(tx: &Transaction<'_>, from: &str, into: &str) -> rusqlite::Result<()> {
    let version: i64 = tx.query_row(
        "UPDATE subtitle_mapping_clock SET version = version + 1 RETURNING version",
        [],
        |r| r.get(0),
    )?;
    tx.execute(
        "INSERT INTO subtitle_episode_mappings
             (work_id, season, source_id, kind, episode_offset, evidence, decided_at,
              retired_offset, version)
         SELECT ?2, season, source_id, kind, episode_offset, evidence, decided_at, retired_offset,
                ?3
           FROM subtitle_episode_mappings WHERE work_id = ?1
         ON CONFLICT (work_id, season, source_id) DO UPDATE SET
             kind = excluded.kind, episode_offset = excluded.episode_offset,
             evidence = excluded.evidence, decided_at = excluded.decided_at,
             retired_offset = excluded.retired_offset, version = excluded.version
         WHERE excluded.kind = 'user'",
        params![from, into, version],
    )?;
    // The (season, source) pairs whose mapping `into` has from `from` now.
    let moved = "SELECT m.season, m.source_id
                   FROM subtitle_episode_mappings m
                   JOIN subtitle_episode_mappings f
                     ON f.work_id = ?1 AND f.season = m.season AND f.source_id = m.source_id
                  WHERE m.work_id = ?2 AND m.kind = f.kind
                    AND m.episode_offset IS f.episode_offset AND m.evidence = f.evidence
                    AND m.decided_at = f.decided_at";
    tx.execute(
        &format!(
            "DELETE FROM subtitle_episode_exceptions
              WHERE work_id = ?2 AND (season, source_id) IN ({moved})"
        ),
        params![from, into],
    )?;
    tx.execute(
        &format!(
            "INSERT INTO subtitle_episode_exceptions
                 (work_id, season, source_id, episode_key, episode, target)
             SELECT ?2, season, source_id, episode_key, episode, target
               FROM subtitle_episode_exceptions
              WHERE work_id = ?1 AND (season, source_id) IN ({moved})"
        ),
        params![from, into],
    )?;
    tx.execute(
        &format!(
            "DELETE FROM subtitle_mapping_conflicts
              WHERE work_id = ?2 AND (season, source_id) IN ({moved})"
        ),
        params![from, into],
    )?;
    tx.execute(
        &format!(
            "INSERT OR IGNORE INTO subtitle_mapping_conflicts
                 (work_id, season, source_id, episode, reason, found_at)
             SELECT ?2, season, source_id, episode, reason, found_at
               FROM subtitle_mapping_conflicts
              WHERE work_id = ?1 AND (season, source_id) IN ({moved})"
        ),
        params![from, into],
    )?;
    Ok(())
}

pub(super) fn works(conn: &Connection, folder_id: &str) -> rusqlite::Result<Vec<WorkRecord>> {
    let mut works: Vec<WorkRecord> = {
        let mut stmt = conn.prepare(
            "SELECT id, watch_folder_id, dir_name, first_seen_at, missing
               FROM works WHERE watch_folder_id = ?1 ORDER BY dir_name",
        )?;
        let rows = stmt.query_map([folder_id], |row| {
            Ok(WorkRecord {
                id: row.get(0)?,
                watch_folder_id: row.get(1)?,
                dir_name: row.get(2)?,
                first_seen_at: row.get(3)?,
                missing: row.get::<_, i64>(4)? != 0,
                seasons: Vec::new(),
                episodes: Vec::new(),
                unrecognized: Vec::new(),
            })
        })?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for work in &mut works {
        let mut stmt =
            conn.prepare("SELECT number FROM seasons WHERE work_id = ?1 ORDER BY number")?;
        work.seasons = stmt
            .query_map([&work.id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;

        let mut stmt = conn.prepare(
            "SELECT m.season, m.episode, m.path, m.kind, m.added_at,
                    m.creator_source_id, s.creator_name, s.anime_no, m.creator_version
               FROM media_files m LEFT JOIN subtitle_sources s ON s.id = m.creator_source_id
              WHERE m.work_id = ?1 ORDER BY m.season, m.episode, m.path",
        )?;
        let rows = stmt.query_map([&work.id], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, String>(1)?,
                FileRecord {
                    path: row.get(2)?,
                    kind: FileKind::from_code(&row.get::<_, String>(3)?).unwrap_or(FileKind::Video),
                    added_at: row.get(4)?,
                    creator: super::creators::creator_of(row.get(5)?, row.get(6)?, row.get(7)?),
                    creator_version: row.get(8)?,
                    applied: None,
                },
            ))
        })?;
        for row in rows {
            let (season, episode, file) = row?;
            match work.episodes.last_mut() {
                Some(last) if last.season == season && last.episode == episode => {
                    last.files.push(file)
                }
                _ => work.episodes.push(EpisodeRecord {
                    season,
                    episode,
                    files: vec![file],
                }),
            }
        }

        let mut stmt = conn.prepare(super::UNRECOGNIZED_OF_WORK)?;
        let rows = stmt.query_map([&work.id], |row| {
            Ok(UnrecognizedRecord {
                path: row.get(0)?,
                reason: Reason::from_code(&row.get::<_, String>(1)?).unwrap_or(Reason::NoEpisode),
                checked: row.get(2)?,
            })
        })?;
        work.unrecognized = rows.collect::<rusqlite::Result<_>>()?;
    }
    Ok(works)
}

/// See [`super::LibraryStore::generation`].
pub(super) fn generation(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT generation FROM library_generation WHERE id = 1",
        [],
        |row| row.get(0),
    )
}
