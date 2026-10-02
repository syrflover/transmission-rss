//! Archive suggestions: the rules the app suggests archiving because the anime
//! has ended or nothing new has come for a while (`docs/specs/collection.md`,
//! 규칙 보관, and `docs/specs/jobs.md`, 할 일).
//!
//! A suggestion is not stored. It is read off the rules, the Anissia snapshots
//! and the history every time, so it appears and goes with the facts it rests
//! on; only the user's `수집 유지` is kept (`store::channels` keeps the grounds
//! the user chose to keep collecting on). A rule is suggested when it has at
//! least one **ground** that the user has not kept collecting on:
//!
//! - The rule is not archived and has a match phrase. A paused rule is
//!   suggested on the same grounds; an archived one, and a subscription that
//!   still waits for its title (no phrase), are not.
//! - [`Ground::Ended`]: the rule is a subscription and its anime's run is over
//!   by the rule the weekly schedule drops its card by
//!   ([`crate::schedule::slot::run_over`]): the end date Anissia gave has
//!   passed, or, only when Anissia gave none, the daily refresh found the anime
//!   no longer listed. Nothing is read from Anissia here: while it cannot be
//!   reached the refresh finds nothing out, so this ground does not appear.
//! - [`Ground::Quiet`] (`새 항목 없음`): **no new item matching the rule has come
//!   for [`QUIET`] (4 weeks) since its last receive, the channel having been
//!   read for those weeks** (user decisions, 2026-10-01). The words are read
//!   from the history, and the weeks from the days the worker read the feed:
//!   - A *new item matching the rule* is a recorded item of the rule's channel,
//!     first seen within the last 4 weeks (after the moment 4 weeks ago, so the
//!     item a receive at exactly that moment took is not new), whose title the rule matches when
//!     the channel's excludes and the rules ahead of it are set aside (the
//!     rule's own judgement, as the rule preview makes it). Whatever became of
//!     the item does not matter, so what an off rule saw while paused, or what
//!     an earlier rule took, counts as new: the work is still coming. An item
//!     the channel excludes is not the rule's.
//!   - The *last receive* is the newest time the rule got an item into
//!     Transmission (`received`), the "마지막 수집" of its detail. A rule that
//!     never received has none, and counts from when it **started
//!     collecting**: the latest of when the app first had the rule (stamped by
//!     the database; a rule from before the stamp has the moment of the
//!     upgrade), when it became a subscription, when its title was given, when
//!     it was last turned back on, and when its channel was first read. Turning the rule back on, or restoring
//!     it, starts the 4 weeks over, however long its last receive was ago.
//!   - Only time the feed was **read** counts. A week in which the channel's
//!     address was dead, or the worker was off, is no quiet week: the worker
//!     leaves the days a read of each channel worked
//!     ([`crate::store::status::StatusStore::read_day_floors`]), and the
//!     ground needs [`QUIET_DAYS`] of them after the moment it counts from.
//!     So a rule whose last item was 5 weeks ago, on a channel that could not
//!     be read for the last 2, has had 3 weeks of reading and is not suggested
//!     until it has had 4, and a channel that fails gives its rules no new
//!     quiet ground. The clock still has to reach [`QUIET`] after that moment,
//!     so a rule is never suggested before 4 weeks have passed. A new item
//!     seen on a day that still counts within the last [`QUIET_DAYS`] read days
//!     is as recent as one of the last 4 weeks ([`Facts::window_start`]).
//!   - The 4 weeks have passed at exactly [`QUIET`] after that moment (when the
//!     feed was read every day), and the ground is gone the moment a matching
//!     item is recorded.
//!   - The window of titles is bounded ([`WINDOW_TITLES`] per channel). When a
//!     channel holds more than that in 4 weeks the recent items cannot all be
//!     told, so its rules get no quiet ground; a rule whose regular expression
//!     does not compile matches nothing and says nothing either.
//!
//! `수집 유지` keeps the grounds the user saw. A ground is told by its
//! [`Ground::key`]: the end of the anime by its date, the quiet stretch by the
//! moment it counts from. So the same ground does not suggest again, a rule
//! that receives and falls quiet again (a new moment) does, and so does an
//! anime that ends after the user kept a quiet rule.

use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use crate::{
    schedule::slot::{run_over, Over},
    store::{
        anissia::Anime,
        channels::{ChannelWithRules, Rule, RuleState},
        status::{read_day, READ_DAYS_KEPT},
    },
    worker::plan::ChannelPlan,
};
use trss_core::{
    calendar::{day_of, DAY_MS},
    Millis,
};

/// How long a rule must go without a new item before it is suggested: 4 weeks.
pub const QUIET: Millis = 28 * DAY_MS;

/// How many days on which its channel was read those 4 weeks must hold.
pub const QUIET_DAYS: usize = READ_DAYS_KEPT;

const _: () = assert!(QUIET == QUIET_DAYS as i64 * DAY_MS);

/// The most titles of one channel's last [`QUIET`] that are read. A channel
/// that held more in 4 weeks (a feed of everything everywhere would) gives its
/// rules no quiet ground, because the recent items cannot all be told.
pub const WINDOW_TITLES: usize = 20_000;

/// Why a rule is suggested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ground {
    /// The anime a subscription follows has stopped airing.
    Ended { anime_no: i64, over: Over },
    /// `새 항목 없음`: no new item matched the rule for [`QUIET`].
    Quiet {
        /// The moment the 4 weeks count from.
        since: Millis,
        /// When the rule last got an item into Transmission, if it ever did.
        last_received: Option<Millis>,
    },
}

impl Ground {
    /// What tells this ground from another of the same rule, kept with
    /// `수집 유지`.
    pub fn key(&self) -> String {
        match self {
            Ground::Ended {
                anime_no,
                over: Over::EndDate(date),
            } => format!("ended:{anime_no}:{date}"),
            Ground::Ended {
                anime_no,
                over: Over::Unlisted,
            } => format!("unlisted:{anime_no}"),
            Ground::Quiet { since, .. } => format!("quiet:{since}"),
        }
    }
}

/// A rule to suggest archiving, with the grounds that are new to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveSuggestion {
    pub rule_id: String,
    pub channel_id: String,
    /// The grounds the user has not chosen to keep collecting on, the end of the
    /// anime first.
    pub grounds: Vec<Ground>,
}

/// What a channel's last [`QUIET`] says about its rules.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recent {
    /// The rules some recent item matches.
    pub matched: HashSet<String>,
    /// The rules this cannot be told for (the window was cut short, or the
    /// rule's regular expression does not compile).
    pub unknown: HashSet<String>,
}

/// Which rules of `channel` the titles of its last [`QUIET`] match, `titles`
/// being those the history holds (`truncated` when there were more than were
/// read). The rules are asked as the rule preview asks them: every rule, in
/// the channel's order, with the channel's excludes, so a title taken by an
/// earlier rule still counts for a later one that matches it too.
pub fn recent_matches(channel: &ChannelWithRules, titles: &[String], truncated: bool) -> Recent {
    let wanted: HashSet<&str> = channel
        .rules
        .iter()
        .filter(|rule| is_suggestible(rule))
        .map(|rule| rule.id.as_str())
        .collect();
    if truncated {
        return Recent {
            matched: HashSet::new(),
            unknown: wanted.into_iter().map(str::to_owned).collect(),
        };
    }

    // Every rule is asked, whatever its state: an archived rule still stands
    // ahead of the others in the order.
    let mut open = channel.clone();
    for rule in &mut open.rules {
        rule.state = RuleState::Active;
    }
    let plan = ChannelPlan::new(open, Path::new(""));
    let unknown: HashSet<String> = plan
        .rule_errors()
        .into_iter()
        .map(|problem| problem.rule_id)
        .collect();

    let mut matched: HashSet<String> = HashSet::new();
    for title in titles {
        let evaluation = plan.evaluate(title);
        if let crate::worker::plan::Judgement::Selected { rule_id, .. } = evaluation.judgement {
            matched.insert(rule_id);
            matched.extend(evaluation.overlapping);
        }
        if wanted
            .iter()
            .all(|id| matched.contains(*id) || unknown.contains(*id))
        {
            break;
        }
    }
    Recent { matched, unknown }
}

/// Whether a rule can be suggested at all: not archived, and with a phrase
/// (a subscription that waits for its title has none).
fn is_suggestible(rule: &Rule) -> bool {
    rule.state != RuleState::Archived && rule.r#match.is_some()
}

/// Everything the suggestions are read off, gathered by the caller.
pub struct Facts<'a> {
    pub now: Millis,
    pub channels: &'a [ChannelWithRules],
    /// The Anissia snapshots of the subscribed anime.
    pub animes: &'a HashMap<i64, Anime>,
    /// The anime the daily refresh found Anissia no longer lists.
    pub unlisted: &'a HashSet<i64>,
    /// When each rule last received, by rule ID.
    pub last_received: &'a HashMap<String, Millis>,
    /// When the app first had each rule, by rule ID.
    pub started: &'a HashMap<String, Millis>,
    /// When each channel was first read (the time of its first history record,
    /// stored once), by channel ID.
    pub first_read: &'a HashMap<String, Millis>,
    /// The [`read_day`] of the oldest of each channel's newest [`QUIET_DAYS`]
    /// days on which the worker read its feed, by channel ID. A channel with
    /// fewer read days is not in the map and gives no quiet ground.
    pub read_floors: &'a HashMap<String, i64>,
    /// The grounds the user chose to keep collecting on, as `(rule ID, key)`.
    pub kept: &'a HashSet<(String, String)>,
}

impl Facts<'_> {
    /// The start of the channel's last [`QUIET`]: the items first seen after
    /// here are the recent ones. An item first seen at this very moment is no
    /// newer than a receive that is exactly [`QUIET`] ago, and is not counted.
    /// When the feed was not read on some of those days the window reaches back
    /// to the first of its last [`QUIET_DAYS`] read days, because an item
    /// seen on a day that still counts has not yet been followed by 4 weeks of
    /// reading.
    pub fn window_start(&self, channel_id: &str) -> Millis {
        let by_clock = self.now - QUIET;
        match self.read_floors.get(channel_id) {
            Some(floor) => by_clock.min(floor * DAY_MS - 1),
            None => by_clock,
        }
    }

    /// The moment the rule's 4 weeks count from; see the module docs. `None`
    /// when nothing says (no receive, no stamp, no history of the channel).
    fn quiet_since(&self, rule: &Rule) -> Option<Millis> {
        let subscription = rule.subscription.as_ref();
        [
            self.last_received.get(&rule.id).copied(),
            self.started.get(&rule.id).copied(),
            rule.resumed_at,
            subscription.map(|s| s.subscribed_at),
            subscription.and_then(|s| s.titled_at),
            // A first read stamped by a clock that was ahead is no start.
            self.first_read
                .get(&rule.channel_id)
                .copied()
                .filter(|&at| at <= self.now),
        ]
        .into_iter()
        .flatten()
        .max()
    }

    /// Whether the user chose to keep collecting on the ground.
    fn is_kept(&self, rule: &Rule, ground: &Ground) -> bool {
        self.kept.contains(&(rule.id.clone(), ground.key()))
    }

    /// The ground for the end of the rule's anime, if it has ended.
    fn ended(&self, rule: &Rule) -> Option<Ground> {
        let subscription = rule.subscription.as_ref()?;
        let anime = self.animes.get(&subscription.anissia_anime_no)?;
        let unlisted = self.unlisted.contains(&anime.anime_no);
        let over = run_over(anime, unlisted, day_of(self.now))?;
        Some(Ground::Ended {
            anime_no: anime.anime_no,
            over,
        })
    }

    /// The quiet ground of the rule, when 4 weeks have passed since its start
    /// moment and its channel was read on [`QUIET_DAYS`] days since (the day of
    /// the moment itself is not counted: a read of it may be from before);
    /// whether anything recent matched is asked separately.
    fn quiet_due(&self, rule: &Rule) -> Option<Ground> {
        let since = self.quiet_since(rule)?;
        let floor = *self.read_floors.get(&rule.channel_id)?;
        (self.now - since >= QUIET && read_day(since) < floor).then(|| Ground::Quiet {
            since,
            last_received: self.last_received.get(&rule.id).copied(),
        })
    }

    /// The channels whose last [`QUIET`] has to be read: those with a rule that
    /// is quiet by the clock and whose quiet ground the user has not kept
    /// already. The caller reads `titles_since(window_start(channel))` of each and
    /// answers [`Facts::suggestions`] with what [`recent_matches`] makes of it.
    pub fn channels_to_read(&self) -> Vec<String> {
        self.channels
            .iter()
            .filter(|cwr| {
                cwr.rules
                    .iter()
                    .filter(|rule| is_suggestible(rule))
                    .any(|rule| {
                        self.quiet_due(rule)
                            .is_some_and(|ground| !self.is_kept(rule, &ground))
                    })
            })
            .map(|cwr| cwr.channel.id.clone())
            .collect()
    }

    /// The rules to suggest archiving, in the order of the channels and their
    /// rules. `recent` has the channels [`Facts::channels_to_read`] named; a
    /// channel it lacks gives no quiet ground.
    pub fn suggestions(&self, recent: &HashMap<String, Recent>) -> Vec<ArchiveSuggestion> {
        let mut out = Vec::new();
        for cwr in self.channels {
            let recent = recent.get(&cwr.channel.id);
            for rule in cwr.rules.iter().filter(|rule| is_suggestible(rule)) {
                let mut grounds = Vec::new();
                grounds.extend(self.ended(rule));
                if let (Some(ground), Some(recent)) = (self.quiet_due(rule), recent) {
                    if !recent.matched.contains(&rule.id) && !recent.unknown.contains(&rule.id) {
                        grounds.push(ground);
                    }
                }
                grounds.retain(|ground| !self.is_kept(rule, ground));
                if !grounds.is_empty() {
                    out.push(ArchiveSuggestion {
                        rule_id: rule.id.clone(),
                        channel_id: cwr.channel.id.clone(),
                        grounds,
                    });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;
