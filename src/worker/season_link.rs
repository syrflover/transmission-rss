//! Connecting a subscription to the season its videos appeared in
//! (`docs/specs/collection.md`, 방영작 구독, and `docs/specs/library.md`, 머리와
//! 시즌).
//!
//! A subscription rule that has no season yet gets one from the videos *its own*
//! collection brought in, never from a folder's name or place:
//!
//! 1. the history items the rule received (`received`, with the rule recorded)
//!    name the torrents it added;
//! 2. Transmission says where each torrent's files are now (its download folder
//!    and each file's name, which the rename after the add has changed);
//! 3. the library, which the worker reads from the watch folders, says which work
//!    and which season folder holds a video at that path.
//!
//! A video the library has not recorded yet (the scan comes later), a torrent
//! Transmission no longer has, and a rule that received nothing leave the rule
//! as it is. A rule whose videos are in more than one season is left alone too:
//! which one the rule belongs to is not told by them. Videos a person put into a
//! folder, and videos other rules received, are no evidence for this rule, even
//! in a folder of the same name.
//!
//! Once connected a rule stays connected. When the season is held by another
//! Anissia anime already the rule is not connected and notes the season, which
//! its detail explains ([`crate::store::channels::Subscription::season_blocked`]).
//! The season ID is `<work id>:<number>` ([`SeasonRef`]); season 0 (specials) is
//! no season of an airing anime and is never connected.
//!
//! # What a pass does not repeat
//!
//! A rule that stays unconnected for good (its torrent is gone, its videos are in
//! several seasons, its season is taken, the library knows its folder by another
//! path) would otherwise ask Transmission for its torrents and look every file up
//! on every cycle. What decides an attempt's outcome is the set of torrents the
//! rule received and what the library holds at the paths Transmission reports,
//! and Transmission's paths change together with the files on disk, which the
//! library then notes. So the pass remembers, per rule, the torrents and the
//! library's generation ([`crate::store::library::LibraryStore::generation`]) of
//! its last attempt that did not connect it, and tries again only when either
//! differs.
//!
//! The memory is in the worker's process ([`Memory`]), not the database: it only
//! saves work, a restart costs one attempt per rule, and nothing else reads it.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::{Arc, Mutex},
};

use super::CycleContext;
use crate::{
    store::channels::{Rule, SeasonLinked, SeasonRef},
    transmission::{self, TorrentPlace},
};

/// What one pass came to (tests and logs).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Linked {
    /// Rules connected to a season now.
    pub linked: usize,
    /// Rules whose season another Anissia anime holds.
    pub taken: usize,
}

/// The inputs of an attempt that left a rule unconnected.
#[derive(Debug, PartialEq, Eq)]
struct Attempt {
    /// The torrents the rule had received, sorted.
    hashes: Vec<String>,
    /// The library's generation before the attempt read anything.
    generation: i64,
}

/// What the pass remembers between cycles (see the module's `What a pass does
/// not repeat`).
#[derive(Default)]
pub struct Remembered {
    /// The last attempt that left the rule unconnected, by rule ID.
    attempts: HashMap<String, Attempt>,
}

pub type Memory = Arc<Mutex<Remembered>>;

impl Remembered {
    /// Keeps only what is about the given rules.
    fn retain(&mut self, rule_ids: &HashSet<&str>) {
        self.attempts.retain(|id, _| rule_ids.contains(id.as_str()));
    }

    fn forget(&mut self, rule_id: &str) {
        self.attempts.remove(rule_id);
    }

    /// Whether trying again could end differently.
    fn worth_trying(&self, rule_id: &str, hashes: &[String], generation: i64) -> bool {
        self.attempts
            .get(rule_id)
            .is_none_or(|last| last.hashes != hashes || last.generation != generation)
    }
}

fn lock(memory: &Memory) -> std::sync::MutexGuard<'_, Remembered> {
    memory.lock().unwrap_or_else(|e| e.into_inner())
}

/// What the library knows of the files of some torrents.
#[derive(Default)]
struct Found {
    /// The seasons (work and number, from 1) that hold a video of them.
    seasons: BTreeSet<(String, u32)>,
}

/// What the library holds at the files of the given torrents, or `None` when
/// it could not be asked.
async fn find(ctx: &CycleContext, places: &[&TorrentPlace]) -> Option<Found> {
    let paths: Vec<String> = places
        .iter()
        .flat_map(|place| {
            let dir = place.download_dir.trim_end_matches('/');
            place
                .files
                .iter()
                .map(move |file| format!("{dir}/{}", file.name))
        })
        .collect();
    let videos = match ctx.library.find_videos(paths).await {
        Ok(videos) => videos,
        Err(err) => {
            eprintln!("Season link: cannot look the videos up: {err}");
            return None;
        }
    };
    let mut found = Found::default();
    for (work_id, season) in videos.into_iter().flatten() {
        if season >= 1 {
            found.seasons.insert((work_id, season));
        }
    }
    Some(found)
}

/// Connects the subscriptions that have no season yet to the season their
/// received videos are in. Reads Transmission only when some subscription is
/// waiting for a season and something it depends on is new (see the module's
/// `What a pass does not repeat`). Failures are logged and leave the rules for
/// the next cycle.
pub async fn link_seasons(ctx: &CycleContext) -> Linked {
    let mut done = Linked::default();
    // Read before anything the attempt depends on, so that a change during the
    // attempt makes the next one try again.
    let generation = match ctx.library.generation().await {
        Ok(generation) => generation,
        Err(err) => {
            eprintln!("Season link: cannot read the library: {err}");
            return done;
        }
    };

    let rules: Vec<Rule> = match ctx.channels.list_channels_with_rules().await {
        Ok(all) => all
            .into_iter()
            .flat_map(|cwr| cwr.rules)
            .filter(|rule| {
                rule.subscription
                    .as_ref()
                    .is_some_and(|s| s.season_id.is_none())
            })
            .collect(),
        Err(err) => {
            eprintln!("Season link: cannot read the rules: {err}");
            return done;
        }
    };
    lock(&ctx.season_link).retain(&rules.iter().map(|r| r.id.as_str()).collect());
    if rules.is_empty() {
        return done;
    }

    let received = match ctx
        .history
        .received_hashes_of_rules(rules.iter().map(|r| r.id.clone()).collect())
        .await
    {
        Ok(received) => received,
        Err(err) => {
            eprintln!("Season link: cannot read the history: {err}");
            return done;
        }
    };
    let waiting: Vec<(&Rule, &Vec<String>)> = {
        let memory = lock(&ctx.season_link);
        rules
            .iter()
            .filter_map(|rule| Some((rule, received.get(&rule.id)?)))
            .filter(|(rule, hashes)| memory.worth_trying(&rule.id, hashes, generation))
            .collect()
    };
    let mut hashes: Vec<String> = waiting
        .iter()
        .flat_map(|(_, hashes)| hashes.iter().cloned())
        .collect();
    hashes.sort();
    hashes.dedup();
    if hashes.is_empty() {
        return done;
    }

    let mut client = ctx.transmission();
    let places = match transmission::torrent_places(&mut client, Some(&hashes), true).await {
        Ok(places) => places,
        Err(err) => {
            eprintln!(
                "Season link: cannot read the torrents from Transmission: {}",
                ctx.redactor.apply(&err.to_string())
            );
            return done;
        }
    };

    for (rule, hashes) in waiting {
        let own: Vec<&TorrentPlace> = places
            .iter()
            .filter(|place| hashes.contains(&place.hash))
            .collect();
        let Some(found) = find(ctx, &own).await else {
            continue;
        };
        let mut seasons = found.seasons.iter();
        let one = match (seasons.next(), seasons.next()) {
            (Some(season), None) => Some(season),
            _ => None,
        };
        let linked = match one {
            Some((work_id, number)) => {
                let season = SeasonRef {
                    work_id: work_id.clone(),
                    number: *number,
                }
                .id();
                match ctx.channels.link_season(&rule.id, &season).await {
                    Ok(SeasonLinked::Linked) => {
                        println!("Season link: rule {} is in season {number}", rule.id);
                        done.linked += 1;
                        true
                    }
                    Ok(SeasonLinked::Taken) => {
                        done.taken += 1;
                        false
                    }
                    Ok(SeasonLinked::Kept | SeasonLinked::Gone) => true,
                    Err(err) => {
                        eprintln!("Season link: cannot connect rule {}: {err}", rule.id);
                        continue;
                    }
                }
            }
            None => false,
        };

        let mut memory = lock(&ctx.season_link);
        if linked {
            memory.forget(&rule.id);
            continue;
        }
        memory.attempts.insert(
            rule.id.clone(),
            Attempt {
                hashes: hashes.clone(),
                generation,
            },
        );
    }
    done
}
