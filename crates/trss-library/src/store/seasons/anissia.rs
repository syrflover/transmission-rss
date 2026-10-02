//! A season's link to an Anissia anime (`docs/specs/library.md`, 작품 연결과
//! 제외): the table `season_anissia`, whose rules are in its migration.
//!
//! This store keeps the link and its version. It does not decide who may
//! change it: a season that a subscription is connected to holds that
//! subscription's anime, and the code that knows subscriptions (in
//! `trss-collect`) writes the link in its own transaction through [`set_in`],
//! after checking the version through [`link_in`].

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use super::{repo, SeasonError, SeasonStore};

/// A season's Anissia link. A season never linked has version 0 and no anime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnissiaLink {
    pub work_id: String,
    pub season: u32,
    /// 0 while the season never had a link; grows with every change.
    pub version: i64,
    /// Anissia's `animeNo`; `None` when nothing is linked.
    pub anime_no: Option<i64>,
}

/// The link of a season as stored, or the untouched one (version 0).
pub fn link_in(conn: &Connection, work_id: &str, season: u32) -> rusqlite::Result<AnissiaLink> {
    let stored: Option<(i64, Option<i64>)> = conn
        .query_row(
            "SELECT version, anime_no FROM season_anissia WHERE work_id = ?1 AND season = ?2",
            params![work_id, season],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let (version, anime_no) = stored.unwrap_or((0, None));
    Ok(AnissiaLink {
        work_id: work_id.to_owned(),
        season,
        version,
        anime_no,
    })
}

/// Makes `anime_no` (or nothing) the season's anime, and moves the version on
/// when that changes the link; the same link again changes nothing. The anime
/// needs its snapshot in `anissia_anime` already, and the work must exist.
/// Returns the link as it is now.
pub fn set_in(
    tx: &Transaction<'_>,
    work_id: &str,
    season: u32,
    anime_no: Option<i64>,
) -> rusqlite::Result<AnissiaLink> {
    let current = link_in(tx, work_id, season)?;
    if current.anime_no == anime_no {
        return Ok(current);
    }
    tx.execute(
        "INSERT INTO season_anissia (work_id, season, anime_no, version)
         VALUES (?1, ?2, ?3, 1)
         ON CONFLICT (work_id, season) DO UPDATE SET
             anime_no = excluded.anime_no, version = version + 1",
        params![work_id, season, anime_no],
    )?;
    link_in(tx, work_id, season)
}

/// Carries work `from`'s Anissia links over to work `into` when an archive
/// move merges `from` into `into` (`from`'s rows are about to go): each season
/// of `from` that links an anime, where `into`'s same season links none. The
/// version moves past both. Seasons `into` has linked keep their links.
pub(crate) fn merge_links(tx: &Transaction<'_>, from: &str, into: &str) -> rusqlite::Result<()> {
    let moved: Vec<(u32, i64, i64)> = {
        let mut stmt = tx.prepare(
            "SELECT season, anime_no, version FROM season_anissia
              WHERE work_id = ?1 AND anime_no IS NOT NULL ORDER BY season",
        )?;
        let rows = stmt.query_map([from], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    for (season, anime_no, moved_version) in moved {
        let kept = link_in(tx, into, season)?;
        if kept.anime_no.is_some() {
            continue;
        }
        tx.execute(
            "INSERT INTO season_anissia (work_id, season, anime_no, version)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (work_id, season) DO UPDATE SET
                 anime_no = excluded.anime_no, version = excluded.version",
            params![into, season, anime_no, kept.version.max(moved_version) + 1],
        )?;
    }
    Ok(())
}

impl SeasonStore {
    /// The Anissia link of a season; version 0 and no anime for a season that
    /// never had one. [`SeasonError::NotFound`] when the work has no such
    /// season recorded.
    pub async fn anissia_link(
        &self,
        work_id: &str,
        season: u32,
    ) -> Result<AnissiaLink, SeasonError> {
        let id = work_id.to_owned();
        self.db
            .run(move |c| {
                if !repo::season_exists(c, &id, season)? {
                    return Err(SeasonError::NotFound);
                }
                Ok(link_in(c, &id, season)?)
            })
            .await
    }

    /// The Anissia links of every season of a work that has a row, by season
    /// number (a link that was cut is in it, with no anime).
    pub async fn anissia_links_of(
        &self,
        work_id: &str,
    ) -> Result<BTreeMap<u32, AnissiaLink>, SeasonError> {
        let id = work_id.to_owned();
        self.db
            .run(move |c| {
                let mut stmt = c.prepare(
                    "SELECT season, version, anime_no FROM season_anissia WHERE work_id = ?1",
                )?;
                let rows = stmt.query_map([&id], |r| {
                    Ok(AnissiaLink {
                        work_id: id.clone(),
                        season: r.get(0)?,
                        version: r.get(1)?,
                        anime_no: r.get(2)?,
                    })
                })?;
                let links = rows.collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(links.into_iter().map(|l| (l.season, l)).collect())
            })
            .await
    }
}
