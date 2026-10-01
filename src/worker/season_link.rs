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
//! as it is; every cycle tries again. A rule whose videos are in more than one
//! season is left alone too: which one the rule belongs to is not told by them.
//! Videos a person put into a folder, and videos other rules received, are no
//! evidence for this rule, even in a folder of the same name.
//!
//! Once connected a rule stays connected. When the season is held by another
//! Anissia anime already the rule is not connected and notes the season, which
//! its detail explains ([`crate::store::channels::Subscription::season_blocked`]).
//! The season ID is `<work id>:<number>` ([`SeasonRef`]); season 0 (specials) is
//! no season of an airing anime and is never connected.

use std::collections::BTreeSet;

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

/// The seasons that hold a video of the given torrents.
async fn seasons_of(ctx: &CycleContext, places: &[&TorrentPlace]) -> BTreeSet<(String, u32)> {
    let mut found = BTreeSet::new();
    for place in places {
        let dir = place.download_dir.trim_end_matches('/');
        for file in &place.files {
            let path = format!("{dir}/{}", file.name);
            match ctx.library.find_video(&path).await {
                Ok(Some((work_id, season))) if season >= 1 => {
                    found.insert((work_id, season));
                }
                Ok(_) => {}
                Err(err) => eprintln!("Season link: cannot look a video up: {err}"),
            }
        }
    }
    found
}

/// Connects the subscriptions that have no season yet to the season their
/// received videos are in. Reads Transmission only when some subscription is
/// waiting for a season and has received something. Failures are logged and
/// leave the rules for the next cycle.
pub async fn link_seasons(ctx: &CycleContext) -> Linked {
    let mut done = Linked::default();
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
    let mut hashes: Vec<String> = received.values().flatten().cloned().collect();
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

    for rule in &rules {
        let Some(hashes) = received.get(&rule.id) else {
            continue;
        };
        let own: Vec<&TorrentPlace> = places
            .iter()
            .filter(|place| hashes.contains(&place.hash))
            .collect();
        let seasons = seasons_of(ctx, &own).await;
        let mut seasons = seasons.into_iter();
        let (Some((work_id, number)), None) = (seasons.next(), seasons.next()) else {
            continue;
        };
        let season = SeasonRef { work_id, number }.id();
        match ctx.channels.link_season(&rule.id, &season).await {
            Ok(SeasonLinked::Linked) => {
                println!("Season link: rule {} is in season {number}", rule.id);
                done.linked += 1;
            }
            Ok(SeasonLinked::Taken) => done.taken += 1,
            Ok(SeasonLinked::Kept | SeasonLinked::Gone) => {}
            Err(err) => eprintln!("Season link: cannot connect rule {}: {err}", rule.id),
        }
    }
    done
}
