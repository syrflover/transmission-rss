//! Channels and their download rules, stored in the app database.
//!
//! Channels are ordered; each owns an ordered list of rules. Updates take the
//! version the caller last saw and fail with [`ChannelError::Conflict`], changing
//! nothing, if someone saved first. Multi-item operations (reordering, replacing
//! a channel with its rule list) are single transactions.
//!
//! Secret query values are stored verbatim in the channel URL. Use
//! [`Channel::masked_url`] / [`mask_url`] for anything shown or logged; the
//! `Debug` output and error messages of this module never contain them.

mod db_handle;
mod delete;
mod delete_rule;
pub mod import;
#[cfg(test)]
mod import_tests;
mod model;
mod repo;
#[cfg(test)]
mod tests;

pub use repo::{NewSubscription, SeasonLinked};

pub use model::{
    mask_url, query_names, Channel, ChannelInput, ChannelWithRules, OrderItem, Rule, RuleInput,
    RuleState, SeasonRef, Subscription, SubtitleMode, Version, MASK,
};

use super::db::{Db, DbError};

#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("{kind} {id} not found")]
    NotFound { kind: &'static str, id: String },
    /// The caller's version is not the stored one; nothing was changed.
    #[error(
        "{kind} {id} was changed by someone else (expected version {expected}, found {actual})"
    )]
    Conflict {
        kind: &'static str,
        id: String,
        expected: Version,
        actual: Version,
    },
    /// A requested order does not list exactly the items that exist now, so
    /// the caller's list is stale; nothing was changed.
    #[error("the {kind} list changed since it was read")]
    OrderMismatch { kind: &'static str },
    /// A rule stays in the channel it was created in.
    #[error("rule {rule_id} cannot be moved to another channel")]
    RuleChannelChange { rule_id: String },
    #[error("{0}")]
    Invalid(&'static str),
    /// The channel already has a rule subscribed to the anime.
    #[error("rule {rule_id} already subscribes to the anime in this channel")]
    AlreadySubscribed { rule_id: String },
}

impl ChannelError {
    /// True when the caller should re-read and retry with current data.
    pub fn is_conflict(&self) -> bool {
        matches!(
            self,
            ChannelError::Conflict { .. } | ChannelError::OrderMismatch { .. }
        )
    }
}

impl From<rusqlite::Error> for ChannelError {
    fn from(e: rusqlite::Error) -> Self {
        ChannelError::Db(DbError::Sqlite(e))
    }
}

/// Async access to channels and rules. Cheap to clone.
#[derive(Clone)]
pub struct ChannelStore {
    db: Db,
}

impl ChannelStore {
    pub fn new(db: Db) -> Self {
        ChannelStore { db }
    }

    /// Adds a channel at the end of the channel order.
    pub async fn create_channel(&self, input: ChannelInput) -> Result<Channel, ChannelError> {
        Ok(self
            .create_channel_with_rules(input, Vec::new())
            .await?
            .channel)
    }

    /// Adds a channel and its rules (in the given order) atomically.
    pub async fn create_channel_with_rules(
        &self,
        input: ChannelInput,
        rules: Vec<RuleInput>,
    ) -> Result<ChannelWithRules, ChannelError> {
        self.db
            .run(move |c| repo::create_channel(c, &input, &rules))
            .await
    }

    pub async fn get_channel(&self, id: &str) -> Result<Option<Channel>, ChannelError> {
        let id = id.to_owned();
        self.db.run(move |c| repo::get_channel(c, &id)).await
    }

    /// Channels in order.
    pub async fn list_channels(&self) -> Result<Vec<Channel>, ChannelError> {
        self.db.run(|c| repo::list_channels(c)).await
    }

    /// Channels in order, each with its rules in order, from one snapshot.
    pub async fn list_channels_with_rules(&self) -> Result<Vec<ChannelWithRules>, ChannelError> {
        self.db.run(repo::list_channels_with_rules).await
    }

    /// Changes a channel's fields if it is still at `expected_version`.
    pub async fn update_channel(
        &self,
        id: &str,
        expected_version: Version,
        input: ChannelInput,
    ) -> Result<Channel, ChannelError> {
        let id = id.to_owned();
        self.db
            .run(move |c| repo::update_channel(c, &id, expected_version, &input))
            .await
    }

    /// Replaces a channel's fields and its entire rule list atomically, if the
    /// channel is still at `expected_version`. Old rules are removed; the new
    /// ones get fresh IDs in the given order.
    pub async fn replace_channel(
        &self,
        id: &str,
        expected_version: Version,
        input: ChannelInput,
        rules: Vec<RuleInput>,
    ) -> Result<ChannelWithRules, ChannelError> {
        let id = id.to_owned();
        self.db
            .run(move |c| repo::replace_channel(c, &id, expected_version, &input, &rules))
            .await
    }

    /// Sets the channel order. `order` must list every channel once, each with
    /// the version the caller saw. Channels that move get a new version.
    pub async fn reorder_channels(
        &self,
        order: Vec<OrderItem>,
    ) -> Result<Vec<Channel>, ChannelError> {
        self.db
            .run(move |c| repo::reorder_channels(c, &order))
            .await
    }

    /// Adds a rule at the end of the channel's rules.
    pub async fn create_rule(
        &self,
        channel_id: &str,
        input: RuleInput,
    ) -> Result<Rule, ChannelError> {
        let channel_id = channel_id.to_owned();
        self.db
            .run(move |c| repo::create_rule(c, &channel_id, &input))
            .await
    }

    /// Adds a rule at the end of the channel's rules that is a subscription
    /// to an Anissia anime, and stores the anime's snapshot, atomically. The
    /// channel keeps one rule per subscribed anime
    /// ([`ChannelError::AlreadySubscribed`]).
    pub async fn create_subscription_rule(
        &self,
        channel_id: &str,
        input: RuleInput,
        subscription: NewSubscription,
    ) -> Result<Rule, ChannelError> {
        let channel_id = channel_id.to_owned();
        self.db
            .run(move |c| repo::create_subscription_rule(c, &channel_id, &input, &subscription))
            .await
    }

    pub async fn get_rule(&self, id: &str) -> Result<Option<Rule>, ChannelError> {
        let id = id.to_owned();
        self.db.run(move |c| repo::get_rule(c, &id)).await
    }

    /// The channel's rules in order, archived ones included.
    pub async fn list_rules(&self, channel_id: &str) -> Result<Vec<Rule>, ChannelError> {
        let channel_id = channel_id.to_owned();
        self.db.run(move |c| repo::list_rules(c, &channel_id)).await
    }

    /// Changes a rule's fields if it is still at `expected_version`.
    /// `channel_id` is the channel the caller believes the rule belongs to;
    /// any other value is rejected with [`ChannelError::RuleChannelChange`].
    pub async fn update_rule(
        &self,
        id: &str,
        expected_version: Version,
        channel_id: &str,
        input: RuleInput,
    ) -> Result<Rule, ChannelError> {
        let id = id.to_owned();
        let channel_id = channel_id.to_owned();
        self.db
            .run(move |c| repo::update_rule(c, &id, expected_version, &channel_id, &input))
            .await
    }

    /// Archives or restores a rule, whatever version it is at: only the worker
    /// does this, in the order its archive and restore need (see
    /// `worker::commands::rule_archive`). The version goes up when the state
    /// changes. `None` when the rule is gone.
    pub async fn set_rule_state(
        &self,
        id: &str,
        state: RuleState,
    ) -> Result<Option<Rule>, ChannelError> {
        let id = id.to_owned();
        self.db
            .run(move |c| repo::set_rule_state(c, &id, state))
            .await
    }

    /// `영상 받기`: turns a rule's collecting on or off (`active`/`paused`) if
    /// it is still at `expected_version`. An archived rule is refused: it is
    /// restored through the worker.
    pub async fn set_video_receiving(
        &self,
        id: &str,
        expected_version: Version,
        on: bool,
    ) -> Result<Rule, ChannelError> {
        let id = id.to_owned();
        self.db
            .run(move |c| repo::set_video_receiving(c, &id, expected_version, on))
            .await
    }

    /// `자막 받기` of a subscription of a collecting rule: off makes it
    /// `none` and keeps the creator, on follows the kept creator (or is
    /// `undecided` when there was none).
    pub async fn set_subtitle_receiving(
        &self,
        id: &str,
        expected_version: Version,
        on: bool,
    ) -> Result<Rule, ChannelError> {
        let id = id.to_owned();
        self.db
            .run(move |c| repo::set_subtitle_receiving(c, &id, expected_version, on))
            .await
    }

    /// Changes the creator a subscription follows; `None` is `제작자 미정`.
    pub async fn set_creator(
        &self,
        id: &str,
        expected_version: Version,
        creator: Option<String>,
    ) -> Result<Rule, ChannelError> {
        let id = id.to_owned();
        self.db
            .run(move |c| repo::set_creator(c, &id, expected_version, creator.as_deref()))
            .await
    }

    /// Makes an existing rule a subscription (`편성표와 연결`), keeping its
    /// match phrase, save folder, order and state.
    pub async fn subscribe_rule(
        &self,
        id: &str,
        expected_version: Version,
        subscription: NewSubscription,
    ) -> Result<Rule, ChannelError> {
        let id = id.to_owned();
        self.db
            .run(move |c| repo::subscribe_rule(c, &id, expected_version, &subscription))
            .await
    }

    /// Connects a subscription to the season `season_id` the rule's videos are
    /// in, unless it has one already or another Anissia anime holds the
    /// season (see [`SeasonLinked`]).
    pub async fn link_season(
        &self,
        rule_id: &str,
        season_id: &str,
    ) -> Result<SeasonLinked, ChannelError> {
        let (rule_id, season_id) = (rule_id.to_owned(), season_id.to_owned());
        self.db
            .run(move |c| repo::link_season(c, &rule_id, &season_id))
            .await
    }

    /// The Anissia anime whose subscriptions hold the season, if any.
    pub async fn season_holder(&self, season_id: &str) -> Result<Option<i64>, ChannelError> {
        let season_id = season_id.to_owned();
        self.db
            .run(move |c| repo::season_holder(c, &season_id))
            .await
    }

    /// The rules whose subscription is connected to a season of the work, with
    /// the season's number.
    pub async fn subscriptions_of_work(
        &self,
        work_id: &str,
    ) -> Result<Vec<(u32, Rule)>, ChannelError> {
        let work_id = work_id.to_owned();
        self.db
            .run(move |c| repo::subscriptions_of_work(c, &work_id))
            .await
    }

    /// Sets the order of a channel's rules. `order` must list every rule of the
    /// channel once (archived included), each with the version the caller saw.
    /// Rules that move get a new version. All or nothing.
    pub async fn reorder_rules(
        &self,
        channel_id: &str,
        order: Vec<OrderItem>,
    ) -> Result<Vec<Rule>, ChannelError> {
        let channel_id = channel_id.to_owned();
        self.db
            .run(move |c| repo::reorder_rules(c, &channel_id, &order))
            .await
    }
}
