//! The observations of Anissia's subtitle lines and the sources they belong to
//! (`docs/specs/subtitles.md`, 자막 후보 조회; the tables are in the migration
//! `anissia/captions.sql`).
//!
//! A *line* is what Anissia keeps for one creator of one anime: the last
//! episode text, post address and update time the creator entered. An
//! *observation* is a state of a line the app saw. [`AnissiaStore::observe`]
//! adds one when a line differs from the creator's previous observation, so a
//! creator who moves from `3화` to `4화`, or fixes a post, leaves both states
//! behind, and nothing deletes an observation (a line that vanishes from
//! Anissia's list leaves its observations as they were).
//!
//! A creator's lines of one anime are tied together by a *source* with an ID of
//! the app (`자막 출처 연결`). The creator's display name finds the source within
//! the anime; it is not an ID, and neither is a post's address.
//!
//! [`AnissiaStore::candidates`] reads an anime's observations back with the
//! revision mark described at [`revision_of`].

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use uuid::Uuid;

use super::{AnissiaStore, Result};
use trss_core::Millis;

/// A line as the worker read it, ready to be compared with the last
/// observation of its creator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub anime_no: i64,
    /// The creator's display name, as Anissia gives it.
    pub creator: String,
    /// The episode text as written.
    pub episode: String,
    /// The post's `http(s)` address.
    pub post_url: String,
    /// `updDt` as written.
    pub updated: String,
    /// The moment `updated` names (Unix ms), `None` when it is not a date.
    pub updated_at: Option<Millis>,
}

/// What [`AnissiaStore::observe`] did with the lines it was given.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Observed {
    /// Lines that became observations: new, or different from the previous one.
    pub added: usize,
    /// Lines the creator's previous observation already says.
    pub unchanged: usize,
}

/// One observation of an anime, with what the app knows of its place among the
/// creator's others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Grows with the time of observation.
    pub id: i64,
    /// The app's ID of the creator's source for the anime.
    pub source_id: String,
    pub creator: String,
    pub post_url: String,
    pub episode: String,
    /// `updDt` as written.
    pub updated: String,
    /// The moment `updated` names (Unix ms); `None` when it is not a date and
    /// time: a failed reading.
    pub updated_at: Option<Millis>,
    /// When the app first saw this state (Unix ms).
    pub first_seen_at: Millis,
    /// Set when the creator was observed with the same episode text before.
    pub revision: Option<Revision>,
}

impl Candidate {
    /// What the candidates are sorted by: the update time, or the time the
    /// state was first seen when the update time is not a date.
    pub fn sort_at(&self) -> Millis {
        self.updated_at.unwrap_or(self.first_seen_at)
    }
}

/// The earlier observation a candidate revises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    /// The creator's latest earlier observation with the same episode text.
    pub of: i64,
    /// Whether that observation had the same post address: the creator fixed
    /// the post (`true`) or posted the episode again elsewhere (`false`).
    pub same_post: bool,
}

/// The revision mark of an observation, given the creator's earlier ones.
///
/// **What is decided today.** A candidate is a *revision candidate* when the
/// app already holds a subtitle of the same creator for the same episode. The
/// app does not hold subtitles yet (the receiving and the archive come with the
/// subtitle jobs), so the closest rule it can state is on what it knows: the
/// creator was observed with the same episode text before. That counts a
/// post the user never received, so it is the mark a revision *could* be, and
/// it is the single place to change once received subtitles are recorded.
fn revision_of(
    earlier: &HashMap<String, (i64, String)>,
    episode: &str,
    post_url: &str,
) -> Option<Revision> {
    earlier.get(episode).map(|(of, post)| Revision {
        of: *of,
        same_post: post == post_url,
    })
}

/// Whether an observation says what `line` says. The times are compared as
/// moments when both are, so the same moment written another way is no change;
/// otherwise as written.
fn says(
    episode: &str,
    post_url: &str,
    updated: &str,
    updated_at: Option<Millis>,
    line: &Line,
) -> bool {
    episode == line.episode
        && post_url == line.post_url
        && match (updated_at, line.updated_at) {
            (Some(a), Some(b)) => a == b,
            _ => updated == line.updated,
        }
}

fn source_of(conn: &Connection, line: &Line, at: Millis) -> rusqlite::Result<String> {
    conn.execute(
        "INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT (anime_no, creator_name) DO NOTHING",
        params![Uuid::new_v4().to_string(), line.anime_no, line.creator, at],
    )?;
    conn.query_row(
        "SELECT id FROM subtitle_sources WHERE anime_no = ?1 AND creator_name = ?2",
        params![line.anime_no, line.creator],
        |r| r.get(0),
    )
}

impl AnissiaStore {
    /// Records `lines` seen at `at`, each as an observation unless the
    /// creator's previous observation of the anime already says the same. One
    /// transaction: a page of lines is kept whole or not at all.
    pub async fn observe(&self, lines: Vec<Line>, at: Millis) -> Result<Observed> {
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let mut done = Observed::default();
                for line in &lines {
                    let source = source_of(&tx, line, at)?;
                    let last: Option<(String, String, String, Option<Millis>)> = tx
                        .query_row(
                            "SELECT episode, post_url, updated, updated_at
                               FROM caption_observations
                              WHERE source_id = ?1 ORDER BY id DESC LIMIT 1",
                            [&source],
                            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                        )
                        .optional()?;
                    if last.is_some_and(|(episode, post, updated, updated_at)| {
                        says(&episode, &post, &updated, updated_at, line)
                    }) {
                        done.unchanged += 1;
                        continue;
                    }
                    tx.execute(
                        "INSERT INTO caption_observations
                             (source_id, post_url, episode, updated, updated_at, first_seen_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            source,
                            line.post_url,
                            line.episode,
                            line.updated,
                            line.updated_at,
                            at
                        ],
                    )?;
                    done.added += 1;
                }
                tx.commit()?;
                Ok::<_, super::AnissiaStoreError>(done)
            })
            .await
    }

    /// The observations of anime `anime_no`, newest update first (a state
    /// whose update time is not a date goes by the time it was first seen).
    pub async fn candidates(&self, anime_no: i64) -> Result<Vec<Candidate>> {
        self.db
            .run(move |c| {
                let mut stmt = c.prepare(
                    "SELECT o.id, o.source_id, s.creator_name, o.post_url, o.episode,
                            o.updated, o.updated_at, o.first_seen_at
                       FROM caption_observations o
                       JOIN subtitle_sources s ON s.id = o.source_id
                      WHERE s.anime_no = ?1
                      ORDER BY o.id",
                )?;
                let rows = stmt.query_map([anime_no], |r| {
                    Ok(Candidate {
                        id: r.get(0)?,
                        source_id: r.get(1)?,
                        creator: r.get(2)?,
                        post_url: r.get(3)?,
                        episode: r.get(4)?,
                        updated: r.get(5)?,
                        updated_at: r.get(6)?,
                        first_seen_at: r.get(7)?,
                        revision: None,
                    })
                })?;
                // Oldest first, so what each creator was observed with before is known.
                let mut seen: HashMap<String, HashMap<String, (i64, String)>> = HashMap::new();
                let mut out = Vec::new();
                for row in rows {
                    let mut candidate = row?;
                    let earlier = seen.entry(candidate.source_id.clone()).or_default();
                    candidate.revision =
                        revision_of(earlier, &candidate.episode, &candidate.post_url);
                    earlier.insert(
                        candidate.episode.clone(),
                        (candidate.id, candidate.post_url.clone()),
                    );
                    out.push(candidate);
                }
                out.sort_by(|a, b| b.sort_at().cmp(&a.sort_at()).then(b.id.cmp(&a.id)));
                Ok::<_, super::AnissiaStoreError>(out)
            })
            .await
    }

    /// When the reading of the recent list is due next (Unix ms), and when it
    /// last read every page; `None` before the first reading was ever begun.
    pub async fn caption_poll(&self) -> Result<Option<(Millis, Option<Millis>)>> {
        self.db
            .run(|c| {
                Ok::<_, super::AnissiaStoreError>(
                    c.query_row(
                        "SELECT next_at, last_read_at FROM anissia_caption_poll WHERE id = 1",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()?,
                )
            })
            .await
    }

    /// Makes the next reading of the recent list due at `next_at`, and, when
    /// `read_at` is given, records that every page was read at that time.
    pub async fn schedule_caption_poll(
        &self,
        next_at: Millis,
        read_at: Option<Millis>,
    ) -> Result<()> {
        self.db
            .run(move |c| {
                c.execute(
                    "INSERT INTO anissia_caption_poll (id, next_at, last_read_at)
                     VALUES (1, ?1, ?2)
                     ON CONFLICT (id) DO UPDATE SET
                         next_at = excluded.next_at,
                         last_read_at = coalesce(excluded.last_read_at, last_read_at)",
                    params![next_at, read_at],
                )?;
                Ok::<_, super::AnissiaStoreError>(())
            })
            .await
    }
}
