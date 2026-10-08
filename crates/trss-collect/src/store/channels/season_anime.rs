//! A season's Anissia anime, changed from the work detail (`docs/specs/
//! library.md`, 작품 연결과 제외).
//!
//! The link itself is the season's (`trss_library::store::seasons::anissia`).
//! A subscription that is connected to a season (`season_id`) holds the
//! subscription's anime there: [`ChannelStore::link_season`] writes the link in
//! the transaction that connects the rule, and the work detail may not change
//! the link of a season a subscription holds, whichever anime it names and
//! whatever state the rule is in. The check and the write are one transaction,
//! so a rule that connects meanwhile either comes first (the change is refused)
//! or finds the link the change made (and is noted as blocked, or connects
//! when it is the same anime).
//!
//! The anime's snapshot is stored with the link, so the link can show its name.
//! An anime some rule subscribes to keeps the snapshot the worker refreshes;
//! the schedule's snapshot of it is not replaced by a search result.

use rusqlite::{params, OptionalExtension, TransactionBehavior};
use trss_library::store::seasons::anissia::{self as season_anissia, AnissiaLink};

use crate::store::{anissia, channels::ChannelStore};
use trss_anissia::Anime;
use trss_core::DbError;

/// Why a season's Anissia anime was not changed. Nothing was changed.
#[derive(Debug, thiserror::Error)]
pub enum SeasonAnimeError {
    #[error(transparent)]
    Db(#[from] DbError),
    /// The library has no such work.
    #[error("no such work")]
    NoWork,
    /// A subscription is connected to the season: its anime is the season's,
    /// and it changes only after the subscription is deleted (the link stays
    /// then).
    #[error("a subscription holds the season")]
    Subscribed {
        /// A rule connected to the season, the first by ID.
        rule_id: String,
        anime_no: i64,
    },
    /// The link is not at the version the change was made from. Carries the
    /// current link.
    #[error("the season's Anissia link was changed by someone else")]
    Conflict(Box<AnissiaLink>),
}

impl From<rusqlite::Error> for SeasonAnimeError {
    fn from(e: rusqlite::Error) -> Self {
        SeasonAnimeError::Db(DbError::Sqlite(e))
    }
}

impl ChannelStore {
    /// Makes `anime` (or, with `None`, nothing) the Anissia anime of season
    /// `season` of work `work_id`, as a change made from the link's `expected`
    /// version. `anime` is stored as the anime's snapshot.
    ///
    /// Refused with [`SeasonAnimeError::Subscribed`] when a subscription is
    /// connected to the season, and with [`SeasonAnimeError::Conflict`] when
    /// the link has moved on. The caller checks that the season is one the
    /// work has.
    pub async fn set_season_anime(
        &self,
        work_id: &str,
        season: u32,
        expected: i64,
        anime: Option<Anime>,
    ) -> Result<AnissiaLink, SeasonAnimeError> {
        let work_id = work_id.to_owned();
        self.db
            .run(move |conn| {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let work_exists: bool = tx
                    .prepare_cached("SELECT EXISTS (SELECT 1 FROM works WHERE id = ?1)")?
                    .query_row([&work_id], |r| r.get(0))?;
                if !work_exists {
                    return Err(SeasonAnimeError::NoWork);
                }
                let held = tx
                    .prepare_cached(
                        "SELECT rule_id, anissia_anime_no FROM rule_subscriptions
                          WHERE season_id = ?1 ORDER BY rule_id LIMIT 1",
                    )?
                    .query_row([format!("{work_id}:{season}")], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                    })
                    .optional()?;
                if let Some((rule_id, anime_no)) = held {
                    return Err(SeasonAnimeError::Subscribed { rule_id, anime_no });
                }
                let current = season_anissia::link_in(&tx, &work_id, season)?;
                if current.version != expected {
                    return Err(SeasonAnimeError::Conflict(Box::new(current)));
                }
                if let Some(anime) = &anime {
                    keep_snapshot(&tx, anime)?;
                }
                let link =
                    season_anissia::set_in(&tx, &work_id, season, anime.map(|a| a.anime_no))?;
                tx.commit()?;
                Ok(link)
            })
            .await
    }
}

/// Stores `anime` as its snapshot, unless a rule subscribes to the anime: the
/// worker keeps that one, from the schedule.
fn keep_snapshot(tx: &rusqlite::Transaction<'_>, anime: &Anime) -> rusqlite::Result<()> {
    let subscribed: bool = tx
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM rule_subscriptions s
                          JOIN anissia_anime a ON a.anime_no = s.anissia_anime_no
                         WHERE s.anissia_anime_no = ?1)",
        )?
        .query_row(params![anime.anime_no], |r| r.get(0))?;
    if subscribed {
        return Ok(());
    }
    anissia::upsert_in(tx, anime)
}
