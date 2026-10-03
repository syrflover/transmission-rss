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
//! revision mark described at [`revision_of`], given what the subtitle jobs
//! received ([`Received`]).

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
    /// Set when a subtitle of the creator for the same episode text was
    /// received from an earlier observation: see [`revision_of`].
    pub revision: Option<Revision>,
}

impl Candidate {
    /// What the candidates are sorted by: the update time, or the time the
    /// state was first seen when the update time is not a date.
    pub fn sort_at(&self) -> Millis {
        self.updated_at.unwrap_or(self.first_seen_at)
    }
}

/// What a revision candidate revises: a subtitle the app received from an
/// earlier observation, or a subtitle file whose creator the user named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    /// The latest earlier observation of the creator with the same episode
    /// text whose subtitle was received. `None` when it revises a subtitle
    /// file of the library whose creator the user named: there is no
    /// observation of it.
    pub of: Option<i64>,
    /// Whether that observation had the same post address: the creator fixed
    /// the post (`Some(true)`) or posted the episode again elsewhere
    /// (`Some(false)`). `None` together with `of`: the post of a file the user
    /// named a creator for is not known.
    pub same_post: Option<bool>,
}

/// A subtitle file of the library whose creator the user named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attributed {
    /// The creator's source.
    pub source_id: String,
    /// The season episode the file is for, as written in its name.
    pub episode: String,
}

/// A subtitle a job received from an observation (`trss-jobs` records it; the
/// caller reads it there and hands it in).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received {
    pub source_id: String,
    /// The episode text as the observation wrote it.
    pub episode: String,
    pub observation_id: i64,
    pub post_url: String,
}

/// The key two episode texts of Anissia's lines share when they are one
/// episode, which the revision mark and the subscribed creator's receipts
/// (`trss-jobs`) compare by: `n:` and a decimal number without its leading
/// zeros and the trailing zeros of its decimal part (`013`, `13` and `13.0`
/// are `n:13`; `13.50` is `n:13.5`), the same number the subtitle sources use
/// (`trss_subtitles::episode::numeric_key`, which this crate does not see);
/// `t:` and the text as written for any other (`SP`). No float is made.
pub fn episode_key(text: &str) -> String {
    match numeric_episode(text) {
        Some(n) => format!("n:{n}"),
        None => format!("t:{text}"),
    }
}

/// The number part of [`episode_key`]; `None` for a text that is no number.
pub fn numeric_episode(text: &str) -> Option<String> {
    let (whole, decimal) = match text.split_once('.') {
        Some((whole, decimal)) => (whole, Some(decimal)),
        None => (text, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || decimal.is_some_and(|d| !digits(d)) {
        return None;
    }
    let integer = match whole.trim_start_matches('0') {
        "" => "0",
        integer => integer,
    };
    match decimal.map(|d| d.trim_end_matches('0')).unwrap_or_default() {
        "" => Some(integer.to_owned()),
        fraction => Some(format!("{integer}.{fraction}")),
    }
}

/// The revision mark of `candidate`, given the subtitles received of its
/// anime.
///
/// A candidate is a *revision candidate* when the app holds a subtitle of the
/// same creator for the same episode (`docs/specs/subtitles.md`, 자막 후보
/// 조회): a subtitle received from an earlier observation of the creator's
/// source with the same episode text. Two limits of what is known now:
///
/// - The episode is the text Anissia's line had, compared by
///   [`episode_key`] (`03` and `3` are one episode, `13.5` is not `13`); the
///   episode a received package really holds is decided when it is analysed,
///   which comes later.
/// - A subtitle the user put in without a creator, then gave one, counts
///   through [`revision_by_attribution`], which the caller applies to the
///   candidates this leaves unmarked.
///
/// An observation newer than the candidate is not what it revises, and the
/// candidate's own receipt is not either.
pub fn revision_of(candidate: &Candidate, received: &[Received]) -> Option<Revision> {
    received
        .iter()
        .filter(|r| {
            r.source_id == candidate.source_id
                && episode_key(&r.episode) == episode_key(&candidate.episode)
                && r.observation_id < candidate.id
        })
        .max_by_key(|r| r.observation_id)
        .map(|r| Revision {
            of: Some(r.observation_id),
            same_post: Some(r.post_url == candidate.post_url),
        })
}

/// The revision mark of `candidate` by the subtitle files of the library whose
/// creator the user named (`docs/specs/subtitles.md`, 자막 후보 조회): the
/// season holds a subtitle of the candidate's creator for the same episode.
///
/// `offset` is what the creator's source maps Anissia's whole episode to the
/// season's by (`0` while the source has no mapping, so the numbers are
/// compared as they are, as the work detail compares them everywhere else); a
/// source whose mapping is undecided has none, and the caller does not ask.
/// Under an offset other than `0` only a whole episode of Anissia's above `0`
/// can be mapped, and to an episode above `0`. The file's post is not known, so the mark carries no `of` and no
/// `same_post`.
pub fn revision_by_attribution(
    candidate: &Candidate,
    offset: i64,
    held: &[Attributed],
) -> Option<Revision> {
    // Anissia's episode `0` and a mapped number that is not above `0` are no
    // episode of the season (as the subscribed creator's receipt reads them),
    // so there is nothing to revise.
    let wanted = if offset == 0 {
        let key = episode_key(&candidate.episode);
        if key == "n:0" {
            return None;
        }
        key
    } else {
        let n: i64 = numeric_episode(&candidate.episode)?.parse().ok()?;
        let mapped = n.checked_add(offset)?;
        if n <= 0 || mapped <= 0 {
            return None;
        }
        format!("n:{mapped}")
    };
    held.iter()
        .any(|a| a.source_id == candidate.source_id && episode_key(&a.episode) == wanted)
        .then_some(Revision {
            of: None,
            same_post: None,
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
    /// The source of creator `creator` of anime `anime_no`, made at `at` when
    /// no line of the creator was observed yet (the user can name a creator
    /// Anissia lists before the first observation of its line, which then
    /// finds this source).
    pub async fn source_of_creator(
        &self,
        anime_no: i64,
        creator: String,
        at: Millis,
    ) -> Result<String> {
        self.db
            .run(move |c| {
                let line = Line {
                    anime_no,
                    creator,
                    episode: String::new(),
                    post_url: String::new(),
                    updated: String::new(),
                    updated_at: None,
                };
                Ok::<_, super::AnissiaStoreError>(source_of(c, &line, at)?)
            })
            .await
    }

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
    /// whose update time is not a date goes by the time it was first seen),
    /// marked as revisions of what `received` holds.
    pub async fn candidates(
        &self,
        anime_no: i64,
        received: Vec<Received>,
    ) -> Result<Vec<Candidate>> {
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
                let mut out = Vec::new();
                for row in rows {
                    let mut candidate = row?;
                    candidate.revision = revision_of(&candidate, &received);
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
