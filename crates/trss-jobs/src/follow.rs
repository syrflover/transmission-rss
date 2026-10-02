//! The subscribed creator's subtitles, received without a pick
//! (`docs/specs/subtitles.md`, 구독 제작자 자동 수신; ADR 0009), and the
//! `자막 구독` suggestion of a subscription whose creator is not chosen yet.
//!
//! # Which subscriptions
//!
//! A subscription takes part while its rule is `active` (`영상 받기` on; a
//! paused or archived rule does nothing), it gets subtitles (`follow` or
//! `undecided`; `none` does nothing), and its season (`season_id`) is linked
//! to the subscription's Anissia anime. A `follow` subscription's creator is
//! found among the anime's sources by Anissia's name
//! (`subtitle_sources.creator_name`). Picking in the work detail's 자막 후보
//! section works whatever the subscription says.
//!
//! # What is received
//!
//! For each episode of the subscribed creator (Anissia's text compared by the
//! app's key, [`episode_key`]: `03` and `3` are one episode), the newest
//! observation is looked at, the first row that applies:
//!
//! | the episode | what happens |
//! | --- | --- |
//! | a job received this observation | nothing |
//! | a job that has not failed holds an observation of the episode (pending, running, waiting, held) | nothing yet: it is looked at again after that job |
//! | a job received an earlier observation of the creator's episode | a **revision** job (`revision_of` that observation); the subtitle in place is untouched |
//! | the source's mapping to the season is undecided, or the episode is no whole number (`0`, `13.5`, `SP`) | nothing: whether the episode has a subtitle cannot be told |
//! | the mapped episode of the season has a subtitle file in the library | nothing: another creator's (or an unknown) subtitle is there, and a new creator's candidate is no revision of it |
//! | another creator's subtitle of the episode was received, or a job that has not failed is receiving it (by that source's mapping, or by the same Anissia episode when it has none) | nothing, for the same reason |
//! | otherwise | a **new episode** job |
//!
//! Each is one job of one candidate, made like a pick ([`JobStore::create`])
//! with origin [`AUTO`] and the request ID `auto:<observation id>`: a second
//! look at the same observation (the same line read twice, a repeated
//! evaluation, a restart) finds that job and makes none. A line whose update
//! time changed again is a new observation, so a new revision job. A job that
//! failed is not made again by itself; the user picks the candidate again.
//!
//! The source's mapping is decided first ([`mapping::decide`]), every time.
//! A work whose folder is gone (`missing`) receives nothing: what its
//! episodes hold is not known.
//!
//! # When
//!
//! [`Follow::evaluate`] looks at every subscription, and a subscription that
//! fails is logged and does not keep the others from theirs. The worker runs
//! it at its start, after a reading of Anissia's lines added observations,
//! after a run of jobs ended (a job that held an episode may have finished),
//! after its cycle connected a subscription to a season, and after the season
//! queue stored a season's AniList entry; the web after the subscribed creator
//! is set or changed, after `영상 받기` or `자막 받기` is turned on, and after a
//! season's AniList or Anissia link is saved. Whoever made jobs wakes the
//! runner.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{OptionalExtension, TransactionBehavior};
use serde_json::json;
use trss_collect::store::{
    anissia::{episode_key, numeric_episode, AnissiaStore, AnissiaStoreError, Candidate},
    channels::{ChannelError, ChannelStore, RuleState, SeasonRef, SubtitleMode},
};
use trss_core::{Db, DbError, Millis};
use trss_library::{
    seasons::combine::combine,
    store::{
        library::{Held, LibraryError, LibraryStore},
        seasons::{SeasonError, SeasonStore},
    },
};

use crate::{
    mapping::{self, Mapping, MappingKind},
    store::{JobError, JobStore, NewItem, NewJob, Pick, AUTO},
    Created, ItemState,
};

#[derive(Debug, thiserror::Error)]
pub enum FollowError {
    #[error(transparent)]
    Jobs(#[from] JobError),
    #[error("rules: {0}")]
    Channels(#[from] ChannelError),
    #[error("season info: {0}")]
    Seasons(#[from] SeasonError),
    #[error("library: {0}")]
    Library(#[from] LibraryError),
    #[error("observations: {0}")]
    Anissia(#[from] AnissiaStoreError),
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<rusqlite::Error> for FollowError {
    fn from(e: rusqlite::Error) -> Self {
        FollowError::Db(DbError::Sqlite(e))
    }
}

type Result<T> = std::result::Result<T, FollowError>;

/// A subscription that takes part (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscribed {
    pub rule_id: String,
    pub work_id: String,
    pub season: u32,
    pub anime_no: i64,
    /// The creator followed; `None` while it is not chosen (`제작자 미정`).
    pub creator: Option<String>,
    /// The rule's 회차 변환.
    pub rule_episode: i64,
}

/// A `자막 구독` suggestion: a work whose subscription has no creator yet and
/// whose anime has candidates. It names no creator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub work_id: String,
    /// The work's folder name.
    pub work_name: String,
    /// The anime's title in the stored schedule, if it is there.
    pub anime_title: Option<String>,
    pub season: u32,
    pub rule_id: String,
    pub anime_no: i64,
    /// The episodes the candidates are about, as Anissia writes them, without
    /// repeats.
    pub episodes: Vec<String>,
    /// How many creators have candidates.
    pub creators: usize,
    /// When the first candidate was seen.
    pub since: Millis,
}

/// What the request ID of an automatic job starts with: `auto:` and the
/// observation. A pick's ID never does.
pub const AUTO_PREFIX: &str = "auto:";

/// The episode key the app compares by, the candidates' revision mark's too
/// ([`episode_key`]).
fn key(text: &str) -> String {
    episode_key(text)
}

/// A whole episode number other than `0`.
fn whole(text: &str) -> Option<i64> {
    let n = numeric_episode(text)?;
    match n.parse::<i64>() {
        Ok(n) if n > 0 => Some(n),
        _ => None,
    }
}

/// What the newest observations of a creator's episodes come to.
struct Grounds<'a> {
    source_id: &'a str,
    /// The anime's observations, every creator's.
    observed: &'a [Candidate],
    /// The jobs' items of the anime.
    picks: &'a [Pick],
    /// The season's whole episodes that have a file.
    held: &'a BTreeMap<u32, Held>,
    /// The source's offset; `None` while undecided.
    offset: Option<i64>,
    /// The other sources' decided offsets.
    others: &'a HashMap<String, i64>,
}

/// The observations to receive, oldest first, each with the observation it
/// revises (see the module docs).
fn to_receive<'a>(g: &Grounds<'a>) -> Vec<(&'a Candidate, Option<i64>)> {
    let ours: Vec<&Candidate> = g
        .observed
        .iter()
        .filter(|c| c.source_id == g.source_id)
        .collect();
    let mut newest: BTreeMap<String, &Candidate> = BTreeMap::new();
    for c in &ours {
        let entry = newest.entry(key(&c.episode)).or_insert(c);
        if c.id > entry.id {
            *entry = c;
        }
    }
    let ours_done =
        |p: &&Pick| p.source_id.as_deref() == Some(g.source_id) && p.item_state == ItemState::Done;
    let mut out = Vec::new();
    for (k, o) in newest {
        let same: HashSet<i64> = ours
            .iter()
            .filter(|c| key(&c.episode) == k)
            .map(|c| c.id)
            .collect();
        let of_episode: Vec<&Pick> = g
            .picks
            .iter()
            .filter(|p| same.contains(&p.observation_id))
            .collect();
        if of_episode
            .iter()
            .any(|p| p.observation_id == o.id && p.item_state == ItemState::Done)
        {
            continue;
        }
        if of_episode
            .iter()
            .any(|p| !matches!(p.item_state, ItemState::Done | ItemState::Failed))
        {
            continue;
        }
        let received = g
            .picks
            .iter()
            .filter(ours_done)
            .filter(|p| key(&p.episode) == k && p.observation_id < o.id)
            .map(|p| p.observation_id)
            .max();
        if let Some(of) = received {
            out.push((o, Some(of)));
            continue;
        }
        let (Some(offset), Some(n)) = (g.offset, whole(&o.episode)) else {
            continue;
        };
        let video = n + offset;
        let has_file = u32::try_from(video)
            .ok()
            .and_then(|v| g.held.get(&v))
            .is_some_and(|h| h.subtitle);
        if video < 1 || has_file {
            continue;
        }
        // Another creator's subtitle of the episode, received or on its way.
        let another = g
            .picks
            .iter()
            .filter(|p| p.item_state != ItemState::Failed)
            .filter(|p| p.source_id.as_deref() != Some(g.source_id))
            .any(
                |p| match p.source_id.as_ref().and_then(|s| g.others.get(s)) {
                    Some(theirs) => whole(&p.episode).map(|m| m + theirs) == Some(video),
                    None => key(&p.episode) == k,
                },
            );
        if another {
            continue;
        }
        out.push((o, None));
    }
    out.sort_by_key(|(c, _)| c.id);
    out
}

/// The subscribed creator's receipts and the `자막 구독` suggestions. Cheap to
/// clone.
#[derive(Clone)]
pub struct Follow {
    db: Db,
    jobs: JobStore,
    channels: ChannelStore,
    seasons: SeasonStore,
    library: LibraryStore,
    anissia: AnissiaStore,
}

impl Follow {
    pub fn new(db: Db) -> Follow {
        Follow {
            jobs: JobStore::new(db.clone()),
            channels: ChannelStore::new(db.clone()),
            seasons: SeasonStore::new(db.clone()),
            library: LibraryStore::new(db.clone()),
            anissia: AnissiaStore::new(db.clone()),
            db,
        }
    }

    /// The subscriptions that take part, by work and season.
    pub async fn subscribed(&self) -> Result<Vec<Subscribed>> {
        let mut out = Vec::new();
        for channel in self.channels.list_channels_with_rules().await? {
            for rule in channel.rules {
                let Some(sub) = rule.subscription else {
                    continue;
                };
                if rule.state != RuleState::Active || sub.subtitles == SubtitleMode::None {
                    continue;
                }
                let Some(season) = sub.season_id.as_deref().and_then(SeasonRef::parse) else {
                    continue;
                };
                let link = match self
                    .seasons
                    .anissia_link(&season.work_id, season.number)
                    .await
                {
                    Ok(link) => link,
                    Err(SeasonError::NotFound) => continue,
                    Err(e) => return Err(e.into()),
                };
                if link.anime_no != Some(sub.anissia_anime_no) {
                    continue;
                }
                out.push(Subscribed {
                    rule_id: rule.id,
                    work_id: season.work_id,
                    season: season.number,
                    anime_no: sub.anissia_anime_no,
                    creator: match sub.subtitles {
                        SubtitleMode::Follow => sub.creator,
                        _ => None,
                    },
                    rule_episode: rule.episode,
                });
            }
        }
        out.sort_by(|a, b| (&a.work_id, a.season).cmp(&(&b.work_id, b.season)));
        Ok(out)
    }

    /// Makes the jobs of every subscribed creator's episodes to receive (see
    /// the module docs). Returns the jobs it made, oldest observation first.
    /// A subscription that fails is logged and the others go on.
    pub async fn evaluate(&self, now: Millis) -> Result<Vec<String>> {
        let mut made = Vec::new();
        for sub in self.subscribed().await? {
            let Some(creator) = sub.creator.clone() else {
                continue;
            };
            match self.follow(&sub, &creator, now).await {
                Ok(jobs) => made.extend(jobs),
                Err(e) => eprintln!(
                    "Subscribed creator of work {} season {} (rule {}): {e}",
                    sub.work_id, sub.season, sub.rule_id
                ),
            }
        }
        Ok(made)
    }

    async fn follow(&self, sub: &Subscribed, creator: &str, now: Millis) -> Result<Vec<String>> {
        let observed = self.anissia.candidates(sub.anime_no, Vec::new()).await?;
        let Some(source_id) = observed
            .iter()
            .find(|c| c.creator == creator)
            .map(|c| c.source_id.clone())
        else {
            return Ok(Vec::new());
        };
        // A work whose folder is gone holds nothing as far as the library
        // knows, which is not the same as holding no subtitle.
        let work_id = sub.work_id.clone();
        let missing = self
            .db
            .run(move |c| {
                Ok::<_, FollowError>(
                    c.query_row("SELECT missing FROM works WHERE id = ?1", [work_id], |r| {
                        r.get::<_, i64>(0)
                    })
                    .optional()?,
                )
            })
            .await?;
        if missing != Some(0) {
            return Ok(Vec::new());
        }
        let Some(held) = self
            .library
            .season_episodes(&sub.work_id, sub.season)
            .await?
        else {
            return Ok(Vec::new());
        };
        let count = self
            .seasons
            .links_of_seasons(vec![(sub.work_id.clone(), sub.season)])
            .await?
            .into_iter()
            .next()
            .flatten()
            .filter(|link| !link.entries.is_empty())
            .and_then(|link| combine(&link.entries).and_then(|c| c.episodes));
        let mut numbers: Vec<u32> = observed
            .iter()
            .filter(|c| c.source_id == source_id)
            .filter_map(|c| whole(&c.episode).and_then(|n| u32::try_from(n).ok()))
            .collect();
        numbers.sort_unstable();
        numbers.dedup();
        let decided = mapping::decide(&numbers, count, sub.rule_episode, sub.season);

        let mappings = {
            let (work_id, season, source) = (sub.work_id.clone(), sub.season, source_id.clone());
            self.db
                .run(move |c| {
                    // Written before anyone else reads: a mapping the user sets
                    // meanwhile is not overwritten with the app's.
                    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                    mapping::store_in(&tx, &work_id, season, &source, &decided, now)?;
                    let all = mapping::read_in(&tx, &work_id, season)?;
                    tx.commit()?;
                    Ok::<_, FollowError>(all)
                })
                .await?
        };
        let decided_offset = |m: &Mapping| match m.kind {
            MappingKind::Undecided => None,
            MappingKind::Auto | MappingKind::User => m.offset,
        };
        let offset = mappings.get(&source_id).and_then(decided_offset);
        let others: HashMap<String, i64> = mappings
            .iter()
            .filter(|(s, _)| **s != source_id)
            .filter_map(|(s, m)| decided_offset(m).map(|o| (s.clone(), o)))
            .collect();

        let picks = self.jobs.picks_of_anime(sub.anime_no).await?;
        let grounds = Grounds {
            source_id: &source_id,
            observed: &observed,
            picks: &picks,
            held: &held,
            offset,
            others: &others,
        };
        let mut made = Vec::new();
        for (candidate, revision_of) in to_receive(&grounds) {
            let request = json!({
                "auto": candidate.id,
                "work_id": sub.work_id,
                "season": sub.season,
                "revision_of": revision_of,
            })
            .to_string();
            let job = NewJob {
                command_id: format!("{AUTO_PREFIX}{}", candidate.id),
                request,
                origin: AUTO.to_owned(),
                work_id: Some(sub.work_id.clone()),
                season: Some(i64::from(sub.season)),
                anime_no: Some(sub.anime_no),
                source_id: Some(source_id.clone()),
                creator: Some(candidate.creator.clone()),
                revision_of,
                items: vec![NewItem {
                    observation_id: Some(candidate.id),
                    episode: candidate.episode.clone(),
                    post_url: candidate.post_url.clone(),
                    found_at: candidate.first_seen_at,
                }],
            };
            if let Created::Created(id) = self.jobs.create(job, now).await? {
                made.push(id);
            }
        }
        Ok(made)
    }

    /// The `자막 구독` suggestions: one per work whose subscription has no
    /// creator yet and whose anime has candidates, the lowest season first.
    pub async fn suggestions(&self) -> Result<Vec<Suggestion>> {
        let mut out: Vec<Suggestion> = Vec::new();
        for sub in self.subscribed().await? {
            if sub.creator.is_some() || out.iter().any(|s| s.work_id == sub.work_id) {
                continue;
            }
            let mut observed = self.anissia.candidates(sub.anime_no, Vec::new()).await?;
            if observed.is_empty() {
                continue;
            }
            observed.sort_by_key(|c| c.id);
            let mut episodes: Vec<String> = Vec::new();
            let mut keys = HashSet::new();
            for c in &observed {
                if keys.insert(key(&c.episode)) {
                    episodes.push(c.episode.clone());
                }
            }
            let creators: HashSet<&str> = observed.iter().map(|c| c.source_id.as_str()).collect();
            let (work_id, anime_no) = (sub.work_id.clone(), sub.anime_no);
            let (work_name, anime_title) = self
                .db
                .run(move |c| {
                    Ok::<_, FollowError>(c.query_row(
                        "SELECT w.dir_name,
                                (SELECT a.subject FROM anissia_anime a WHERE a.anime_no = ?2)
                         FROM works w WHERE w.id = ?1",
                        rusqlite::params![work_id, anime_no],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
                    )?)
                })
                .await?;
            out.push(Suggestion {
                work_id: sub.work_id.clone(),
                work_name,
                anime_title,
                season: sub.season,
                rule_id: sub.rule_id.clone(),
                anime_no: sub.anime_no,
                episodes,
                creators: creators.len(),
                since: observed.iter().map(|c| c.first_seen_at).min().unwrap_or(0),
            });
        }
        Ok(out)
    }

    /// The mappings of season `season` of the work, by source.
    pub async fn mappings(&self, work_id: &str, season: u32) -> Result<HashMap<String, Mapping>> {
        let work_id = work_id.to_owned();
        self.db
            .run(move |c| Ok::<_, FollowError>(mapping::read_in(c, &work_id, season)?))
            .await
    }
}
