//! A movie's link to an Anissia anime (`docs/specs/library.md`, 작품 연결과
//! 제외; `docs/adr/0018-movies-in-season-zero-with-their-own-link.md`): the
//! table `movie_anissia`, whose rules are in its migration.
//!
//! A movie is an episode of season 0, and has an Anissia anime of its own
//! beside the one its season may be linked to ([`super::anissia`]). The work
//! that has only a movie is a **movie-only work**: its one video is directly in
//! the work folder, which the scan records as season 0, episode `1`
//! ([`crate::discovery::MOVIE_SEASON`], [`crate::discovery::MOVIE_EPISODE`]).
//! Nothing is stored for that: a work is movie-only while a video of season 0
//! is directly in its folder and it has no season from 1 on ([`MOVIE_ONLY`]).
//! A scan never records both, but an archive merge can copy a movie-only
//! work's root video into a work with seasons until the next scan.
//!
//! The link works as the season's does: a `version` that goes up with every
//! change, 0 while the movie never had a row, and a change made from an older
//! version changes nothing. The link outlives the episode's row (a scan that
//! misses the video for a moment does not lose what the user linked) and goes
//! with the work.

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use trss_core::episode::EpisodeNumber;

use super::{SeasonError, SeasonStore};

/// An SQL condition that is true for a work `w` (a row of `works`) that is
/// movie-only: a video of season 0 directly in its folder, and no season from 1
/// on. The one place the condition is spelled, for the queries that need it.
pub const MOVIE_ONLY: &str = movie_only!();

/// A movie's Anissia link. A movie never linked has version 0 and no anime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovieLink {
    pub work_id: String,
    /// The episode of season 0 that is the movie, a whole number from 1.
    pub episode: u32,
    /// 0 while the movie never had a link; grows with every change.
    pub version: i64,
    /// Anissia's `animeNo`; `None` when nothing is linked.
    pub anime_no: Option<i64>,
}

/// Whether the work is movie-only ([`MOVIE_ONLY`]).
pub fn is_movie_only(conn: &Connection, work_id: &str) -> rusqlite::Result<bool> {
    conn.prepare_cached(&format!("SELECT {MOVIE_ONLY} FROM works w WHERE w.id = ?1"))?
        .query_row([work_id], |r| r.get(0))
        .optional()
        .map(|found| found.unwrap_or(false))
}

/// Whether season 0 of a work in the library (one whose watch folder is
/// registered) has episode `episode` (compared as numbers: `01` is `1`).
/// Episode 0 is never a movie: a link needs an episode from 1.
pub(super) fn episode_exists(
    conn: &Connection,
    work_id: &str,
    episode: u32,
) -> rusqlite::Result<bool> {
    if episode == 0 {
        return Ok(false);
    }
    let mut stmt = conn.prepare_cached(
        "SELECT e.episode FROM episodes e
           JOIN works w ON w.id = e.work_id
           JOIN watch_folders f ON f.id = w.watch_folder_id AND f.unregistered_at IS NULL
          WHERE e.work_id = ?1 AND e.season = 0",
    )?;
    let key = episode.to_string();
    let rows = stmt.query_map([work_id], |r| r.get::<_, String>(0))?;
    for row in rows {
        if EpisodeNumber::parse(&row?).is_some_and(|n| n.is_key(&key)) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The link of a movie as stored, or the untouched one (version 0).
pub fn link_in(conn: &Connection, work_id: &str, episode: u32) -> rusqlite::Result<MovieLink> {
    let stored: Option<(i64, Option<i64>)> = conn
        .prepare_cached(
            "SELECT version, anime_no FROM movie_anissia WHERE work_id = ?1 AND episode = ?2",
        )?
        .query_row(params![work_id, episode], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    let (version, anime_no) = stored.unwrap_or((0, None));
    Ok(MovieLink {
        work_id: work_id.to_owned(),
        episode,
        version,
        anime_no,
    })
}

/// Makes `anime_no` (or nothing) the movie's anime, and moves the version on
/// when that changes the link; the same link again changes nothing. The anime
/// needs its snapshot in `anissia_anime` already, and the work must exist.
/// Returns the link as it is now.
pub fn set_in(
    tx: &Transaction<'_>,
    work_id: &str,
    episode: u32,
    anime_no: Option<i64>,
) -> rusqlite::Result<MovieLink> {
    let current = link_in(tx, work_id, episode)?;
    if current.anime_no == anime_no {
        return Ok(current);
    }
    tx.prepare_cached(
        "INSERT INTO movie_anissia (work_id, episode, anime_no, version)
         VALUES (?1, ?2, ?3, 1)
         ON CONFLICT (work_id, episode) DO UPDATE SET
             anime_no = excluded.anime_no, version = version + 1",
    )?
    .execute(params![work_id, episode, anime_no])?;
    link_in(tx, work_id, episode)
}

/// Carries work `from`'s movie links over to work `into` when an archive move
/// merges `from` into `into` (`from`'s rows are about to go), as the season
/// links are carried ([`super::anissia::merge_links`]): each movie of `from`
/// that links an anime, where `into` has no row for the same episode. The
/// version moves past both. An episode `into` has a row for keeps it as it is,
/// a cut included.
pub(crate) fn merge_links(tx: &Transaction<'_>, from: &str, into: &str) -> rusqlite::Result<()> {
    let moved: Vec<(u32, i64, i64)> = {
        let mut stmt = tx.prepare_cached(
            "SELECT episode, anime_no, version FROM movie_anissia
              WHERE work_id = ?1 AND anime_no IS NOT NULL ORDER BY episode",
        )?;
        let rows = stmt.query_map([from], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for (episode, anime_no, moved_version) in moved {
        if link_in(tx, into, episode)?.version > 0 {
            continue;
        }
        tx.prepare_cached(
            "INSERT INTO movie_anissia (work_id, episode, anime_no, version)
             VALUES (?1, ?2, ?3, ?4)",
        )?
        .execute(params![into, episode, anime_no, moved_version + 1])?;
    }
    Ok(())
}

impl SeasonStore {
    /// The Anissia link of the movie that is episode `episode` of the work's
    /// season 0; version 0 and no anime for a movie that never had one.
    /// [`SeasonError::NotFound`] when the work has no such episode recorded.
    pub async fn movie_link(&self, work_id: &str, episode: u32) -> Result<MovieLink, SeasonError> {
        let id = work_id.to_owned();
        self.db
            .run(move |c| {
                if !episode_exists(c, &id, episode)? {
                    return Err(SeasonError::NotFound);
                }
                Ok(link_in(c, &id, episode)?)
            })
            .await
    }

    /// The Anissia links of every movie of a work that has a row, by episode
    /// (a link that was cut is in it, with no anime).
    pub async fn movie_links_of(
        &self,
        work_id: &str,
    ) -> Result<BTreeMap<u32, MovieLink>, SeasonError> {
        let id = work_id.to_owned();
        self.db
            .run(move |c| {
                let mut stmt = c.prepare_cached(
                    "SELECT episode, version, anime_no FROM movie_anissia WHERE work_id = ?1",
                )?;
                let rows = stmt.query_map([&id], |r| {
                    Ok(MovieLink {
                        work_id: id.clone(),
                        episode: r.get(0)?,
                        version: r.get(1)?,
                        anime_no: r.get(2)?,
                    })
                })?;
                let links = rows.collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(links.into_iter().map(|l| (l.episode, l)).collect())
            })
            .await
    }
}
