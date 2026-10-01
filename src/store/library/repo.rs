//! Synchronous SQL of the library. Every write is one `BEGIN IMMEDIATE`
//! transaction, so a scan recorded by the web and one recorded by the worker
//! never interleave, and each reads the folder's state inside its own
//! transaction (which decides what is new).

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use uuid::Uuid;

use super::{
    AutomaticApplied, AutomaticPlan, EpisodeRecord, FileRecord, FolderSummary, Followed,
    LibraryError, ScanReport, UnrecognizedRecord, WatchFolder, WorkRecord,
};
use crate::{
    discovery::{EpisodeFile, FileKind, Reason, Scan, ScanError, ScannedWork, WorkRead},
    store::history::Millis,
};

fn begin(conn: &mut Connection) -> rusqlite::Result<Transaction<'_>> {
    conn.transaction_with_behavior(TransactionBehavior::Immediate)
}

fn new_id() -> String {
    Uuid::new_v4().to_string()
}

const FOLDER_COLUMNS: &str = "id, path, created_at, baselined, checked_at, error, automatic";

fn folder_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WatchFolder> {
    Ok(WatchFolder {
        id: row.get(0)?,
        path: row.get(1)?,
        created_at: row.get(2)?,
        baselined: row.get::<_, i64>(3)? != 0,
        checked_at: row.get(4)?,
        error: row.get(5)?,
        automatic: row.get::<_, i64>(6)? != 0,
    })
}

pub(super) fn folders(conn: &Connection) -> rusqlite::Result<Vec<WatchFolder>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {FOLDER_COLUMNS} FROM watch_folders ORDER BY rowid"
    ))?;
    let rows = stmt.query_map([], folder_from_row)?;
    rows.collect()
}

pub(super) fn folder(conn: &Connection, id: &str) -> rusqlite::Result<Option<WatchFolder>> {
    conn.query_row(
        &format!("SELECT {FOLDER_COLUMNS} FROM watch_folders WHERE id = ?1"),
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
        "SELECT f.id, f.path, f.created_at, f.baselined, f.checked_at, f.error, f.automatic,
                (SELECT count(*) FROM works w WHERE w.watch_folder_id = f.id),
                (SELECT count(*) FROM works w WHERE w.watch_folder_id = f.id AND w.missing = 1),
                (SELECT count(*) FROM works w
                  WHERE w.watch_folder_id = f.id AND w.missing = 0
                    AND w.first_seen_at IS NOT NULL AND w.first_seen_at >= ?1)
           FROM watch_folders f ORDER BY f.rowid",
    )?;
    let rows = stmt.query_map([new_since], |row| {
        Ok(FolderSummary {
            folder: folder_from_row(row)?,
            works: row.get::<_, i64>(7)? as usize,
            missing_works: row.get::<_, i64>(8)? as usize,
            new_works: row.get::<_, i64>(9)? as usize,
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
    let report = apply(&tx, &id, &Ok(scan.clone()), now)?.expect("the folder was just added");
    let folder = folder(&tx, &id)?.expect("the folder was just added");
    tx.commit()?;
    Ok((folder, report))
}

fn insert_folder(
    tx: &Transaction<'_>,
    path: &str,
    automatic: bool,
    now: Millis,
) -> Result<String, LibraryError> {
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

/// Deletes folder `id` with its works; how many works went.
fn delete_folder(tx: &Transaction<'_>, id: &str) -> rusqlite::Result<Option<usize>> {
    let works: i64 = tx.query_row(
        "SELECT count(*) FROM works WHERE watch_folder_id = ?1",
        [id],
        |row| row.get(0),
    )?;
    let removed = tx.execute("DELETE FROM watch_folders WHERE id = ?1", [id])?;
    Ok((removed > 0).then_some(works as usize))
}

pub(super) fn remove_folder(
    conn: &mut Connection,
    id: &str,
) -> Result<Option<usize>, LibraryError> {
    let tx = begin(conn)?;
    if folder(&tx, id)?.is_some_and(|f| f.automatic) {
        return Err(LibraryError::Automatic);
    }
    let removed = delete_folder(&tx, id)?;
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
        if let Some(works) = delete_folder(tx, id)? {
            applied.removed += 1;
            applied.removed_works += works;
        }
    }
    for (id, path) in &plan.keep {
        tx.execute(
            "UPDATE watch_folders SET automatic = 1, path = ?2 WHERE id = ?1",
            params![id, path],
        )?;
        applied.converted += 1;
    }
    for new in &plan.add {
        let id = insert_folder(tx, &new.path, true, now)?;
        if let Some(scan) = &new.scan {
            apply(tx, &id, &Ok(scan.clone()), now)?;
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
    let updated = tx.execute(
        "UPDATE watch_folders SET automatic = 1 WHERE path = ?1",
        [path],
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
    let report = apply(&tx, id, scan, now)?;
    tx.commit()?;
    Ok(report)
}

/// A work as the store has it, before a scan.
struct KnownWork {
    id: String,
    missing: bool,
}

fn apply(
    tx: &Transaction<'_>,
    folder_id: &str,
    scan: &Result<Scan, ScanError>,
    now: Millis,
) -> rusqlite::Result<Option<ScanReport>> {
    let baselined: Option<i64> = tx
        .query_row(
            "SELECT baselined FROM watch_folders WHERE id = ?1",
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
            let (name, seen_at) = row?;
            pending.insert(name, seen_at);
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
                sync_work(tx, &id, work, files_stamp, &mut report)?;
            }
        }
    }

    for (name, work) in &known {
        if seen.contains(name.as_str()) {
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
    tx.execute(
        "UPDATE watch_folders SET checked_at = ?2, error = ?3, baselined = 1 WHERE id = ?1",
        params![folder_id, now, error],
    )?;
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
}

/// Makes the records of one work the scan's: the files that are gone are
/// dropped, the new ones get `stamp`, and the seasons and unrecognized files
/// are the scan's.
fn sync_work(
    tx: &Transaction<'_>,
    work_id: &str,
    scanned: &ScannedWork,
    stamp: Option<Millis>,
    report: &mut ScanReport,
) -> rusqlite::Result<()> {
    let mut known: HashMap<String, KnownFile> = HashMap::new();
    {
        let mut stmt = tx.prepare(
            "SELECT path, season, episode, kind, added_at FROM media_files WHERE work_id = ?1",
        )?;
        let rows = stmt.query_map([work_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                KnownFile {
                    season: row.get(1)?,
                    episode: row.get(2)?,
                    kind: row.get(3)?,
                    added_at: row.get(4)?,
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
    // now) is dropped and recorded again with the time it had.
    let mut carried: HashMap<&str, Option<Millis>> = HashMap::new();
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
            carried.insert(path, file.added_at);
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
        let added_at = match (
            known.contains_key(&file.path),
            carried.get(file.path.as_str()),
        ) {
            (true, None) => continue,
            (_, Some(time)) => *time,
            (false, None) => {
                report.files_added += 1;
                stamp
            }
        };
        tx.execute(
            "INSERT OR IGNORE INTO episodes (work_id, season, episode) VALUES (?1, ?2, ?3)",
            params![work_id, file.season, file.episode],
        )?;
        tx.execute(
            "INSERT INTO media_files (work_id, path, season, episode, kind, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                work_id,
                file.path,
                file.season,
                file.episode,
                file.kind.code(),
                added_at
            ],
        )?;
    }

    tx.execute(
        "DELETE FROM unrecognized_files WHERE work_id = ?1",
        [work_id],
    )?;
    for file in &scanned.unrecognized {
        tx.execute(
            "INSERT OR REPLACE INTO unrecognized_files (work_id, path, reason)
             VALUES (?1, ?2, ?3)",
            params![work_id, file.path, file.reason.code()],
        )?;
    }
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
        "INSERT OR IGNORE INTO media_files (work_id, path, season, episode, kind, added_at)
         SELECT ?2, path, season, episode, kind, added_at FROM media_files WHERE work_id = ?1",
        params![from, into],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO unrecognized_files (work_id, path, reason)
         SELECT ?2, path, reason FROM unrecognized_files WHERE work_id = ?1",
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
            "SELECT season, episode, path, kind, added_at FROM media_files
              WHERE work_id = ?1 ORDER BY season, episode, path",
        )?;
        let rows = stmt.query_map([&work.id], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, String>(1)?,
                FileRecord {
                    path: row.get(2)?,
                    kind: FileKind::from_code(&row.get::<_, String>(3)?).unwrap_or(FileKind::Video),
                    added_at: row.get(4)?,
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

        let mut stmt = conn.prepare(
            "SELECT path, reason FROM unrecognized_files WHERE work_id = ?1 ORDER BY path",
        )?;
        let rows = stmt.query_map([&work.id], |row| {
            Ok(UnrecognizedRecord {
                path: row.get(0)?,
                reason: Reason::from_code(&row.get::<_, String>(1)?).unwrap_or(Reason::NoEpisode),
            })
        })?;
        work.unrecognized = rows.collect::<rusqlite::Result<_>>()?;
    }
    Ok(works)
}
