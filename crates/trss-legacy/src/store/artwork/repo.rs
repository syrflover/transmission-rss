//! Synchronous SQL of the artwork state. Every write is one `BEGIN IMMEDIATE`
//! transaction that reads the version it compares inside itself.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use uuid::Uuid;

use super::{
    ArtworkError, ClaimedJob, Format, ImageRef, Job, JobKind, Mode, Note, Searched, Selection,
    Source, UserChange,
};
use trss_core::Millis;

fn begin(conn: &mut Connection) -> rusqlite::Result<Transaction<'_>> {
    conn.transaction_with_behavior(TransactionBehavior::Immediate)
}

const COLUMNS: &str = "a.mode, a.source, a.anilist_media_id, a.image_id, a.image_origin,
     a.image_path, a.image_size, a.image_sha256, a.image_format, a.version, a.job,
     a.job_requested_at, a.job_attempts, a.job_not_before, a.note";

fn bad(column: usize, what: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        rusqlite::types::Type::Text,
        format!("unknown {what}").into(),
    )
}

/// Reads [`COLUMNS`] starting at `at`.
fn from_row(work_id: String, row: &rusqlite::Row<'_>, at: usize) -> rusqlite::Result<Selection> {
    let mode: String = row.get(at)?;
    let source: Option<String> = row.get(at + 1)?;
    let image_id: Option<String> = row.get(at + 3)?;
    let image = match image_id {
        None => None,
        Some(id) => {
            let origin: String = row.get(at + 4)?;
            let format: String = row.get(at + 8)?;
            Some(ImageRef {
                id,
                origin: Source::from_code(&origin).ok_or_else(|| bad(at + 4, "origin"))?,
                relative_path: row.get(at + 5)?,
                byte_size: row.get::<_, i64>(at + 6)?.max(0) as u64,
                sha256: row.get(at + 7)?,
                format: Format::from_code(&format).ok_or_else(|| bad(at + 8, "format"))?,
            })
        }
    };
    let job: Option<String> = row.get(at + 10)?;
    let job = match job {
        None => None,
        Some(kind) => Some(Job {
            kind: JobKind::from_code(&kind).ok_or_else(|| bad(at + 10, "job"))?,
            requested_at: row.get(at + 11)?,
            attempts: row.get::<_, i64>(at + 12)?.max(0) as u32,
            not_before: row.get(at + 13)?,
        }),
    };
    let note: Option<String> = row.get(at + 14)?;
    Ok(Selection {
        work_id,
        mode: Mode::from_code(&mode).ok_or_else(|| bad(at, "mode"))?,
        source: match source {
            None => None,
            Some(s) => Some(Source::from_code(&s).ok_or_else(|| bad(at + 1, "source"))?),
        },
        anilist_media_id: row.get(at + 2)?,
        image,
        version: row.get(at + 9)?,
        job,
        note: note.as_deref().and_then(Note::from_code),
    })
}

/// A work recorded before its row could exist (none should be) gets the
/// unselected `auto` row, without a search: only registration starts one.
fn ensure_row(tx: &Transaction<'_>, work_id: &str) -> Result<(), ArtworkError> {
    if !in_library(tx, work_id)? {
        return Err(ArtworkError::NotFound);
    }
    tx.execute(
        "INSERT OR IGNORE INTO work_artwork (work_id, mode) VALUES (?1, 'auto')",
        [work_id],
    )?;
    Ok(())
}

fn read(conn: &Connection, work_id: &str) -> rusqlite::Result<Option<Selection>> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM work_artwork a WHERE a.work_id = ?1"),
        [work_id],
        |row| from_row(work_id.to_owned(), row, 0),
    )
    .optional()
}

/// Whether `work_id` is a work in the library: recorded, and its watch folder
/// registered. A work of an unregistered folder keeps its selection, but it
/// is neither shown nor changed until the folder is registered again.
fn in_library(conn: &Connection, work_id: &str) -> rusqlite::Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM works w JOIN watch_folders f ON f.id = w.watch_folder_id
              WHERE w.id = ?1 AND f.unregistered_at IS NULL",
            [work_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .is_some())
}

pub(super) fn selection(conn: &mut Connection, work_id: &str) -> Result<Selection, ArtworkError> {
    if !in_library(conn, work_id)? {
        return Err(ArtworkError::NotFound);
    }
    if let Some(selection) = read(conn, work_id)? {
        return Ok(selection);
    }
    let tx = begin(conn)?;
    ensure_row(&tx, work_id)?;
    let selection = read(&tx, work_id)?.ok_or(ArtworkError::NotFound)?;
    tx.commit()?;
    Ok(selection)
}

pub(super) fn image_ids(conn: &Connection) -> rusqlite::Result<HashMap<String, String>> {
    let mut stmt =
        conn.prepare("SELECT work_id, image_id FROM work_artwork WHERE image_id IS NOT NULL")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}

pub(super) fn image_ids_of(
    conn: &Connection,
    work_ids: &[String],
) -> rusqlite::Result<HashMap<String, String>> {
    let mut stmt = conn
        .prepare("SELECT image_id FROM work_artwork WHERE work_id = ?1 AND image_id IS NOT NULL")?;
    let mut out = HashMap::new();
    for id in work_ids {
        if let Some(image_id) = stmt.query_row([id], |r| r.get(0)).optional()? {
            out.insert(id.clone(), image_id);
        }
    }
    Ok(out)
}

/// Reads the selection inside `tx` and checks it is at `expected`.
fn current(tx: &Transaction<'_>, work_id: &str, expected: i64) -> Result<Selection, ArtworkError> {
    ensure_row(tx, work_id)?;
    let selection = read(tx, work_id)?.ok_or(ArtworkError::NotFound)?;
    if selection.version != expected {
        return Err(ArtworkError::Conflict(Box::new(selection)));
    }
    Ok(selection)
}

pub(super) fn change(
    conn: &mut Connection,
    work_id: &str,
    expected: i64,
    change: UserChange,
    now: Millis,
) -> Result<Selection, ArtworkError> {
    let tx = begin(conn)?;
    let selection = current(&tx, work_id, expected)?;
    match change {
        UserChange::Clear => {
            tx.execute(
                "UPDATE work_artwork SET mode = 'disabled', source = NULL,
                     anilist_media_id = NULL, image_id = NULL, image_origin = NULL,
                     image_path = NULL, image_size = NULL, image_sha256 = NULL,
                     image_format = NULL, version = version + 1, job = NULL,
                     job_requested_at = NULL, job_attempts = 0, job_not_before = NULL,
                     job_image_url = NULL, note = NULL, note_at = NULL
                 WHERE work_id = ?1",
                [work_id],
            )?;
        }
        UserChange::Auto => {
            // Going back to `auto` drops what was selected (a new search
            // decides), and asks for that search.
            tx.execute(
                "UPDATE work_artwork SET mode = 'auto', source = NULL,
                     anilist_media_id = NULL, image_id = NULL, image_origin = NULL,
                     image_path = NULL, image_size = NULL, image_sha256 = NULL,
                     image_format = NULL, version = version + 1, job = 'search',
                     job_requested_at = ?2, job_attempts = 0, job_not_before = NULL,
                     job_image_url = NULL, note = NULL, note_at = NULL
                 WHERE work_id = ?1",
                params![work_id, now],
            )?;
        }
        UserChange::Repair => {
            if selection.source != Some(Source::Anilist) {
                return Err(ArtworkError::Invalid(
                    "AniList에서 고른 표지만 다시 받을 수 있어요.",
                ));
            }
            // The selected ID's image again; what is selected stays. The
            // request time moves past a job still running, so that job's
            // late result or failure tells itself apart from this request.
            tx.execute(
                "UPDATE work_artwork SET job = 'fetch',
                     job_requested_at = MAX(?2, COALESCE(job_requested_at + 1, ?2)),
                     job_attempts = 0, job_not_before = NULL, job_image_url = NULL,
                     note = NULL, note_at = NULL
                 WHERE work_id = ?1",
                params![work_id, now],
            )?;
        }
    }
    let selection = read(&tx, work_id)?.ok_or(ArtworkError::NotFound)?;
    tx.commit()?;
    Ok(selection)
}

/// Whether `relative_path` is a reserved file that no selection took yet.
fn staged(tx: &Transaction<'_>, relative_path: &str) -> rusqlite::Result<bool> {
    let state: Option<String> = tx
        .query_row(
            "SELECT state FROM artwork_files WHERE relative_path = ?1",
            [relative_path],
            |r| r.get(0),
        )
        .optional()?;
    Ok(state.as_deref() == Some("staging"))
}

/// Sets the image (and what is selected). `bump` for a change of the
/// selection; an image arriving for the selection as it is keeps the version,
/// so a user's choice made from it still applies.
fn set_image(
    tx: &Transaction<'_>,
    work_id: &str,
    mode: Mode,
    anilist_media_id: Option<i64>,
    image: &ImageRef,
    bump: bool,
) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE work_artwork SET mode = ?2, source = ?3, anilist_media_id = ?4,
             image_id = ?5, image_origin = ?3, image_path = ?6, image_size = ?7,
             image_sha256 = ?8, image_format = ?9, version = version + ?10, job = NULL,
             job_requested_at = NULL, job_attempts = 0, job_not_before = NULL,
             job_image_url = NULL, note = NULL, note_at = NULL
         WHERE work_id = ?1",
        params![
            work_id,
            mode.code(),
            image.origin.code(),
            anilist_media_id,
            image.id,
            image.relative_path,
            image.byte_size as i64,
            image.sha256,
            image.format.code(),
            bump as i64,
        ],
    )?;
    tx.execute(
        "UPDATE artwork_files SET state = 'published' WHERE relative_path = ?1",
        [&image.relative_path],
    )?;
    Ok(())
}

pub(super) fn select_manual(
    conn: &mut Connection,
    work_id: &str,
    expected: i64,
    anilist_media_id: Option<i64>,
    image: &ImageRef,
) -> Result<Selection, ArtworkError> {
    debug_assert_eq!(
        image.origin == Source::Anilist,
        anilist_media_id.is_some(),
        "an AniList image comes with its ID"
    );
    let tx = begin(conn)?;
    current(&tx, work_id, expected)?;
    if !staged(&tx, &image.relative_path)? {
        return Err(ArtworkError::Interrupted);
    }
    set_image(&tx, work_id, Mode::Manual, anilist_media_id, image, true)?;
    let selection = read(&tx, work_id)?.ok_or(ArtworkError::NotFound)?;
    tx.commit()?;
    Ok(selection)
}

/// Lets an `auto` cover follow the season links: when the entry the cover
/// follows ([`crate::store::seasons::cover_target`]) is not the selected one,
/// selects it and asks for its image. The image the work has stays until the
/// new one is received, so a failure leaves the cover as it was (with the
/// reason as the note). Whether the image was asked for.
pub(super) fn follow_season_link(
    conn: &mut Connection,
    work_id: &str,
    now: Millis,
) -> Result<bool, ArtworkError> {
    let tx = begin(conn)?;
    ensure_row(&tx, work_id)?;
    let Some(target) = crate::store::seasons::cover_target(&tx, work_id)? else {
        return Ok(false);
    };
    let selection = read(&tx, work_id)?.ok_or(ArtworkError::NotFound)?;
    if selection.mode != Mode::Auto
        || (selection.source == Some(Source::Anilist) && selection.anilist_media_id == Some(target))
    {
        return Ok(false);
    }
    // A new selection: a search or fetch of the old one that is still running
    // finds the version moved on and drops its result.
    tx.execute(
        "UPDATE work_artwork SET source = 'anilist', anilist_media_id = ?2,
             version = version + 1, job = 'fetch', job_requested_at = ?3,
             job_attempts = 0, job_not_before = NULL, job_image_url = NULL,
             note = NULL, note_at = NULL
         WHERE work_id = ?1",
        params![work_id, target, now],
    )?;
    tx.commit()?;
    Ok(true)
}

pub(super) fn next_job(conn: &Connection, now: Millis) -> rusqlite::Result<Option<ClaimedJob>> {
    conn.query_row(
        "SELECT a.work_id, w.dir_name, a.version, a.job, a.anilist_media_id,
                a.job_image_url, a.job_attempts, a.job_requested_at
           FROM work_artwork a JOIN works w ON w.id = a.work_id
           JOIN watch_folders f ON f.id = w.watch_folder_id AND f.unregistered_at IS NULL
          WHERE a.job IS NOT NULL AND (a.job_not_before IS NULL OR a.job_not_before <= ?1)
          ORDER BY a.job = 'search', a.job_requested_at, a.work_id
          LIMIT 1",
        [now],
        |row| {
            let kind: String = row.get(3)?;
            Ok(ClaimedJob {
                work_id: row.get(0)?,
                dir_name: row.get(1)?,
                version: row.get(2)?,
                kind: JobKind::from_code(&kind).ok_or_else(|| bad(3, "job"))?,
                anilist_media_id: row.get(4)?,
                image_url: row.get(5)?,
                attempts: row.get::<_, i64>(6)?.max(0) as u32,
                requested_at: row.get(7)?,
            })
        },
    )
    .optional()
}

/// Whether the job of `kind` taken at `version`, asked for at
/// `requested_at`, still stands: the selection did not change and that same
/// request is still waiting (in `auto` for a search). A repair asked for since
/// is a newer request, which runs itself.
fn job_stands(
    tx: &Transaction<'_>,
    work_id: &str,
    version: i64,
    requested_at: Millis,
    kind: JobKind,
) -> rusqlite::Result<Option<Selection>> {
    let Some(selection) = read(tx, work_id)? else {
        return Ok(None);
    };
    let wanted = selection.version == version
        && selection
            .job
            .as_ref()
            .is_some_and(|j| j.kind == kind && j.requested_at == requested_at)
        && (kind != JobKind::Search || selection.mode == Mode::Auto);
    Ok(wanted.then_some(selection))
}

pub(super) fn searched(
    conn: &mut Connection,
    work_id: &str,
    version: i64,
    requested_at: Millis,
    outcome: &Searched,
    now: Millis,
) -> rusqlite::Result<bool> {
    let tx = begin(conn)?;
    if job_stands(&tx, work_id, version, requested_at, JobKind::Search)?.is_none() {
        return Ok(false);
    }
    match outcome {
        Searched::Selected {
            anilist_media_id,
            image_url,
        } => {
            // The ID is selected now; the image follows, and until it is
            // verified the work shows no new image.
            tx.execute(
                "UPDATE work_artwork SET source = 'anilist', anilist_media_id = ?2,
                     image_id = NULL, image_origin = NULL, image_path = NULL,
                     image_size = NULL, image_sha256 = NULL, image_format = NULL,
                     version = version + 1, job = 'fetch', job_requested_at = ?3,
                     job_attempts = 0, job_not_before = NULL, job_image_url = ?4,
                     note = NULL, note_at = NULL
                 WHERE work_id = ?1",
                params![work_id, anilist_media_id, now, image_url],
            )?;
        }
        Searched::Left(note) => {
            tx.execute(
                "UPDATE work_artwork SET job = NULL, job_requested_at = NULL,
                     job_attempts = 0, job_not_before = NULL, job_image_url = NULL,
                     note = ?2, note_at = ?3
                 WHERE work_id = ?1",
                params![work_id, note.code(), now],
            )?;
        }
    }
    tx.commit()?;
    Ok(true)
}

pub(super) fn fetched(
    conn: &mut Connection,
    work_id: &str,
    version: i64,
    requested_at: Millis,
    anilist_media_id: i64,
    image: &ImageRef,
) -> Result<bool, ArtworkError> {
    let tx = begin(conn)?;
    let stands = job_stands(&tx, work_id, version, requested_at, JobKind::Fetch)?;
    let applies = match &stands {
        Some(s) => {
            s.anilist_media_id == Some(anilist_media_id) && staged(&tx, &image.relative_path)?
        }
        None => false,
    };
    if !applies {
        // Nobody refers to the new file: the cleanup removes it.
        tx.execute(
            "UPDATE artwork_files SET state = 'published' WHERE relative_path = ?1",
            [&image.relative_path],
        )?;
        tx.commit()?;
        return Ok(false);
    }
    let mode = stands.map(|s| s.mode).unwrap_or(Mode::Auto);
    set_image(&tx, work_id, mode, Some(anilist_media_id), image, false)?;
    tx.commit()?;
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn job_later(
    conn: &mut Connection,
    work_id: &str,
    version: i64,
    requested_at: Millis,
    retry_at: Option<Millis>,
    failed: bool,
    note: Note,
    now: Millis,
) -> rusqlite::Result<()> {
    let tx = begin(conn)?;
    let Some(selection) = read(&tx, work_id)? else {
        return Ok(());
    };
    // A newer request (a repair asked for while this one ran) is not touched.
    let same_request = selection
        .job
        .as_ref()
        .is_some_and(|j| j.requested_at == requested_at);
    if selection.version != version || !same_request {
        return Ok(());
    }
    match retry_at {
        Some(at) => {
            tx.execute(
                "UPDATE work_artwork SET job_not_before = ?2,
                     job_attempts = job_attempts + ?3
                 WHERE work_id = ?1",
                params![work_id, at, failed as i64],
            )?;
        }
        None => {
            tx.execute(
                "UPDATE work_artwork SET job = NULL, job_requested_at = NULL,
                     job_attempts = 0, job_not_before = NULL, job_image_url = NULL,
                     note = ?2, note_at = ?3
                 WHERE work_id = ?1",
                params![work_id, note.code(), now],
            )?;
        }
    }
    tx.commit()
}

/// Carries work `from`'s selection over to work `into` when an archive move
/// merges `from` into `into` (`from`'s row is about to go): only when `into`
/// is `auto` with nothing selected (or has no row yet) and `from` has
/// something chosen, `manual` or `auto` with an image. The image reference
/// comes along, so the file stays referenced. Otherwise `into` keeps its own.
/// The version moves past both, so no job or screen of either applies to it.
pub(crate) fn merge_selection(
    tx: &Transaction<'_>,
    from: &str,
    into: &str,
) -> rusqlite::Result<()> {
    let Some(moved) = read(tx, from)? else {
        return Ok(());
    };
    let chosen = moved.mode == Mode::Manual || (moved.mode == Mode::Auto && moved.image.is_some());
    if !chosen {
        return Ok(());
    }
    let kept = read(tx, into)?;
    let open = kept
        .as_ref()
        .is_none_or(|k| k.mode == Mode::Auto && k.source.is_none());
    if !open {
        return Ok(());
    }
    let version = kept.map_or(0, |k| k.version).max(moved.version) + 1;
    tx.execute(
        "INSERT OR REPLACE INTO work_artwork
             (work_id, mode, source, anilist_media_id, image_id, image_origin, image_path,
              image_size, image_sha256, image_format, version, job, job_requested_at,
              job_attempts, job_not_before, job_image_url, note, note_at)
         SELECT ?2, mode, source, anilist_media_id, image_id, image_origin, image_path,
                image_size, image_sha256, image_format, ?3, job, job_requested_at,
                job_attempts, job_not_before, job_image_url, note, note_at
           FROM work_artwork WHERE work_id = ?1",
        params![from, into, version],
    )?;
    Ok(())
}

// --- files -----------------------------------------------------------------------------

/// A file the app recorded in `artwork_files`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub relative_path: String,
    pub staging_path: String,
    pub dev: Option<u64>,
    pub ino: Option<u64>,
    pub created_at: Millis,
}

pub(super) fn reserve_file(
    conn: &mut Connection,
    relative_path: &str,
    staging_path: &str,
    now: Millis,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO artwork_files (relative_path, staging_path, state, created_at)
         VALUES (?1, ?2, 'staging', ?3)",
        params![relative_path, staging_path, now],
    )?;
    Ok(())
}

pub(super) fn file_identity(
    conn: &mut Connection,
    relative_path: &str,
    dev: u64,
    ino: u64,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE artwork_files SET dev = ?2, ino = ?3 WHERE relative_path = ?1",
        params![relative_path, dev as i64, ino as i64],
    )?;
    Ok(())
}

pub(super) fn forget_file(conn: &mut Connection, relative_path: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM artwork_files WHERE relative_path = ?1 AND state = 'staging'",
        [relative_path],
    )?;
    Ok(())
}

pub(super) fn abandon_file(conn: &mut Connection, relative_path: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE artwork_files SET state = 'published' WHERE relative_path = ?1",
        [relative_path],
    )?;
    Ok(())
}

/// The recorded files in `state`.
pub(crate) fn files_of_state(conn: &Connection, state: &str) -> rusqlite::Result<Vec<FileRow>> {
    let mut stmt = conn.prepare(
        "SELECT relative_path, staging_path, dev, ino, created_at
           FROM artwork_files WHERE state = ?1 ORDER BY relative_path",
    )?;
    let rows = stmt.query_map([state], |r| {
        Ok(FileRow {
            relative_path: r.get(0)?,
            staging_path: r.get(1)?,
            dev: r.get::<_, Option<i64>>(2)?.map(|v| v as u64),
            ino: r.get::<_, Option<i64>>(3)?.map(|v| v as u64),
            created_at: r.get(4)?,
        })
    })?;
    rows.collect()
}

/// Every image path a selection refers to, as written.
pub(crate) fn referenced_paths(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt =
        conn.prepare("SELECT DISTINCT image_path FROM work_artwork WHERE image_path IS NOT NULL")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    rows.collect()
}

pub(crate) fn delete_file_row(conn: &Connection, relative_path: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM artwork_files WHERE relative_path = ?1",
        [relative_path],
    )?;
    Ok(())
}

// --- request pace ----------------------------------------------------------------------

pub(super) fn take_slot(
    conn: &mut Connection,
    now: Millis,
    spacing_ms: i64,
    max_wait_ms: Option<i64>,
) -> rusqlite::Result<Result<Millis, i64>> {
    let tx = begin(conn)?;
    let pace: Option<(Millis, Option<Millis>)> = tx
        .query_row(
            "SELECT next_at, blocked_until FROM anilist_pace WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (next_at, blocked) = pace.unwrap_or((now, None));
    let slot = now.max(next_at).max(blocked.unwrap_or(now));
    if let Some(max) = max_wait_ms {
        if slot - now > max {
            return Ok(Err(slot - now));
        }
    }
    tx.execute(
        "INSERT INTO anilist_pace (id, next_at, blocked_until) VALUES (1, ?1, ?2)
         ON CONFLICT (id) DO UPDATE SET next_at = excluded.next_at",
        params![slot + spacing_ms, blocked],
    )?;
    tx.commit()?;
    Ok(Ok(slot))
}

pub(super) fn block(conn: &mut Connection, until: Millis) -> rusqlite::Result<()> {
    let tx = begin(conn)?;
    tx.execute(
        "INSERT INTO anilist_pace (id, next_at, blocked_until) VALUES (1, ?1, ?1)
         ON CONFLICT (id) DO UPDATE SET
             next_at = max(next_at, excluded.next_at),
             blocked_until = max(coalesce(blocked_until, 0), excluded.blocked_until)",
        params![until],
    )?;
    tx.commit()
}

/// A new app-issued ID (images, file names).
pub(crate) fn new_id() -> String {
    Uuid::new_v4().to_string()
}
