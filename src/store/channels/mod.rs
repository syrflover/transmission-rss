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

mod delete;
mod model;
mod repo;
#[cfg(test)]
mod tests;

pub use model::{
    mask_url, query_names, Channel, ChannelInput, ChannelWithRules, OrderItem, Rule, RuleInput,
    RuleState, Version, MASK,
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
