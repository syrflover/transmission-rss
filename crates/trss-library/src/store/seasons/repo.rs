//! Synchronous SQL of the season links. Every write is one `BEGIN IMMEDIATE`
//! transaction that reads the version it compares inside itself.

use std::collections::{BTreeMap, HashMap};

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use crate::store::seasons::{ClaimedSearch, Job, Note, Origin, SeasonError, SeasonLink};
use trss_anilist::{Airing, Entry, FuzzyDate, Sequel};
use trss_core::Millis;

/// A day, the time between two refreshes of an entry that is not finished.
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

fn begin(conn: &mut Connection) -> rusqlite::Result<Transaction<'_>> {
    conn.transaction_with_behavior(TransactionBehavior::Immediate)
}

const ENTRY_COLUMNS: &str = "e.id, e.romaji, e.english, e.native, e.format, e.status, e.episodes,
     e.start_year, e.start_month, e.start_day, e.end_year, e.end_month, e.end_day,
     e.studios, e.genres, e.description, e.airing, e.sequels, e.fetched_at, e.korean_titles";

fn json<T: serde::de::DeserializeOwned>(column: usize, text: &str) -> rusqlite::Result<T> {
    serde_json::from_str(text).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            e.to_string().into(),
        )
    })
}

fn entry_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    let studios: String = row.get(13)?;
    let genres: String = row.get(14)?;
    let airing: String = row.get(16)?;
    let sequels: String = row.get(17)?;
    let korean_titles: String = row.get(19)?;
    Ok(Entry {
        id: row.get(0)?,
        romaji: row.get(1)?,
        english: row.get(2)?,
        native: row.get(3)?,
        format: row.get(4)?,
        status: row.get(5)?,
        episodes: row.get::<_, Option<i64>>(6)?.map(|n| n.max(0) as u32),
        start: FuzzyDate {
            year: row.get(7)?,
            month: row.get(8)?,
            day: row.get(9)?,
        },
        end: FuzzyDate {
            year: row.get(10)?,
            month: row.get(11)?,
            day: row.get(12)?,
        },
        studios: json::<Vec<String>>(13, &studios)?,
        genres: json::<Vec<String>>(14, &genres)?,
        description: row.get(15)?,
        airing: json::<Vec<Airing>>(16, &airing)?,
        sequels: json::<Vec<Sequel>>(17, &sequels)?,
        fetched_at: row.get(18)?,
        korean_titles: json::<Vec<String>>(19, &korean_titles)?,
    })
}

pub(super) fn entry(conn: &Connection, id: i64) -> rusqlite::Result<Option<Entry>> {
    conn.query_row(
        &format!("SELECT {ENTRY_COLUMNS} FROM anilist_entries e WHERE e.id = ?1"),
        [id],
        entry_from_row,
    )
    .optional()
}

pub(super) fn put_entry(conn: &Connection, entry: &Entry) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO anilist_entries
             (id, romaji, english, native, format, status, episodes,
              start_year, start_month, start_day, end_year, end_month, end_day,
              studios, genres, description, airing, sequels, fetched_at, korean_titles,
              refresh_not_before)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                 ?18, ?19, ?20, NULL)
         ON CONFLICT (id) DO UPDATE SET
             romaji = excluded.romaji, english = excluded.english, native = excluded.native,
             format = excluded.format, status = excluded.status, episodes = excluded.episodes,
             start_year = excluded.start_year, start_month = excluded.start_month,
             start_day = excluded.start_day, end_year = excluded.end_year,
             end_month = excluded.end_month, end_day = excluded.end_day,
             studios = excluded.studios, genres = excluded.genres,
             description = excluded.description, airing = excluded.airing,
             sequels = excluded.sequels, fetched_at = excluded.fetched_at,
             korean_titles = excluded.korean_titles,
             refresh_not_before = NULL",
        params![
            entry.id,
            entry.romaji,
            entry.english,
            entry.native,
            entry.format,
            entry.status,
            entry.episodes,
            entry.start.year,
            entry.start.month,
            entry.start.day,
            entry.end.year,
            entry.end.month,
            entry.end.day,
            to_json(&entry.studios),
            to_json(&entry.genres),
            entry.description,
            to_json(&entry.airing),
            to_json(&entry.sequels),
            entry.fetched_at,
            to_json(&entry.korean_titles),
        ],
    )?;
    Ok(())
}

/// A stored list as JSON text.
fn to_json<T: serde::Serialize>(list: &T) -> String {
    serde_json::to_string(list).expect("plain data serializes")
}

/// Whether the season is recorded for a work in the library (one whose watch
/// folder is registered).
pub(super) fn season_exists(
    conn: &Connection,
    work_id: &str,
    season: u32,
) -> rusqlite::Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM seasons s
               JOIN works w ON w.id = s.work_id
               JOIN watch_folders f ON f.id = w.watch_folder_id AND f.unregistered_at IS NULL
              WHERE s.work_id = ?1 AND s.number = ?2",
            params![work_id, season],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .is_some())
}

pub(super) fn first_season(conn: &Connection, work_id: &str) -> rusqlite::Result<Option<u32>> {
    conn.query_row(
        "SELECT min(number) FROM seasons WHERE work_id = ?1 AND number >= 1",
        [work_id],
        |r| r.get(0),
    )
}

/// The AniList entry the work's cover follows: the first entry of the lowest
/// numbered recorded season that links one (not season 0, the specials).
pub(crate) fn cover_target(conn: &Connection, work_id: &str) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT l.anilist_id FROM season_entries l
           JOIN seasons s ON s.work_id = l.work_id AND s.number = l.season
          WHERE l.work_id = ?1 AND l.season >= 1
          ORDER BY l.season, l.position LIMIT 1",
        [work_id],
        |r| r.get(0),
    )
    .optional()
}

fn entries_of(conn: &Connection, work_id: &str, season: u32) -> rusqlite::Result<Vec<Entry>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {ENTRY_COLUMNS} FROM season_entries l JOIN anilist_entries e ON e.id = l.anilist_id
          WHERE l.work_id = ?1 AND l.season = ?2 ORDER BY l.position"
    ))?;
    let rows = stmt.query_map(params![work_id, season], entry_from_row)?;
    rows.collect()
}

struct InfoRow {
    version: i64,
    origin: Origin,
    job: Option<Job>,
    note: Option<Note>,
}

fn info_from_row(row: &rusqlite::Row<'_>, at: usize) -> rusqlite::Result<InfoRow> {
    let origin: String = row.get(at + 1)?;
    let job: Option<String> = row.get(at + 2)?;
    let note: Option<String> = row.get(at + 6)?;
    Ok(InfoRow {
        version: row.get(at)?,
        origin: Origin::from_code(&origin).ok_or_else(|| {
            rusqlite::Error::FromSqlConversionFailure(
                at + 1,
                rusqlite::types::Type::Text,
                "unknown origin".into(),
            )
        })?,
        job: match job {
            None => None,
            Some(_) => Some(Job {
                requested_at: row.get(at + 3)?,
                attempts: row.get::<_, i64>(at + 4)?.max(0) as u32,
                not_before: row.get(at + 5)?,
            }),
        },
        note: note.as_deref().and_then(Note::from_code),
    })
}

const INFO_COLUMNS: &str =
    "i.version, i.origin, i.job, i.job_requested_at, i.job_attempts, i.job_not_before, i.note";

fn read_info(conn: &Connection, work_id: &str, season: u32) -> rusqlite::Result<Option<InfoRow>> {
    conn.query_row(
        &format!("SELECT {INFO_COLUMNS} FROM season_info i WHERE i.work_id = ?1 AND i.season = ?2"),
        params![work_id, season],
        |row| info_from_row(row, 0),
    )
    .optional()
}

/// The link of a season as stored, or the untouched one (version 0).
fn read_link(conn: &Connection, work_id: &str, season: u32) -> rusqlite::Result<SeasonLink> {
    let info = read_info(conn, work_id, season)?;
    let entries = entries_of(conn, work_id, season)?;
    Ok(match info {
        Some(info) => SeasonLink {
            work_id: work_id.to_owned(),
            season,
            version: info.version,
            origin: info.origin,
            entries,
            job: info.job,
            note: info.note,
        },
        None => SeasonLink {
            work_id: work_id.to_owned(),
            season,
            version: 0,
            origin: Origin::Auto,
            entries,
            job: None,
            note: None,
        },
    })
}

pub(super) fn link_of_recorded(
    conn: &Connection,
    work_id: &str,
    season: u32,
) -> Result<SeasonLink, SeasonError> {
    if !season_exists(conn, work_id, season)? {
        return Err(SeasonError::NotFound);
    }
    Ok(read_link(conn, work_id, season)?)
}

pub(super) fn links_of(
    conn: &Connection,
    work_id: &str,
) -> rusqlite::Result<BTreeMap<u32, SeasonLink>> {
    let seasons: Vec<u32> = {
        let mut stmt = conn.prepare("SELECT season FROM season_info WHERE work_id = ?1")?;
        let rows = stmt.query_map([work_id], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    seasons
        .into_iter()
        .map(|season| Ok((season, read_link(conn, work_id, season)?)))
        .collect()
}

/// Reads the link inside `tx` for a user's change made from `expected`: the
/// season must be recorded and the version must match.
fn current(
    tx: &Transaction<'_>,
    work_id: &str,
    season: u32,
    expected: i64,
) -> Result<SeasonLink, SeasonError> {
    if !season_exists(tx, work_id, season)? {
        return Err(SeasonError::NotFound);
    }
    let link = read_link(tx, work_id, season)?;
    if link.version != expected {
        return Err(SeasonError::Conflict(Box::new(link)));
    }
    Ok(link)
}

/// Creates the season's row if it has none (version 1), or moves it to the
/// next version, and sets who made the links and what is left to do.
fn bump(
    tx: &Transaction<'_>,
    work_id: &str,
    season: u32,
    origin: Origin,
    search_requested: Option<Millis>,
) -> rusqlite::Result<()> {
    let job = search_requested.map(|_| "search");
    tx.execute(
        "INSERT INTO season_info (work_id, season, version, origin, job, job_requested_at)
         VALUES (?1, ?2, 1, ?3, ?4, ?5)
         ON CONFLICT (work_id, season) DO UPDATE SET
             version = version + 1, origin = excluded.origin, job = excluded.job,
             job_requested_at = excluded.job_requested_at, job_attempts = 0,
             job_not_before = NULL, note = NULL",
        params![work_id, season, origin.code(), job, search_requested],
    )?;
    Ok(())
}

fn replace_entries(
    tx: &Transaction<'_>,
    work_id: &str,
    season: u32,
    ids: &[i64],
) -> Result<(), SeasonError> {
    tx.execute(
        "DELETE FROM season_entries WHERE work_id = ?1 AND season = ?2",
        params![work_id, season],
    )?;
    for (position, id) in ids.iter().enumerate() {
        let known = tx
            .query_row("SELECT 1 FROM anilist_entries WHERE id = ?1", [id], |r| {
                r.get::<_, i64>(0)
            })
            .optional()?;
        if known.is_none() {
            return Err(SeasonError::MissingEntry(*id));
        }
        tx.execute(
            "INSERT INTO season_entries (work_id, season, position, anilist_id)
             VALUES (?1, ?2, ?3, ?4)",
            params![work_id, season, position as i64, id],
        )?;
    }
    Ok(())
}

/// Carries work `from`'s season links over to work `into` when an archive
/// move merges `from` into `into` (`from`'s rows are about to go): each season
/// of `from` that links entries, where `into`'s same season links none. Who
/// linked them comes along; the version moves past both, and a pending search
/// of `into`'s season is dropped (the season is linked now). Seasons `into`
/// has linked keep their links.
pub(crate) fn merge_links(tx: &Transaction<'_>, from: &str, into: &str) -> rusqlite::Result<()> {
    let seasons: Vec<(u32, i64, String)> = {
        let mut stmt = tx.prepare(
            "SELECT i.season, i.version, i.origin FROM season_info i
              WHERE i.work_id = ?1
                AND EXISTS (SELECT 1 FROM season_entries l
                             WHERE l.work_id = i.work_id AND l.season = i.season)
              ORDER BY i.season",
        )?;
        let rows = stmt.query_map([from], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for (season, moved_version, origin) in seasons {
        let linked: i64 = tx.query_row(
            "SELECT count(*) FROM season_entries WHERE work_id = ?1 AND season = ?2",
            params![into, season],
            |r| r.get(0),
        )?;
        if linked > 0 {
            continue;
        }
        let kept_version: Option<i64> = tx
            .query_row(
                "SELECT version FROM season_info WHERE work_id = ?1 AND season = ?2",
                params![into, season],
                |r| r.get(0),
            )
            .optional()?;
        let version = kept_version.unwrap_or(0).max(moved_version) + 1;
        tx.execute(
            "INSERT INTO season_info (work_id, season, version, origin)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (work_id, season) DO UPDATE SET
                 version = excluded.version, origin = excluded.origin, job = NULL,
                 job_requested_at = NULL, job_attempts = 0, job_not_before = NULL,
                 note = NULL",
            params![into, season, version, origin],
        )?;
        tx.execute(
            "INSERT INTO season_entries (work_id, season, position, anilist_id)
             SELECT ?2, season, position, anilist_id FROM season_entries
              WHERE work_id = ?1 AND season = ?3",
            params![from, into, season],
        )?;
    }
    Ok(())
}

pub(super) fn set_links(
    conn: &mut Connection,
    work_id: &str,
    season: u32,
    expected: i64,
    ids: &[i64],
) -> Result<SeasonLink, SeasonError> {
    let mut seen = std::collections::HashSet::new();
    if !ids.iter().all(|id| *id > 0 && seen.insert(*id)) {
        return Err(SeasonError::Invalid("같은 항목을 두 번 이을 수 없어요."));
    }
    let tx = begin(conn)?;
    current(&tx, work_id, season, expected)?;
    bump(&tx, work_id, season, Origin::User, None)?;
    replace_entries(&tx, work_id, season, ids)?;
    let link = read_link(&tx, work_id, season)?;
    tx.commit()?;
    Ok(link)
}

pub(super) fn restart_auto(
    conn: &mut Connection,
    work_id: &str,
    season: u32,
    expected: i64,
    now: Millis,
) -> Result<SeasonLink, SeasonError> {
    let tx = begin(conn)?;
    current(&tx, work_id, season, expected)?;
    if first_season(&tx, work_id)? != Some(season) {
        return Err(SeasonError::Invalid(
            "자동으로 찾는 것은 가장 앞 시즌만 해요.",
        ));
    }
    bump(&tx, work_id, season, Origin::Auto, Some(now))?;
    replace_entries(&tx, work_id, season, &[])?;
    let link = read_link(&tx, work_id, season)?;
    tx.commit()?;
    Ok(link)
}

pub(super) fn next_search(
    conn: &mut Connection,
    now: Millis,
) -> rusqlite::Result<Option<ClaimedSearch>> {
    loop {
        let tx = begin(conn)?;
        let row = tx
            .query_row(
                "SELECT i.work_id, i.season, w.dir_name, i.version, i.job_attempts
                   FROM season_info i JOIN works w ON w.id = i.work_id
                   JOIN watch_folders f ON f.id = w.watch_folder_id AND f.unregistered_at IS NULL
                  WHERE i.job = 'search' AND (i.job_not_before IS NULL OR i.job_not_before <= ?1)
                  ORDER BY i.job_requested_at, i.work_id, i.season LIMIT 1",
                [now],
                |row| {
                    Ok(ClaimedSearch {
                        work_id: row.get(0)?,
                        season: row.get(1)?,
                        dir_name: row.get(2)?,
                        version: row.get(3)?,
                        attempts: row.get::<_, i64>(4)?.max(0) as u32,
                    })
                },
            )
            .optional()?;
        let Some(job) = row else {
            return Ok(None);
        };
        if first_season(&tx, &job.work_id)? == Some(job.season) {
            return Ok(Some(job));
        }
        // The season is not the first any more (or is gone): the search no
        // longer applies.
        tx.execute(
            "UPDATE season_info SET job = NULL, job_requested_at = NULL, job_attempts = 0,
                 job_not_before = NULL
              WHERE work_id = ?1 AND season = ?2",
            params![job.work_id, job.season],
        )?;
        tx.commit()?;
    }
}

/// Whether the season's search job still stands at `version`.
fn job_stands(
    tx: &Transaction<'_>,
    work_id: &str,
    season: u32,
    version: i64,
) -> rusqlite::Result<bool> {
    Ok(tx
        .query_row(
            "SELECT 1 FROM season_info
              WHERE work_id = ?1 AND season = ?2 AND version = ?3 AND job = 'search'",
            params![work_id, season, version],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        .is_some())
}

pub(super) fn auto_linked(
    conn: &mut Connection,
    work_id: &str,
    season: u32,
    version: i64,
    entry_id: i64,
) -> Result<bool, SeasonError> {
    let tx = begin(conn)?;
    if !job_stands(&tx, work_id, season, version)? {
        return Ok(false);
    }
    bump(&tx, work_id, season, Origin::Auto, None)?;
    replace_entries(&tx, work_id, season, &[entry_id])?;
    tx.commit()?;
    Ok(true)
}

pub(super) fn search_later(
    conn: &mut Connection,
    work_id: &str,
    season: u32,
    version: i64,
    retry_at: Option<Millis>,
    failed: bool,
    note: Note,
) -> rusqlite::Result<bool> {
    let tx = begin(conn)?;
    if !job_stands(&tx, work_id, season, version)? {
        return Ok(false);
    }
    match retry_at {
        Some(at) => {
            tx.execute(
                "UPDATE season_info SET job_not_before = ?3, job_attempts = job_attempts + ?4
                  WHERE work_id = ?1 AND season = ?2",
                params![work_id, season, at, i64::from(failed)],
            )?;
        }
        None => {
            tx.execute(
                "UPDATE season_info SET job = NULL, job_requested_at = NULL, job_attempts = 0,
                     job_not_before = NULL, note = ?3
                  WHERE work_id = ?1 AND season = ?2",
                params![work_id, season, note.code()],
            )?;
        }
    }
    tx.commit()?;
    Ok(true)
}

pub(super) fn next_refresh(conn: &Connection, now: Millis) -> rusqlite::Result<Option<i64>> {
    conn.query_row(
        "SELECT e.id FROM anilist_entries e
          WHERE e.status IN ('RELEASING', 'NOT_YET_RELEASED')
            AND e.fetched_at <= ?1 - ?2
            AND (e.refresh_not_before IS NULL OR e.refresh_not_before <= ?1)
            AND EXISTS (SELECT 1 FROM season_entries l
                          JOIN works w ON w.id = l.work_id
                          JOIN watch_folders f ON f.id = w.watch_folder_id
                         WHERE l.anilist_id = e.id AND f.unregistered_at IS NULL)
          ORDER BY e.fetched_at, e.id LIMIT 1",
        params![now, DAY_MS],
        |r| r.get(0),
    )
    .optional()
}

pub(super) fn refresh_later(conn: &Connection, id: i64, retry_at: Millis) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE anilist_entries SET refresh_not_before = ?2 WHERE id = ?1",
        params![id, retry_at],
    )?;
    Ok(())
}

pub(super) fn refresh_gone(conn: &Connection, id: i64, now: Millis) -> rusqlite::Result<()> {
    // Treated as received now: what is kept shows, and the next try is a day on.
    conn.execute(
        "UPDATE anilist_entries SET fetched_at = ?2, refresh_not_before = NULL WHERE id = ?1",
        params![id, now],
    )?;
    Ok(())
}

/// What the library list reads from one linked entry of a recorded season.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedFact {
    pub season: u32,
    pub position: u32,
    pub start_year: Option<i32>,
    pub status: Option<String>,
    /// Native, English and romaji titles, those that exist.
    pub titles: Vec<String>,
}

/// The linked entries of the recorded seasons, by work.
pub fn facts(conn: &Connection) -> rusqlite::Result<HashMap<String, Vec<LinkedFact>>> {
    let mut stmt = conn.prepare(
        "SELECT l.work_id, l.season, l.position, e.start_year, e.status,
                e.native, e.english, e.romaji
           FROM season_entries l
           JOIN anilist_entries e ON e.id = l.anilist_id
           JOIN seasons s ON s.work_id = l.work_id AND s.number = l.season
          ORDER BY l.work_id, l.season, l.position",
    )?;
    let mut facts: HashMap<String, Vec<LinkedFact>> = HashMap::new();
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let titles = (5..8)
            .filter_map(|i| row.get::<_, Option<String>>(i).transpose())
            .collect::<rusqlite::Result<Vec<String>>>()?;
        facts.entry(row.get(0)?).or_default().push(LinkedFact {
            season: row.get(1)?,
            position: row.get::<_, i64>(2)?.max(0) as u32,
            start_year: row.get(3)?,
            status: row.get(4)?,
            titles,
        });
    }
    Ok(facts)
}
