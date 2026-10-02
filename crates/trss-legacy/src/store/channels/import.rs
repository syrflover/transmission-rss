//! Applying an import of several channels in one transaction.
//!
//! An import adds new channels and replaces existing ones with the file's
//! version. All of it is one `BEGIN IMMEDIATE` transaction: if any channel
//! fails (a stale version, a database error), nothing of the import remains.
//!
//! Replacing a channel keeps the ID of every existing rule that the file's rule
//! *matches* (see [`match_rules`]), so anything that points at a rule ID keeps
//! pointing at it. Existing rules the file has no match for are deleted; the
//! file's other rules get new IDs. The exception is a title-waiting
//! subscription ([`is_title_waiting_subscription`]): the file cannot express
//! one, so a replacement leaves it as it is, after the file's rules. [`ChannelStore::replace_channel`] instead
//! reissues every ID, which is why an import does not use it.

use std::{
    collections::{HashMap, HashSet},
    time::{SystemTime, UNIX_EPOCH},
};

use rusqlite::{params, Connection, Transaction, TransactionBehavior};
use uuid::Uuid;

use super::model::{Channel, ChannelInput, ChannelWithRules, Rule, RuleInput, Version};
use super::{
    import_subscriptions::{subscribe, ImportSubscription, SubscriptionOutcome},
    repo, ChannelError, ChannelStore,
};
use trss_core::{
    settings::{set_collect_folder_if_unset, SettingsError},
    Millis,
};
use trss_library::store::library::{ensure_automatic_in, LibraryError};

/// One channel of the file: its fields and rules in file order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportChannel {
    pub input: ChannelInput,
    pub rules: Vec<RuleInput>,
}

/// What to do with one channel of the file. A channel to skip is simply left
/// out of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportAction {
    /// Append as a new channel with new IDs, whatever else exists.
    Add(ImportChannel),
    /// Replace the existing channel `id`, if it is still at `expected_version`.
    Replace {
        id: String,
        expected_version: Version,
        channel: ImportChannel,
    },
}

/// The result of one applied action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportedChannel {
    Added(ChannelWithRules),
    Replaced {
        channel: ChannelWithRules,
        /// Rules of the file that took over an existing rule's ID.
        kept_rules: usize,
        /// Existing rules that were deleted, as they were before.
        removed_rules: Vec<Rule>,
        /// Title-waiting subscriptions of the channel that the replacement left
        /// untouched; they are the last rules of the channel.
        waiting_kept: usize,
    },
}

impl ImportedChannel {
    pub fn channel(&self) -> &ChannelWithRules {
        match self {
            ImportedChannel::Added(channel) => channel,
            ImportedChannel::Replaced { channel, .. } => channel,
        }
    }

    pub(super) fn channel_mut(&mut self) -> &mut ChannelWithRules {
        match self {
            ImportedChannel::Added(channel) => channel,
            ImportedChannel::Replaced { channel, .. } => channel,
        }
    }
}

/// Whether `rule` is a subscription still waiting for its title (a match phrase
/// of `null`, no first release yet). A legacy file cannot express it, so an
/// import replacing its channel keeps it.
pub fn is_title_waiting_subscription(rule: &Rule) -> bool {
    rule.r#match.is_none() && rule.subscription.is_some()
}

/// For each incoming rule, the index in `existing` of the rule whose ID it
/// keeps, or `None` when it becomes a new rule.
///
/// An incoming rule matches an existing rule of the same channel when they have
/// the same match phrase, regex flag and case-insensitive flag. Everything else
/// (directory, episode, state) is replaced by the incoming values. Pairing is
/// one to one and in order: when a key occurs several times, the first incoming
/// rule takes the first existing one, and so on; the surplus on either side is
/// new or removed. A rule without a phrase (still waiting for its title) has no
/// identity to compare, so it never matches.
pub fn match_rules(existing: &[Rule], incoming: &[RuleInput]) -> Vec<Option<usize>> {
    type Key<'a> = (&'a str, bool, bool);
    let mut pool: HashMap<Key<'_>, Vec<usize>> = HashMap::new();
    for (index, rule) in existing.iter().enumerate().rev() {
        if let Some(phrase) = rule.r#match.as_deref() {
            pool.entry((phrase, rule.regex, rule.case_insensitive))
                .or_default()
                .push(index);
        }
    }
    incoming
        .iter()
        .map(|rule| {
            let phrase = rule.r#match.as_deref()?;
            pool.get_mut(&(phrase, rule.regex, rule.case_insensitive))?
                .pop()
        })
        .collect()
}

impl ChannelStore {
    /// Applies `actions` in order as one transaction and returns one result per
    /// action, in the same order. Added channels are appended after the
    /// existing ones in action order; a replaced channel keeps its place.
    ///
    /// Fails with [`ChannelError::Conflict`] if a replaced channel changed
    /// since the caller read it, and with [`ChannelError::Invalid`] if a channel
    /// or rule is invalid or one channel is replaced twice. In every failure
    /// nothing of the import is applied.
    pub async fn import_channels(
        &self,
        actions: Vec<ImportAction>,
    ) -> Result<Vec<ImportedChannel>, ChannelError> {
        self.import_channels_setting_folder(actions, None).await
    }

    /// [`ChannelStore::import_channels`] that also sets the collect folder to
    /// `collect_folder` in the same transaction, for an import that adopts the
    /// file's folders while none is set, and makes that folder a watch folder
    /// as the collect folder always is (without reading it: the worker's next
    /// cycle does). If a collect folder was set in the meantime the import fails
    /// with [`ChannelError::Conflict`] and nothing of it is applied.
    pub async fn import_channels_setting_folder(
        &self,
        actions: Vec<ImportAction>,
        collect_folder: Option<String>,
    ) -> Result<Vec<ImportedChannel>, ChannelError> {
        self.db
            .run(move |c| {
                apply_import(c, &actions, collect_folder.as_deref(), &[], || 0)
                    .map(|(channels, _)| channels)
            })
            .await
    }
}

/// [`ChannelStore::import_channels_setting_folder`]'s transaction, which also
/// makes the rules `subscriptions` name subscriptions in the same transaction
/// ([`super::import_subscriptions`]).
pub(super) fn apply_import(
    conn: &mut Connection,
    actions: &[ImportAction],
    collect_folder: Option<&str>,
    subscriptions: &[ImportSubscription],
    now: impl FnOnce() -> Millis,
) -> Result<(Vec<ImportedChannel>, Vec<SubscriptionOutcome>), ChannelError> {
    if collect_folder == Some("") {
        return Err(ChannelError::Invalid("collect folder is empty"));
    }
    let mut replaced_ids = HashSet::new();
    for action in actions {
        let channel = match action {
            ImportAction::Add(channel) => channel,
            ImportAction::Replace { id, channel, .. } => {
                if !replaced_ids.insert(id.as_str()) {
                    return Err(ChannelError::Invalid(
                        "one channel cannot be replaced twice in an import",
                    ));
                }
                channel
            }
        };
        channel.input.validate()?;
        channel.rules.iter().try_for_each(RuleInput::validate)?;
    }
    for subscription in subscriptions {
        subscription.validate(actions)?;
    }

    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    // Read with the write lock held: the import time is not older than a write
    // that finished while this transaction waited to begin.
    let at = now();
    if let Some(folder) = collect_folder {
        set_collect_folder_if_unset(&tx, folder).map_err(|e| match e {
            SettingsError::Conflict { expected, actual } => ChannelError::Conflict {
                kind: "collect folder",
                id: String::new(),
                expected,
                actual,
            },
            SettingsError::Db(e) => ChannelError::Db(e),
            SettingsError::Invalid(reason) => ChannelError::Invalid(reason),
        })?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        ensure_automatic_in(&tx, folder, now).map_err(|e| match e {
            LibraryError::Db(e) => ChannelError::Db(e),
            _ => ChannelError::Invalid("the collect folder cannot be a watch folder"),
        })?;
    }
    let mut results = Vec::with_capacity(actions.len());
    for action in actions {
        results.push(match action {
            ImportAction::Add(channel) => add_channel(&tx, channel)?,
            ImportAction::Replace {
                id,
                expected_version,
                channel,
            } => replace_keeping_rule_ids(&tx, id, *expected_version, channel)?,
        });
    }
    let outcomes = subscribe(&tx, &mut results, subscriptions, at)?;
    tx.commit()?;
    Ok((results, outcomes))
}

fn new_id() -> String {
    Uuid::new_v4().to_string()
}

fn json(list: &[String]) -> String {
    serde_json::to_string(list).expect("a string list always serializes")
}

fn insert_rule(
    tx: &Transaction<'_>,
    channel_id: &str,
    position: usize,
    rule: &RuleInput,
) -> Result<(), ChannelError> {
    tx.execute(
        "INSERT INTO rules (id, channel_id, position, match_text, regex, case_insensitive,
                            directory, episode, episode_auto, episode_decided, state, version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9, ?10, 1)",
        params![
            new_id(),
            channel_id,
            position as i64,
            rule.r#match,
            rule.regex,
            rule.case_insensitive,
            rule.directory,
            rule.episode,
            rule.episode_auto,
            rule.state.as_str(),
        ],
    )?;
    Ok(())
}

fn add_channel(
    tx: &Transaction<'_>,
    channel: &ImportChannel,
) -> Result<ImportedChannel, ChannelError> {
    let id = new_id();
    let position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM channels",
        [],
        |r| r.get(0),
    )?;
    let input = &channel.input;
    tx.execute(
        "INSERT INTO channels (id, position, url, excludes, secret_query, past_search, name, version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1)",
        params![
            id,
            position,
            input.url,
            json(&input.excludes),
            json(&input.secret_query),
            input.past_search,
            input.stored_name(),
        ],
    )?;
    for (position, rule) in channel.rules.iter().enumerate() {
        insert_rule(tx, &id, position, rule)?;
    }
    Ok(ImportedChannel::Added(read_channel(tx, &id)?))
}

fn replace_keeping_rule_ids(
    tx: &Transaction<'_>,
    id: &str,
    expected: Version,
    channel: &ImportChannel,
) -> Result<ImportedChannel, ChannelError> {
    let current = repo::get_channel(tx, id)?.ok_or_else(|| ChannelError::NotFound {
        kind: "channel",
        id: id.to_owned(),
    })?;
    if current.version != expected {
        return Err(ChannelError::Conflict {
            kind: "channel",
            id: id.to_owned(),
            expected,
            actual: current.version,
        });
    }

    let input = &channel.input;
    tx.execute(
        "UPDATE channels
         SET url = ?2, excludes = ?3, secret_query = ?4, past_search = ?5, name = ?6,
             version = version + 1
         WHERE id = ?1",
        params![
            id,
            input.url,
            json(&input.excludes),
            json(&input.secret_query),
            input.past_search,
            input.stored_name(),
        ],
    )?;

    let existing = repo::list_rules(tx, id)?;
    let matches = match_rules(&existing, &channel.rules);
    let mut kept = HashSet::new();
    let mut kept_rules = 0;
    for (position, (rule, matched)) in channel.rules.iter().zip(&matches).enumerate() {
        match matched {
            Some(index) => {
                kept.insert(*index);
                kept_rules += 1;
                tx.execute(
                    "UPDATE rules
                     SET position = ?2, match_text = ?3, regex = ?4, case_insensitive = ?5,
                         directory = ?6,
                         episode_basis = CASE WHEN ?8 AND episode = ?7 THEN episode_basis END,
                         episode_previous = CASE WHEN ?8 AND episode = ?7
                                                 THEN episode_previous END,
                         episode_decided = episode_decided OR ?8,
                         episode = ?7, episode_auto = ?8, state = ?9,
                         version = version + 1
                     WHERE id = ?1",
                    params![
                        existing[*index].id,
                        position as i64,
                        rule.r#match,
                        rule.regex,
                        rule.case_insensitive,
                        rule.directory,
                        rule.episode,
                        rule.episode_auto,
                        rule.state.as_str(),
                    ],
                )?;
            }
            None => insert_rule(tx, id, position, rule)?,
        }
    }

    let mut removed_rules = Vec::new();
    let mut waiting_kept = 0;
    for (index, rule) in existing.into_iter().enumerate() {
        if kept.contains(&index) {
            continue;
        }
        if is_title_waiting_subscription(&rule) {
            // Left as it is (its subscription row goes on pointing at it); only
            // its place moves behind the file's rules, which keep the places
            // their index in the file names.
            tx.execute(
                "UPDATE rules SET position = ?2 WHERE id = ?1",
                params![rule.id, (channel.rules.len() + waiting_kept) as i64],
            )?;
            waiting_kept += 1;
        } else {
            tx.execute("DELETE FROM rules WHERE id = ?1", [&rule.id])?;
            removed_rules.push(rule);
        }
    }

    Ok(ImportedChannel::Replaced {
        channel: read_channel(tx, id)?,
        kept_rules,
        removed_rules,
        waiting_kept,
    })
}

pub(super) fn read_channel(conn: &Connection, id: &str) -> Result<ChannelWithRules, ChannelError> {
    let channel: Channel =
        repo::get_channel(conn, id)?.expect("the channel exists in this transaction");
    let rules = repo::list_rules(conn, id)?;
    Ok(ChannelWithRules { channel, rules })
}
