//! Making rules of an import subscriptions, in the import's own transaction.
//!
//! The legacy YAML import offers a subscription for a rule whose comment names
//! an Anissia anime (`trss_import::suggest`); the rules the user checked are
//! passed along with the channels. They are written after the channels, so a
//! failure anywhere leaves nothing of the import, and the rules exist (with the
//! IDs they got or kept) when their subscriptions are written.
//!
//! A subscription made here is the one `POST /api/subscriptions` makes, with
//! `subscribed_at` set to the import time: what history recorded for the
//! channel before is past for the rule ([`crate::plan::ChannelPlan`]).
//! The import receives nothing.
//!
//! A rule is refused its subscription, without failing the import, when it is a
//! subscription already (a replaced channel kept the rule of an earlier
//! subscription) or when the channel already follows the anime with another
//! rule.

use rusqlite::{params, OptionalExtension, Transaction};

use crate::store::{
    anissia,
    channels::{
        import::{apply_import, read_channel, ImportAction, ImportedChannel},
        repo::{check_creator, check_not_subscribed, NewSubscription},
        ChannelError, ChannelStore,
    },
};
use trss_core::Millis;

/// A rule of an import that becomes a subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSubscription {
    /// Which action of the import (its place in the list) holds the rule.
    pub action: usize,
    /// The rule's place among that action's channel's rules.
    pub rule: usize,
    /// What to make of the rule. Its `subscribed_at` is not used: the import
    /// stamps the time itself, inside its transaction.
    pub subscription: NewSubscription,
    /// Whether `subscription.anime` is a stand-in because Anissia could not be
    /// asked (see [`ImportSubscription::stand_in`]): it never replaces a
    /// snapshot the app already has. An anime Anissia answered without is not
    /// subscribed at all; the caller leaves such a rule out.
    pub placeholder: bool,
}

impl ImportSubscription {
    /// A snapshot standing in for an anime Anissia could not be asked about.
    /// The worker's daily refresh finds it due at once (it was never
    /// received), so the real values replace it when Anissia answers.
    ///
    /// `airs` is the weekday (0 for Sunday to 6 for Saturday) and `HH:MM` time
    /// the legacy comment gave: the anime sits on that weekday until the
    /// refresh. Without it the anime sits in `기타` with no air time.
    pub fn stand_in(anime_no: i64, subject: &str, airs: Option<(u8, &str)>) -> trss_anissia::Anime {
        let (week, air_time) = match airs {
            Some((week, time)) if week <= 6 => (week, Some(time.to_owned())),
            _ => (trss_anissia::WEEK_OTHER, None),
        };
        trss_anissia::Anime {
            anime_no,
            subject: subject.to_owned(),
            original_subject: None,
            week,
            air_time,
            start_date: None,
            end_date: None,
            status: "OFF".to_owned(),
            fetched_at: 0,
        }
    }

    pub(super) fn validate(&self, actions: &[ImportAction]) -> Result<(), ChannelError> {
        let channel = match actions.get(self.action) {
            Some(ImportAction::Add(channel)) => channel,
            Some(ImportAction::Replace { channel, .. }) => channel,
            None => {
                return Err(ChannelError::Invalid(
                    "a subscription names no import action",
                ))
            }
        };
        if self.rule >= channel.rules.len() {
            return Err(ChannelError::Invalid("a subscription names no rule"));
        }
        check_creator(&self.subscription)
    }
}

/// What became of one [`ImportSubscription`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubscriptionOutcome {
    Created,
    /// The rule is a subscription already; it was left as it is.
    RuleAlreadySubscribed,
    /// The channel follows the anime with another rule.
    AnimeTakenInChannel {
        rule_id: String,
    },
}

impl ChannelStore {
    /// [`ChannelStore::import_channels_setting_folder`] that also makes the
    /// rules `subscriptions` name subscriptions, in the same transaction. The
    /// outcomes run in the order of `subscriptions`. `now` gives the import time
    /// (Unix ms) every subscription is stamped with; it is called once, after the
    /// transaction took the write lock, so a subscription is never dated before a
    /// write that the lock made the import wait for.
    pub async fn import_channels_subscribing(
        &self,
        actions: Vec<ImportAction>,
        collect_folder: Option<String>,
        subscriptions: Vec<ImportSubscription>,
        now: impl FnOnce() -> Millis + Send + 'static,
    ) -> Result<(Vec<ImportedChannel>, Vec<SubscriptionOutcome>), ChannelError> {
        self.db
            .run(move |c| apply_import(c, &actions, collect_folder.as_deref(), &subscriptions, now))
            .await
    }
}

/// Writes the subscriptions after the channels of `results`, and refreshes the
/// channels they changed.
pub(super) fn subscribe(
    tx: &Transaction<'_>,
    results: &mut [ImportedChannel],
    subscriptions: &[ImportSubscription],
    at: Millis,
) -> Result<Vec<SubscriptionOutcome>, ChannelError> {
    let mut outcomes = Vec::with_capacity(subscriptions.len());
    for import in subscriptions {
        let channel = results[import.action].channel();
        let rule = &channel.rules[import.rule];
        if rule.subscription.is_some() {
            outcomes.push(SubscriptionOutcome::RuleAlreadySubscribed);
            continue;
        }
        let (channel_id, rule_id) = (channel.channel.id.clone(), rule.id.clone());
        let new = &import.subscription;
        match check_not_subscribed(tx, &channel_id, new.anime.anime_no) {
            Ok(()) => {}
            Err(ChannelError::AlreadySubscribed { rule_id }) => {
                outcomes.push(SubscriptionOutcome::AnimeTakenInChannel { rule_id });
                continue;
            }
            Err(e) => return Err(e),
        }

        let known: Option<i64> = tx
            .query_row(
                "SELECT anime_no FROM anissia_anime WHERE anime_no = ?1",
                [new.anime.anime_no],
                |r| r.get(0),
            )
            .optional()?;
        if !(import.placeholder && known.is_some()) {
            anissia::upsert_in(tx, &new.anime)?;
        }
        tx.execute(
            "INSERT INTO rule_subscriptions (rule_id, anissia_anime_no, subtitles, creator, season_id, subscribed_at)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
            params![
                rule_id,
                new.anime.anime_no,
                new.subtitles.as_str(),
                new.creator,
                at,
            ],
        )?;
        *results[import.action].channel_mut() = read_channel(tx, &channel_id)?;
        outcomes.push(SubscriptionOutcome::Created);
    }
    Ok(outcomes)
}
