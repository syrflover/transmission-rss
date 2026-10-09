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
//! | the episode is a conflict of the source's mapping ([`mapping::conflicts`]: a decimal or text such as `13.5`, a number outside the season's episodes once mapped, or, for an `auto` mapping, one whose own air time points at another offset; an episode the user's exception covers is none) | nothing: the other episodes of the source are received as usual, and the conflict is recorded for the user (`subtitle_mapping_conflicts`) |
//! | the user's exception says the episode is not received | nothing |
//! | the source's mapping to the season is undecided, or the episode is no whole number (`0`, `13.5`, `SP`) and no exception of the user's covers it | nothing: whether the episode has a subtitle cannot be told |
//! | the mapped episode of the season (the exception's target, else the episode plus the offset) has a subtitle file in the library whose creator the user named as this creator, and the app first saw this observation after that (`creator_set_at` of the file; the earliest of such files counts) | a **revision** job of a subtitle the user attributed (`revises_attributed`, `revision_of` unset: nothing of it was received); the subtitle in place is untouched |
//! | the mapped episode of the season (the exception's target, else the episode plus the offset) has any other subtitle file in the library | nothing: another creator's (or an unknown) subtitle is there, and a new creator's candidate is no revision of it. A line of this creator seen before the user named the file stays a revision candidate on screen (수정) only |
//! | another creator's subtitle of the episode was received, or a job that has not failed is receiving it (by that source's mapping, or by the same Anissia episode when it has none) | nothing, for the same reason |
//! | otherwise | a **new episode** job |
//!
//! The conflict row applies to every path below it; the revision of a received
//! episode, above it, is of what was received whatever the mapping now says.
//! The rows from the mapping on apply the same checks to an
//! attributed revision: the mapping is decided and the episode is a whole
//! number, and no other creator's subtitle of the episode was received or is on
//! its way. The
//! bytes are not compared with the subtitle in the library: the job only
//! receives, and the replacement comparison (`교체 비교와 승인`) does not exist
//! yet, which is where "nothing to replace" would be recorded.
//!
//! Each is one job of one candidate, made like a pick ([`JobRequests::create`])
//! with origin [`AUTO`] and the request ID `auto:<observation id>`: a second
//! look at the same observation (the same line read twice, a repeated
//! evaluation, a restart) finds that job and makes none. A line whose update
//! time changed again is a new observation, so a new revision job. A job that
//! failed is not made again by itself; the user picks the candidate again.
//!
//! The source's mapping is decided first ([`mapping::decide`], from the times
//! Anissia wrote on the creator's episodes and the season's AniList schedule),
//! every time, and the source's conflicts are rewritten with it in one
//! transaction. A work whose folder is gone (`missing`) receives nothing: what
//! its episodes hold is not known.
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
    anissia::{episode_key, AnissiaStore, AnissiaStoreError, Candidate},
    channels::{ChannelError, ChannelStore, RuleState, SeasonRef, SubtitleMode},
};
use trss_core::{episode::EpisodeNumber, Db, DbError, Millis};
use trss_library::{
    seasons::combine::{combine, schedule_times},
    store::{
        library::{AttributedSubtitle, Held, LibraryError, LibraryStore},
        seasons::{SeasonError, SeasonStore},
    },
};

use crate::{
    mapping::{self, whole, Mapped, Mapping},
    store::{JobError, JobRequests, JobViews, MappingStamp, NewItem, NewJob, Pick, AUTO},
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

/// What the library knows of a subscription's season for its mapping.
struct SeasonGround {
    /// The air time (Unix ms) of each season episode, from every linked entry.
    schedule: BTreeMap<u32, i64>,
    /// The AniList episode count of the linked entries together.
    count: Option<u32>,
    /// The episodes of all earlier seasons together, when each is known.
    previous: Option<u32>,
}

impl SeasonGround {
    fn season(&self, number: u32) -> mapping::Season<'_> {
        mapping::Season {
            number,
            schedule: &self.schedule,
            count: self.count,
            previous: self.previous,
        }
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
    /// The source's mapping, when it has one (decided or not).
    mapping: Option<&'a Mapping>,
    /// The other sources' decided mappings.
    others: &'a HashMap<String, Mapping>,
    /// The season's subtitle files whose creator the user named.
    attributed: &'a [AttributedSubtitle],
    /// The keys of the source's episodes that do not fit its mapping.
    conflicted: &'a HashSet<String>,
}

/// An observation to receive and what it is a revision of.
#[derive(Debug, Clone, Copy)]
struct Receipt<'a> {
    candidate: &'a Candidate,
    /// The observation of the creator's episode received before.
    revision_of: Option<i64>,
    /// It revises a subtitle file the user gave this creator.
    revises_attributed: bool,
}

/// The observations to receive, oldest first, each with what it revises (see
/// the module docs).
fn to_receive<'a>(g: &Grounds<'a>) -> Vec<Receipt<'a>> {
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
        // The user's `받지 않음` is the user's explicit word: it stops the
        // episode's revisions too, which an app-found conflict does not.
        if g.mapping
            .and_then(|m| m.exception_of(&o.episode))
            .is_some_and(|e| e.target.is_none())
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
            out.push(Receipt {
                candidate: o,
                revision_of: Some(of),
                revises_attributed: false,
            });
            continue;
        }
        // A received episode's revision is of what was received, whatever the
        // mapping now says; from here on the episode must fit the mapping.
        if g.conflicted.contains(&k) {
            continue;
        }
        // The user's exception for the episode comes before the offset.
        let Some(Mapped::Episode(video)) = g.mapping.map(|m| m.season_episode(&o.episode)) else {
            continue;
        };
        let has_file = u32::try_from(video)
            .ok()
            .and_then(|v| g.held.get(&v))
            .is_some_and(|h| h.subtitle);
        if video < 1 {
            continue;
        }
        // A subtitle file is there: the line revises it only when the user gave
        // the file to this very creator and the line is newer than that: first
        // seen after it and, when Anissia's time reads, written after it. A
        // line seen before stays a revision candidate on screen only, and so
        // does one the app first sees late but that Anissia shows was there
        // already (a creator named before any line of theirs was observed).
        let revises_attributed = has_file
            && g.attributed.iter().any(|a| {
                a.source_id == g.source_id
                    && whole(&a.episode) == Some(video)
                    && a.set_at < o.first_seen_at
                    && o.updated_at.is_none_or(|u| a.set_at < u)
            });
        if has_file && !revises_attributed {
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
                    Some(theirs) => theirs.season_episode(&p.episode) == Mapped::Episode(video),
                    None => key(&p.episode) == k,
                },
            );
        if another {
            continue;
        }
        out.push(Receipt {
            candidate: o,
            revision_of: None,
            revises_attributed,
        });
    }
    out.sort_by_key(|r| r.candidate.id);
    out
}

/// The subscribed creator's receipts and the `자막 구독` suggestions. Cheap to
/// clone.
#[derive(Clone)]
pub struct Follow {
    db: Db,
    requests: JobRequests,
    views: JobViews,
    channels: ChannelStore,
    seasons: SeasonStore,
    library: LibraryStore,
    anissia: AnissiaStore,
}

impl Follow {
    pub fn new(db: Db) -> Follow {
        Follow {
            requests: JobRequests::new(db.clone()),
            views: JobViews::new(db.clone()),
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

    /// Whether the work's folder is there. A work whose folder is gone
    /// (`missing`) holds nothing as far as the library knows, which is not the
    /// same as holding no subtitle, so nothing is received for it.
    pub(crate) async fn folder_present(&self, work_id: &str) -> Result<bool> {
        let work_id = work_id.to_owned();
        let missing = self
            .db
            .run(move |c| {
                Ok::<_, FollowError>(
                    c.prepare_cached("SELECT missing FROM works WHERE id = ?1")?
                        .query_row([work_id], |r| r.get::<_, i64>(0))
                        .optional()?,
                )
            })
            .await?;
        Ok(missing == Some(0))
    }

    /// The source of the subscribed creator `creator` among the observed
    /// sources of the subscription's anime, found by Anissia's name.
    pub(crate) async fn creator_source(
        &self,
        sub: &Subscribed,
        creator: &str,
    ) -> Result<Option<String>> {
        let observed = self.anissia.candidates(sub.anime_no, Vec::new()).await?;
        Ok(observed
            .into_iter()
            .find(|c| c.creator == creator)
            .map(|c| c.source_id))
    }

    /// What the library knows of the subscription's season for its mapping
    /// ([`SeasonGround`]).
    async fn season_ground(&self, work_id: &str, season: u32) -> Result<SeasonGround> {
        // The earlier seasons, then the season's own.
        let mut wanted: Vec<(String, u32)> = (1..season).map(|s| (work_id.to_owned(), s)).collect();
        wanted.push((work_id.to_owned(), season));
        let mut links = self.seasons.links_of_seasons(wanted).await?;
        let own = links
            .pop()
            .flatten()
            .filter(|link| !link.entries.is_empty());
        let previous = links
            .into_iter()
            .map(|link| {
                link.filter(|l| !l.entries.is_empty())
                    .and_then(|l| combine(&l.entries).and_then(|c| c.episodes))
            })
            .try_fold(0u32, |sum, n| n.and_then(|n| sum.checked_add(n)));
        Ok(match own {
            Some(link) => SeasonGround {
                schedule: schedule_times(&link.entries),
                count: combine(&link.entries).and_then(|c| c.episodes),
                previous,
            },
            None => SeasonGround {
                schedule: BTreeMap::new(),
                count: None,
                previous,
            },
        })
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
        if !self.folder_present(&sub.work_id).await? {
            return Ok(Vec::new());
        }
        let Some(held) = self
            .library
            .season_episodes(&sub.work_id, sub.season)
            .await?
        else {
            return Ok(Vec::new());
        };
        let ground = self.season_ground(&sub.work_id, sub.season).await?;
        // The earliest observation of each whole episode of the source, and the
        // newest text of each episode key.
        let mut earliest: BTreeMap<u32, &Candidate> = BTreeMap::new();
        let mut newest_text: BTreeMap<String, &Candidate> = BTreeMap::new();
        for c in observed.iter().filter(|c| c.source_id == source_id) {
            if let Some(n) = whole(&c.episode).and_then(|n| u32::try_from(n).ok()) {
                let entry = earliest.entry(n).or_insert(c);
                if c.id < entry.id {
                    *entry = c;
                }
            }
            let entry = newest_text.entry(key(&c.episode)).or_insert(c);
            if c.id > entry.id {
                *entry = c;
            }
        }
        let posted: Vec<mapping::Posted> = earliest
            .iter()
            .map(|(&episode, c)| mapping::Posted {
                episode,
                at: c.updated_at,
            })
            .collect();
        let texts: Vec<String> = newest_text.values().map(|c| c.episode.clone()).collect();
        let decided = mapping::decide(&posted, &ground.season(sub.season));
        let total = ground.season(sub.season).total();

        let (mappings, conflicted) = {
            let (work_id, season, source) = (sub.work_id.clone(), sub.season, source_id.clone());
            self.db
                .run(move |c| {
                    // Written before anyone else reads: a mapping the user sets
                    // meanwhile is not overwritten with the app's, and the
                    // conflicts are the ones of the mapping that stands.
                    let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                    let before = mapping::read_in(&tx, &work_id, season)?
                        .remove(&source)
                        .map(|m| m.version);
                    let stored = mapping::store_in(&tx, &work_id, season, &source, &decided, now)?;
                    // What follows the mapping moves with a change of it
                    // ([`crate::place::relocate`]).
                    if before != Some(stored.version) {
                        crate::place::relocate::reevaluate_in(
                            &tx, &work_id, season, &source, total, now,
                        )?;
                    }
                    let all = mapping::read_in(&tx, &work_id, season)?;
                    let found = match all.get(&source) {
                        Some(stored) => {
                            mapping::conflicts(stored, &texts, &posted, &ground.season(season))
                        }
                        None => Vec::new(),
                    };
                    mapping::store_conflicts(&tx, &work_id, season, &source, &found, now)?;
                    tx.commit()?;
                    let keys: HashSet<String> = found.iter().map(|f| key(&f.episode)).collect();
                    Ok::<_, FollowError>((all, keys))
                })
                .await?
        };
        let own = mappings.get(&source_id);
        let others: HashMap<String, Mapping> = mappings
            .iter()
            .filter(|(s, m)| **s != source_id && m.decided_offset().is_some())
            .map(|(s, m)| (s.clone(), m.clone()))
            .collect();

        let attributed = self
            .library
            .attributed_subtitles(&sub.work_id, sub.season)
            .await?;
        let picks = self.views.picks_of_anime(sub.anime_no).await?;
        let grounds = Grounds {
            source_id: &source_id,
            observed: &observed,
            picks: &picks,
            held: &held,
            mapping: own,
            others: &others,
            attributed: &attributed,
            conflicted: &conflicted,
        };
        let mut made = Vec::new();
        for Receipt {
            candidate,
            revision_of,
            revises_attributed,
        } in to_receive(&grounds)
        {
            let request = json!({
                "auto": candidate.id,
                "work_id": sub.work_id,
                "season": sub.season,
                "revision_of": revision_of,
                "revises_attributed": revises_attributed,
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
                revises_attributed,
                items: vec![NewItem {
                    observation_id: Some(candidate.id),
                    episode: candidate.episode.clone(),
                    post_url: candidate.post_url.clone(),
                    found_at: candidate.first_seen_at,
                }],
            };
            // Decided under the mapping read above: a save of the user's between
            // that read and now makes the store refuse the job (the version is
            // checked in the insert's own transaction), and the next look
            // decides again under the new mapping.
            let under = MappingStamp {
                work_id: sub.work_id.clone(),
                season: sub.season,
                source_id: source_id.clone(),
                version: own.map_or(0, |m| m.version),
            };
            if let Some(Created::Created(id)) =
                self.requests.create_under_mapping(job, now, under).await?
            {
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
            let (work_name, anime_title) = self.titles(&sub.work_id, sub.anime_no).await?;
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

    /// The Anissia anime a subtitle source (a creator's lines) belongs to;
    /// `None` for a source the app does not know.
    pub async fn source_anime(&self, source_id: &str) -> Result<Option<i64>> {
        let source_id = source_id.to_owned();
        self.db
            .run(move |c| {
                Ok::<_, FollowError>(
                    c.prepare_cached("SELECT anime_no FROM subtitle_sources WHERE id = ?1")?
                        .query_row([source_id], |r| r.get::<_, i64>(0))
                        .optional()?,
                )
            })
            .await
    }

    /// What the library knows of the season for a mapping the user sets: the
    /// ground the app's own decision stands on ([`SeasonGround`]).
    pub async fn season_facts(&self, work_id: &str, season: u32) -> Result<SeasonFacts> {
        let ground = self.season_ground(work_id, season).await?;
        Ok(SeasonFacts {
            previous: ground.previous,
            total: ground.season(season).total(),
        })
    }

    /// Saves the mapping the user set for a source of the season
    /// ([`mapping::set_user_in`]), moving what follows it within the
    /// season's episodes.
    pub async fn set_user_mapping(
        &self,
        work_id: &str,
        season: u32,
        source_id: &str,
        version: i64,
        user: mapping::UserMapping,
        now: Millis,
    ) -> Result<mapping::Saved> {
        let total = self
            .season_ground(work_id, season)
            .await?
            .season(season)
            .total();
        let (work_id, source_id) = (work_id.to_owned(), source_id.to_owned());
        self.db
            .run(move |c| {
                Ok::<_, FollowError>(mapping::set_user_in(
                    c, &work_id, season, &source_id, version, &user, total, now,
                )?)
            })
            .await
    }

    /// Takes the user's mapping of a source of the season back to the app
    /// ([`mapping::revert_in`]).
    pub async fn revert_mapping(
        &self,
        work_id: &str,
        season: u32,
        source_id: &str,
        version: i64,
    ) -> Result<mapping::Saved> {
        let (work_id, source_id) = (work_id.to_owned(), source_id.to_owned());
        self.db
            .run(move |c| {
                Ok::<_, FollowError>(mapping::revert_in(
                    c, &work_id, season, &source_id, version,
                )?)
            })
            .await
    }

    /// The subscribed creators' sources that need the user's mapping, the
    /// source of the `회차 확인 필요` to-do: the mapping is undecided, or an
    /// episode of the source does not fit it and no exception of the user's
    /// covers it. A subscription takes part as for the receipts
    /// ([`Follow::subscribed`]); a source whose creator is no longer the
    /// subscription's is none. Lowest season first within a work.
    pub async fn episode_checks(&self) -> Result<Vec<EpisodeCheck>> {
        let mut out = Vec::new();
        for sub in self.subscribed().await? {
            let Some(creator) = sub.creator.clone() else {
                continue;
            };
            let (work_id, season, anime_no, name) = (
                sub.work_id.clone(),
                sub.season,
                sub.anime_no,
                creator.clone(),
            );
            // The mapping first: the observations are read only for an
            // undecided mapping, and a decided one with nothing that does not
            // fit it costs these reads and no more.
            let found = self
                .db
                .run(move |c| {
                    let Some(source) = c
                        .prepare_cached("SELECT id FROM subtitle_sources
                              WHERE anime_no = ?1 AND creator_name = ?2 ORDER BY id LIMIT 1")?.query_row(
                            rusqlite::params![anime_no, name],
                            |r| r.get::<_, String>(0),
                        )
                        .optional()?
                    else {
                        return Ok::<_, FollowError>(None);
                    };
                    let Some(stored) = mapping::read_in(c, &work_id, season)?.remove(&source)
                    else {
                        return Ok(None);
                    };
                    let mut stmt = c.prepare_cached(
                        "SELECT episode, found_at FROM subtitle_mapping_conflicts
                          WHERE work_id = ?1 AND season = ?2 AND source_id = ?3
                          ORDER BY episode",
                    )?;
                    let rows = stmt.query_map(rusqlite::params![work_id, season, source], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, Millis>(1)?))
                    })?;
                    let mut uncovered = Vec::new();
                    for row in rows {
                        let (episode, found_at) = row?;
                        if stored.exception_of(&episode).is_none() {
                            uncovered.push((episode, found_at));
                        }
                    }
                    // `0` is the line a creator registers before the first
                    // episode: it is neither received nor asked about, so an
                    // undecided mapping asks only once the source has an
                    // observation of another episode.
                    let mut has_episode = false;
                    if stored.kind == mapping::MappingKind::Undecided {
                        let mut stmt = c.prepare_cached(
                            "SELECT DISTINCT episode FROM caption_observations WHERE source_id = ?1",
                        )?;
                        let episodes = stmt.query_map([&source], |r| r.get::<_, String>(0))?;
                        for episode in episodes {
                            let whole = EpisodeNumber::parse(&episode?).and_then(|n| n.whole());
                            if whole != Some(0) {
                                has_episode = true;
                                break;
                            }
                        }
                    }
                    Ok(Some((source, stored, uncovered, has_episode)))
                })
                .await?;
            let Some((source_id, stored, uncovered, has_episode)) = found else {
                continue;
            };
            let undecided = (stored.kind == mapping::MappingKind::Undecided && has_episode)
                .then(|| stored.evidence.clone());
            if undecided.is_none() && uncovered.is_empty() {
                continue;
            }
            let since = undecided
                .as_ref()
                .map(|_| stored.decided_at)
                .into_iter()
                .chain(uncovered.iter().map(|(_, at)| *at))
                .min()
                .unwrap_or(0);
            let (work_name, anime_title) = self.titles(&sub.work_id, sub.anime_no).await?;
            out.push(EpisodeCheck {
                work_id: sub.work_id.clone(),
                work_name,
                anime_title,
                season: sub.season,
                source_id,
                creator,
                undecided,
                episodes: uncovered.into_iter().map(|(episode, _)| episode).collect(),
                since,
            });
        }
        Ok(out)
    }

    /// The work's folder name and the anime's title in the stored schedule, if
    /// it is there.
    async fn titles(&self, work_id: &str, anime_no: i64) -> Result<(String, Option<String>)> {
        let work_id = work_id.to_owned();
        self.db
            .run(move |c| {
                Ok::<_, FollowError>(
                    c.prepare_cached(
                        "SELECT w.dir_name,
                            (SELECT a.subject FROM anissia_anime a WHERE a.anime_no = ?2)
                     FROM works w WHERE w.id = ?1",
                    )?
                    .query_row(rusqlite::params![work_id, anime_no], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
                    })?,
                )
            })
            .await
    }
}

/// What the library knows of a season for the mapping the user sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeasonFacts {
    /// The episodes of all earlier seasons together, when each is known (`0`
    /// for the first season).
    pub previous: Option<u32>,
    /// `N`, the season's episode count a mapping is measured against: the
    /// AniList count, else the highest scheduled episode
    /// ([`mapping::Season::total`]).
    pub total: Option<u32>,
}

/// A subscribed creator's source that needs the user's mapping
/// ([`Follow::episode_checks`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeCheck {
    pub work_id: String,
    /// The work's folder name.
    pub work_name: String,
    /// The anime's title in the stored schedule, if it is there.
    pub anime_title: Option<String>,
    pub season: u32,
    pub source_id: String,
    /// The creator followed.
    pub creator: String,
    /// Why the mapping is undecided; `None` when it is decided.
    pub undecided: Option<String>,
    /// The episodes that do not fit the mapping and have no exception, as
    /// Anissia writes them.
    pub episodes: Vec<String>,
    /// Since when: the mapping went undecided, or the oldest such episode was
    /// found.
    pub since: Millis,
}
