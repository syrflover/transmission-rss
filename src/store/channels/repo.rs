//! Synchronous SQL for channels and rules. Every mutation runs in one
//! `BEGIN IMMEDIATE` transaction, which also takes the write lock before the
//! version check so a check-then-write cannot interleave with the other
//! process.

use std::collections::{HashMap, HashSet};

use rusqlite::types::Type;
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};
use uuid::Uuid;

use super::model::{
    Channel, ChannelInput, ChannelWithRules, OrderItem, Rule, RuleInput, RuleState, Subscription,
    SubtitleMode, Version,
};
use super::ChannelError;
use crate::store::{anissia, history::Millis};

type Result<T> = std::result::Result<T, ChannelError>;

const CHANNEL_COLUMNS: &str =
    "id, position, version, url, excludes, secret_query, past_search, name";
/// A rule with its subscription, if it has one (columns 11 to 15).
const RULE_COLUMNS: &str = "r.id, r.channel_id, r.position, r.version, r.match_text, r.regex, \
     r.case_insensitive, r.directory, r.episode, r.episode_auto, r.state, \
     s.anissia_anime_no, s.subtitles, s.creator, s.season_id, s.subscribed_at";
const RULE_FROM: &str = "rules r LEFT JOIN rule_subscriptions s ON s.rule_id = r.id";

fn begin(conn: &mut Connection) -> Result<Transaction<'_>> {
    Ok(conn.transaction_with_behavior(TransactionBehavior::Immediate)?)
}

fn new_id() -> String {
    Uuid::new_v4().to_string()
}

fn conversion_error(idx: usize, ty: Type, msg: &'static str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(idx, ty, msg.into())
}

fn string_list(row: &Row<'_>, idx: usize) -> rusqlite::Result<Vec<String>> {
    let json: String = row.get(idx)?;
    serde_json::from_str(&json)
        .map_err(|_| conversion_error(idx, Type::Text, "stored list is not a JSON string array"))
}

fn to_json(list: &[String]) -> String {
    serde_json::to_string(list).expect("a string list always serializes")
}

fn channel_from_row(row: &Row<'_>) -> rusqlite::Result<Channel> {
    Ok(Channel {
        id: row.get(0)?,
        position: row.get(1)?,
        version: row.get(2)?,
        url: row.get(3)?,
        excludes: string_list(row, 4)?,
        secret_query: string_list(row, 5)?,
        past_search: row.get(6)?,
        name: row.get(7)?,
    })
}

fn rule_from_row(row: &Row<'_>) -> rusqlite::Result<Rule> {
    let state: String = row.get(10)?;
    let anime_no: Option<i64> = row.get(11)?;
    let subscription = match anime_no {
        Some(anissia_anime_no) => {
            let subtitles: String = row.get(12)?;
            Some(Subscription {
                anissia_anime_no,
                subtitles: SubtitleMode::parse(&subtitles)
                    .ok_or_else(|| conversion_error(12, Type::Text, "unknown subtitle mode"))?,
                creator: row.get(13)?,
                season_id: row.get(14)?,
                subscribed_at: row.get(15)?,
            })
        }
        None => None,
    };
    Ok(Rule {
        id: row.get(0)?,
        channel_id: row.get(1)?,
        position: row.get(2)?,
        version: row.get(3)?,
        r#match: row.get(4)?,
        regex: row.get(5)?,
        case_insensitive: row.get(6)?,
        directory: row.get(7)?,
        episode: row.get(8)?,
        episode_auto: row.get(9)?,
        state: RuleState::parse(&state)
            .ok_or_else(|| conversion_error(10, Type::Text, "unknown rule state"))?,
        subscription,
    })
}

fn fetch_channel(conn: &Connection, id: &str) -> Result<Option<Channel>> {
    Ok(conn
        .query_row(
            &format!("SELECT {CHANNEL_COLUMNS} FROM channels WHERE id = ?1"),
            [id],
            channel_from_row,
        )
        .optional()?)
}

fn fetch_channels(conn: &Connection) -> Result<Vec<Channel>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {CHANNEL_COLUMNS} FROM channels ORDER BY position, id"
    ))?;
    let rows = stmt.query_map([], channel_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn fetch_rule(conn: &Connection, id: &str) -> Result<Option<Rule>> {
    Ok(conn
        .query_row(
            &format!("SELECT {RULE_COLUMNS} FROM {RULE_FROM} WHERE r.id = ?1"),
            [id],
            rule_from_row,
        )
        .optional()?)
}

fn fetch_rules(conn: &Connection, channel_id: &str) -> Result<Vec<Rule>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RULE_COLUMNS} FROM {RULE_FROM} WHERE r.channel_id = ?1 ORDER BY r.position, r.id"
    ))?;
    let rows = stmt.query_map([channel_id], rule_from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn require_channel(conn: &Connection, id: &str) -> Result<Channel> {
    fetch_channel(conn, id)?.ok_or_else(|| ChannelError::NotFound {
        kind: "channel",
        id: id.to_owned(),
    })
}

fn check_version(kind: &'static str, id: &str, expected: Version, actual: Version) -> Result<()> {
    if expected == actual {
        Ok(())
    } else {
        Err(ChannelError::Conflict {
            kind,
            id: id.to_owned(),
            expected,
            actual,
        })
    }
}

fn insert_channel(tx: &Transaction<'_>, input: &ChannelInput) -> Result<String> {
    let id = new_id();
    let position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM channels",
        [],
        |r| r.get(0),
    )?;
    tx.execute(
        "INSERT INTO channels (id, position, url, excludes, secret_query, past_search, name, version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1)",
        params![
            id,
            position,
            input.url,
            to_json(&input.excludes),
            to_json(&input.secret_query),
            input.past_search,
            input.stored_name(),
        ],
    )?;
    Ok(id)
}

fn insert_rule(
    tx: &Transaction<'_>,
    channel_id: &str,
    position: i64,
    input: &RuleInput,
) -> Result<String> {
    let id = new_id();
    tx.execute(
        "INSERT INTO rules (id, channel_id, position, match_text, regex, case_insensitive,
                            directory, episode, episode_auto, state, version)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1)",
        params![
            id,
            channel_id,
            position,
            input.r#match,
            input.regex,
            input.case_insensitive,
            input.directory,
            input.episode,
            input.episode_auto,
            input.state.as_str(),
        ],
    )?;
    Ok(id)
}

fn insert_rules(tx: &Transaction<'_>, channel_id: &str, rules: &[RuleInput]) -> Result<()> {
    for (position, rule) in rules.iter().enumerate() {
        insert_rule(tx, channel_id, position as i64, rule)?;
    }
    Ok(())
}

fn channel_with_rules(conn: &Connection, id: &str) -> Result<ChannelWithRules> {
    Ok(ChannelWithRules {
        channel: require_channel(conn, id)?,
        rules: fetch_rules(conn, id)?,
    })
}

pub fn create_channel(
    conn: &mut Connection,
    input: &ChannelInput,
    rules: &[RuleInput],
) -> Result<ChannelWithRules> {
    input.validate()?;
    rules.iter().try_for_each(RuleInput::validate)?;

    let tx = begin(conn)?;
    let id = insert_channel(&tx, input)?;
    insert_rules(&tx, &id, rules)?;
    let created = channel_with_rules(&tx, &id)?;
    tx.commit()?;
    Ok(created)
}

pub fn get_channel(conn: &Connection, id: &str) -> Result<Option<Channel>> {
    fetch_channel(conn, id)
}

pub fn list_channels(conn: &Connection) -> Result<Vec<Channel>> {
    fetch_channels(conn)
}

/// All channels with their rules, read as one consistent snapshot.
pub fn list_channels_with_rules(conn: &mut Connection) -> Result<Vec<ChannelWithRules>> {
    let tx = conn.transaction()?;
    fetch_channels(&tx)?
        .into_iter()
        .map(|channel| {
            let rules = fetch_rules(&tx, &channel.id)?;
            Ok(ChannelWithRules { channel, rules })
        })
        .collect()
}

const UPDATE_CHANNEL: &str = "UPDATE channels
     SET url = ?2, excludes = ?3, secret_query = ?4, past_search = ?5, name = ?6,
         version = version + 1
     WHERE id = ?1";

fn write_channel_fields(tx: &Transaction<'_>, id: &str, input: &ChannelInput) -> Result<()> {
    tx.execute(
        UPDATE_CHANNEL,
        params![
            id,
            input.url,
            to_json(&input.excludes),
            to_json(&input.secret_query),
            input.past_search,
            input.stored_name(),
        ],
    )?;
    Ok(())
}

pub fn update_channel(
    conn: &mut Connection,
    id: &str,
    expected: Version,
    input: &ChannelInput,
) -> Result<Channel> {
    input.validate()?;

    let tx = begin(conn)?;
    let current = require_channel(&tx, id)?;
    check_version("channel", id, expected, current.version)?;
    write_channel_fields(&tx, id, input)?;
    let updated = require_channel(&tx, id)?;
    tx.commit()?;
    Ok(updated)
}

/// Replaces the channel's fields and its whole rule list in one transaction.
/// The old rules are removed and the new ones get fresh IDs.
pub fn replace_channel(
    conn: &mut Connection,
    id: &str,
    expected: Version,
    input: &ChannelInput,
    rules: &[RuleInput],
) -> Result<ChannelWithRules> {
    input.validate()?;
    rules.iter().try_for_each(RuleInput::validate)?;

    let tx = begin(conn)?;
    let current = require_channel(&tx, id)?;
    check_version("channel", id, expected, current.version)?;
    write_channel_fields(&tx, id, input)?;
    tx.execute("DELETE FROM rules WHERE channel_id = ?1", [id])?;
    insert_rules(&tx, id, rules)?;
    let replaced = channel_with_rules(&tx, id)?;
    tx.commit()?;
    Ok(replaced)
}

pub fn create_rule(conn: &mut Connection, channel_id: &str, input: &RuleInput) -> Result<Rule> {
    input.validate()?;

    let tx = begin(conn)?;
    require_channel(&tx, channel_id)?;
    let position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM rules WHERE channel_id = ?1",
        [channel_id],
        |r| r.get(0),
    )?;
    let id = insert_rule(&tx, channel_id, position, input)?;
    let created = fetch_rule(&tx, &id)?.expect("the rule was just inserted");
    tx.commit()?;
    Ok(created)
}

/// What a rule needs to become a subscription: the anime as Anissia's schedule
/// listed it (stored as the anime's snapshot), how it gets subtitles, and when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSubscription {
    pub anime: anissia::Anime,
    pub subtitles: SubtitleMode,
    /// The creator to follow: set exactly when `subtitles` is `Follow`.
    pub creator: Option<String>,
    pub subscribed_at: Millis,
}

/// Adds a rule at the end of the channel's rules that is a subscription to
/// `subscription.anime`, and stores the anime's snapshot, in one transaction.
/// A channel has one rule per subscribed anime: a second one is refused with
/// [`ChannelError::AlreadySubscribed`].
pub fn create_subscription_rule(
    conn: &mut Connection,
    channel_id: &str,
    input: &RuleInput,
    subscription: &NewSubscription,
) -> Result<Rule> {
    input.validate()?;
    let creator_fits = match subscription.subtitles {
        SubtitleMode::Follow => subscription
            .creator
            .as_deref()
            .is_some_and(|c| !c.is_empty()),
        SubtitleMode::Undecided | SubtitleMode::None => subscription.creator.is_none(),
    };
    if !creator_fits {
        return Err(ChannelError::Invalid(
            "a creator is set exactly when subtitles follow a creator",
        ));
    }

    let tx = begin(conn)?;
    require_channel(&tx, channel_id)?;
    let existing: Option<String> = tx
        .query_row(
            "SELECT s.rule_id FROM rule_subscriptions s JOIN rules r ON r.id = s.rule_id
              WHERE r.channel_id = ?1 AND s.anissia_anime_no = ?2",
            params![channel_id, subscription.anime.anime_no],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(rule_id) = existing {
        return Err(ChannelError::AlreadySubscribed { rule_id });
    }
    anissia::upsert_in(&tx, &subscription.anime)?;
    let position: i64 = tx.query_row(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM rules WHERE channel_id = ?1",
        [channel_id],
        |r| r.get(0),
    )?;
    let id = insert_rule(&tx, channel_id, position, input)?;
    tx.execute(
        "INSERT INTO rule_subscriptions (rule_id, anissia_anime_no, subtitles, creator, season_id, subscribed_at)
         VALUES (?1, ?2, ?3, ?4, NULL, ?5)",
        params![
            id,
            subscription.anime.anime_no,
            subscription.subtitles.as_str(),
            subscription.creator,
            subscription.subscribed_at,
        ],
    )?;
    let created = fetch_rule(&tx, &id)?.expect("the rule was just inserted");
    tx.commit()?;
    Ok(created)
}

pub fn get_rule(conn: &Connection, id: &str) -> Result<Option<Rule>> {
    fetch_rule(conn, id)
}

pub fn list_rules(conn: &Connection, channel_id: &str) -> Result<Vec<Rule>> {
    require_channel(conn, channel_id)?;
    fetch_rules(conn, channel_id)
}

pub fn update_rule(
    conn: &mut Connection,
    id: &str,
    expected: Version,
    channel_id: &str,
    input: &RuleInput,
) -> Result<Rule> {
    input.validate()?;

    let tx = begin(conn)?;
    let current = fetch_rule(&tx, id)?.ok_or_else(|| ChannelError::NotFound {
        kind: "rule",
        id: id.to_owned(),
    })?;
    if current.channel_id != channel_id {
        return Err(ChannelError::RuleChannelChange {
            rule_id: id.to_owned(),
        });
    }
    check_version("rule", id, expected, current.version)?;
    tx.execute(
        "UPDATE rules
         SET match_text = ?2, regex = ?3, case_insensitive = ?4, directory = ?5,
             episode = ?6, episode_auto = ?7, state = ?8, version = version + 1
         WHERE id = ?1",
        params![
            id,
            input.r#match,
            input.regex,
            input.case_insensitive,
            input.directory,
            input.episode,
            input.episode_auto,
            input.state.as_str(),
        ],
    )?;
    let updated = fetch_rule(&tx, id)?.expect("the rule still exists");
    tx.commit()?;
    Ok(updated)
}

/// Sets a rule's state without a version check (the worker's archive and
/// restore; see `worker::commands::rule_archive`). The version goes up only
/// when the state changes, so an edit made meanwhile from a screen that saw
/// the old state is answered with a conflict. `None` when the rule is gone.
pub fn set_rule_state(conn: &mut Connection, id: &str, state: RuleState) -> Result<Option<Rule>> {
    let tx = begin(conn)?;
    tx.execute(
        "UPDATE rules SET state = ?2, version = version + 1 WHERE id = ?1 AND state <> ?2",
        params![id, state.as_str()],
    )?;
    let rule = fetch_rule(&tx, id)?;
    tx.commit()?;
    Ok(rule)
}

/// Applies `order` to `current` (`(id, version, position)` in current order).
/// `order` must name every current item exactly once with its current version;
/// items whose position changes get a new version.
fn apply_order(
    tx: &Transaction<'_>,
    table: &'static str,
    kind: &'static str,
    current: &[(String, Version, i64)],
    order: &[OrderItem],
) -> Result<()> {
    let by_id: HashMap<&str, (Version, i64)> = current
        .iter()
        .map(|(id, version, position)| (id.as_str(), (*version, *position)))
        .collect();
    let mut seen = HashSet::new();
    let same_set = order.len() == current.len()
        && order
            .iter()
            .all(|item| by_id.contains_key(item.id.as_str()) && seen.insert(item.id.as_str()));
    if !same_set {
        return Err(ChannelError::OrderMismatch { kind });
    }
    for item in order {
        check_version(kind, &item.id, item.version, by_id[item.id.as_str()].0)?;
    }

    let update = format!("UPDATE {table} SET position = ?1, version = version + 1 WHERE id = ?2");
    for (index, item) in order.iter().enumerate() {
        let index = index as i64;
        if by_id[item.id.as_str()].1 != index {
            tx.execute(&update, params![index, item.id])?;
        }
    }
    Ok(())
}

pub fn reorder_channels(conn: &mut Connection, order: &[OrderItem]) -> Result<Vec<Channel>> {
    let tx = begin(conn)?;
    let channels = fetch_channels(&tx)?;
    let current: Vec<_> = channels
        .iter()
        .map(|c| (c.id.clone(), c.version, c.position))
        .collect();
    apply_order(&tx, "channels", "channel", &current, order)?;
    let reordered = fetch_channels(&tx)?;
    tx.commit()?;
    Ok(reordered)
}

pub fn reorder_rules(
    conn: &mut Connection,
    channel_id: &str,
    order: &[OrderItem],
) -> Result<Vec<Rule>> {
    let tx = begin(conn)?;
    require_channel(&tx, channel_id)?;
    let rules = fetch_rules(&tx, channel_id)?;
    let current: Vec<_> = rules
        .iter()
        .map(|r| (r.id.clone(), r.version, r.position))
        .collect();
    apply_order(&tx, "rules", "rule", &current, order)?;
    let reordered = fetch_rules(&tx, channel_id)?;
    tx.commit()?;
    Ok(reordered)
}
