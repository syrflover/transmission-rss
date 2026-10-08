//! Synchronous SQL for the collection history. Every write runs in one
//! `BEGIN IMMEDIATE` transaction.

use rusqlite::types::Type;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row, TransactionBehavior};

use crate::store::history::{
    model::{
        CycleState, HistoryChange, HistoryCursor, HistoryItem, HistoryPage, HistoryQuery,
        HistoryResult, KnownItem, Observation, Recorded, Transition, MAX_PAGE_SIZE,
    },
    HistoryError,
};
use trss_core::Millis;

type Result<T> = std::result::Result<T, HistoryError>;

const ITEM_COLUMNS: &str = "id, channel_id, channel_label, identity_key, title, link, \
     first_seen_at, last_seen_at, result, result_at, rule_id, reason, torrent_hash, first_read";

fn parse_result(idx: usize, code: &str) -> rusqlite::Result<HistoryResult> {
    HistoryResult::parse(code).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            idx,
            Type::Text,
            format!("unknown history result code {code:?}").into(),
        )
    })
}

fn item_from_row(row: &Row<'_>) -> rusqlite::Result<HistoryItem> {
    let code: String = row.get(8)?;
    Ok(HistoryItem {
        id: row.get(0)?,
        channel_id: row.get(1)?,
        channel_label: row.get(2)?,
        identity_key: row.get(3)?,
        title: row.get(4)?,
        link: row.get(5)?,
        first_seen_at: row.get(6)?,
        last_seen_at: row.get(7)?,
        result: parse_result(8, &code)?,
        result_at: row.get(9)?,
        rule_id: row.get(10)?,
        reason: row.get(11)?,
        torrent_hash: row.get(12)?,
        first_read: row.get(13)?,
    })
}

/// Where a sighting was made. Only a read of the channel's feed can be its
/// first read; an item recorded from anywhere else (the past search, which
/// reads a tracker's search feed) says nothing of what the channel's own feed
/// held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The worker read the channel's feed.
    Feed,
    /// Some other read of the tracker: it never establishes the channel's
    /// first read, nor is its item marked as part of it.
    Elsewhere,
}

/// Records all observations in one transaction, in order. The outcome list
/// matches the input list.
pub fn record(
    conn: &mut Connection,
    at: Millis,
    origin: Origin,
    observations: &[Observation],
) -> Result<Vec<Recorded>> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let out = record_in(&tx, at, origin, observations)?;
    tx.commit()?;
    Ok(out)
}

/// [`record`] of one observation, with the ID of its item.
pub fn record_one(
    conn: &mut Connection,
    at: Millis,
    origin: Origin,
    observation: &Observation,
) -> Result<(i64, Recorded)> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let recorded = record_in(&tx, at, origin, std::slice::from_ref(observation))?
        .pop()
        .expect("one observation is recorded once");
    let id = tx
        .prepare_cached("SELECT id FROM history_items WHERE channel_id = ?1 AND identity_key = ?2")?
        .query_row(
            params![observation.channel_id, observation.identity_key],
            |row| row.get(0),
        )?;
    tx.commit()?;
    Ok((id, recorded))
}

/// [`record`] inside the caller's transaction `tx`, which another store
/// writes in too (`store::revisions` writes a revision row with its item).
pub fn record_in(
    tx: &Connection,
    at: Millis,
    origin: Origin,
    observations: &[Observation],
) -> Result<Vec<Recorded>> {
    let mut out = Vec::with_capacity(observations.len());

    for obs in observations {
        let stored: Option<(i64, String)> = tx
            .prepare_cached(
                "SELECT id, result FROM history_items WHERE channel_id = ?1 AND identity_key = ?2",
            )?
            .query_row(params![obs.channel_id, obs.identity_key], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?;

        let Some((id, code)) = stored else {
            // The channel's first record of its feed is its first read, and
            // the records of that same cycle (the same `at`) belong to it.
            // Which they are is kept with each item, so that no later
            // comparison of times can move the line, whatever the clock did.
            // A record from elsewhere is neither: it leaves the first read to
            // the cycle that reads the feed.
            let first_read = match origin {
                Origin::Elsewhere => false,
                Origin::Feed => {
                    let first_read_cycle: Option<bool> = tx
                        .prepare_cached(
                            "SELECT first_read_at = ?2 FROM history_first_reads
                             WHERE channel_id = ?1",
                        )?
                        .query_row(params![obs.channel_id, at], |row| row.get(0))
                        .optional()?;
                    match first_read_cycle {
                        Some(same_cycle) => same_cycle,
                        None => {
                            tx.prepare_cached(
                                "INSERT INTO history_first_reads (channel_id, first_read_at)
                                 VALUES (?1, ?2)",
                            )?
                            .execute(params![obs.channel_id, at])?;
                            true
                        }
                    }
                }
            };
            tx.prepare_cached(
                "INSERT INTO history_items (channel_id, channel_label, identity_key, title, link,
                     first_seen_at, last_seen_at, result, result_at, rule_id, reason, torrent_hash,
                     first_read)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?6, ?8, ?9, ?10, ?11)",
            )?
            .execute(params![
                obs.channel_id,
                obs.channel_label,
                obs.identity_key,
                obs.title,
                obs.link,
                at,
                obs.result.code(),
                obs.rule_id,
                obs.reason,
                obs.torrent_hash,
                first_read,
            ])?;
            out.push(Recorded::New);
            continue;
        };

        let stored_result = parse_result(1, &code)?;

        // Every sighting refreshes when the item was last in the feed and its
        // latest title and link; `first_seen_at` is never touched.
        tx.prepare_cached(
            "UPDATE history_items
             SET last_seen_at = max(last_seen_at, ?2), title = ?3, link = ?4, channel_label = ?5
             WHERE id = ?1",
        )?
        .execute(params![id, at, obs.title, obs.link, obs.channel_label])?;

        match Transition::between(stored_result, obs.result) {
            Transition::Keep => {
                // A settled item whose hash was unknown learns it.
                if obs.torrent_hash.is_some() && stored_result.is_settled() {
                    tx.prepare_cached(
                        "UPDATE history_items SET torrent_hash = ?2
                         WHERE id = ?1 AND torrent_hash IS NULL",
                    )?
                    .execute(params![id, obs.torrent_hash])?;
                }
                out.push(Recorded::Unchanged);
            }
            Transition::Refresh => {
                tx.prepare_cached(
                    "UPDATE history_items SET rule_id = ?2, reason = ?3 WHERE id = ?1",
                )?
                .execute(params![id, obs.rule_id, obs.reason])?;
                out.push(Recorded::Unchanged);
            }
            Transition::Change => {
                tx.prepare_cached(
                    "UPDATE history_items
                     SET result = ?2, result_at = ?3, rule_id = ?4, reason = ?5,
                         torrent_hash = COALESCE(?6, torrent_hash)
                     WHERE id = ?1",
                )?
                .execute(params![
                    id,
                    obs.result.code(),
                    at,
                    obs.rule_id,
                    obs.reason,
                    obs.torrent_hash
                ])?;
                tx.prepare_cached("INSERT INTO history_changes
                         (item_id, changed_at, from_result, to_result, rule_id, reason, torrent_hash)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)")?.execute(
                    params![
                        id,
                        at,
                        stored_result.code(),
                        obs.result.code(),
                        obs.rule_id,
                        obs.reason,
                        obs.torrent_hash
                    ],
                )?;
                out.push(Recorded::Changed {
                    from: stored_result,
                });
            }
        }
    }

    Ok(out)
}

pub fn list(conn: &Connection, query: &HistoryQuery) -> Result<HistoryPage> {
    let limit = query.limit.clamp(1, MAX_PAGE_SIZE);

    let mut sql = format!("SELECT {ITEM_COLUMNS} FROM history_items WHERE 1 = 1");
    let mut args: Vec<rusqlite::types::Value> = Vec::new();

    if let Some(result) = query.result {
        args.push(result.code().to_owned().into());
        sql.push_str(&format!(" AND result = ?{}", args.len()));
    }
    if !query.results.is_empty() {
        let mut marks = Vec::new();
        for result in &query.results {
            args.push(result.code().to_owned().into());
            marks.push(format!("?{}", args.len()));
        }
        sql.push_str(&format!(" AND result IN ({})", marks.join(", ")));
    }
    if let Some(channel_id) = &query.channel_id {
        args.push(channel_id.clone().into());
        sql.push_str(&format!(" AND channel_id = ?{}", args.len()));
    }
    if let Some(after) = query.after {
        args.push(after.first_seen_at.into());
        args.push(after.id.into());
        sql.push_str(&format!(
            " AND (first_seen_at, id) < (?{}, ?{})",
            args.len() - 1,
            args.len()
        ));
    }
    // One extra row tells whether another page follows.
    args.push(((limit + 1) as i64).into());
    sql.push_str(&format!(
        " ORDER BY first_seen_at DESC, id DESC LIMIT ?{}",
        args.len()
    ));

    let mut stmt = conn.prepare(&sql)?;
    let mut items = stmt
        .query_map(params_from_iter(args), item_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let next = if items.len() > limit {
        items.truncate(limit);
        items.last().map(|last| HistoryCursor {
            first_seen_at: last.first_seen_at,
            id: last.id,
        })
    } else {
        None
    };

    Ok(HistoryPage { items, next })
}

pub fn changes(conn: &Connection, item_id: i64) -> Result<Vec<HistoryChange>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, item_id, changed_at, from_result, to_result, rule_id, reason, torrent_hash
         FROM history_changes WHERE item_id = ?1 ORDER BY id",
    )?;
    let rows = stmt
        .query_map([item_id], |row| {
            let from: String = row.get(3)?;
            let to: String = row.get(4)?;
            Ok(HistoryChange {
                id: row.get(0)?,
                item_id: row.get(1)?,
                changed_at: row.get(2)?,
                from: parse_result(3, &from)?,
                to: parse_result(4, &to)?,
                rule_id: row.get(5)?,
                reason: row.get(6)?,
                torrent_hash: row.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// The torrent hashes recorded for the items of the given channels.
pub fn torrent_hashes_of_channels(
    conn: &Connection,
    channel_ids: &[String],
) -> Result<Vec<String>> {
    if channel_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = vec!["?"; channel_ids.len()].join(", ");
    let mut stmt = conn.prepare(&format!(
        "SELECT DISTINCT torrent_hash FROM history_items
         WHERE torrent_hash IS NOT NULL AND channel_id IN ({placeholders})"
    ))?;
    let hashes = stmt
        .query_map(params_from_iter(channel_ids), |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(hashes)
}

/// How many rule IDs one query of [`received_hashes_of_rules`] binds, which
/// keeps the bound values well under SQLite's limit.
const RECEIVED_HASHES_CHUNK: usize = 400;

/// The query of [`received_hashes_of_rules`] for `rules` rule IDs. It reads
/// through `history_items_by_rule`, so its cost follows the rules asked about,
/// not the whole history.
pub(super) fn received_hashes_sql(rules: usize) -> String {
    let placeholders = vec!["?"; rules].join(", ");
    format!(
        "SELECT rule_id, torrent_hash FROM history_items
          WHERE result = 'received' AND torrent_hash IS NOT NULL
            AND rule_id IN ({placeholders})"
    )
}

/// The torrent hashes of the items each of the given rules received
/// (`received` with the rule recorded), by rule ID, each rule's hashes sorted
/// and without repeats. A rule that received nothing is not in the map.
pub fn received_hashes_of_rules(
    conn: &Connection,
    rule_ids: &[String],
) -> Result<std::collections::HashMap<String, Vec<String>>> {
    let mut by_rule: std::collections::HashMap<String, std::collections::BTreeSet<String>> =
        Default::default();
    for chunk in rule_ids.chunks(RECEIVED_HASHES_CHUNK) {
        let mut stmt = conn.prepare(&received_hashes_sql(chunk.len()))?;
        let rows = stmt
            .query_map(params_from_iter(chunk), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (rule_id, hash) in rows {
            by_rule.entry(rule_id).or_default().insert(hash);
        }
    }
    Ok(by_rule
        .into_iter()
        .map(|(rule, hashes)| (rule, hashes.into_iter().collect()))
        .collect())
}

/// The release title and torrent hash of each item each of the given rules
/// received (`received` with the rule recorded and a torrent hash), by rule ID,
/// restricted to the torrents named in `hashes`. It reads through
/// `history_items_by_rule` like [`received_hashes_of_rules`] and filters on the
/// hash in the query, so what is read follows the rules and the torrents asked
/// about, not everything the rules ever received.
pub fn received_titles_of_rules(
    conn: &Connection,
    rule_ids: &[String],
    hashes: &[String],
) -> Result<std::collections::HashMap<String, Vec<(String, String)>>> {
    let mut by_rule: std::collections::HashMap<String, Vec<(String, String)>> = Default::default();
    for rules in rule_ids.chunks(RECEIVED_HASHES_CHUNK) {
        for hashes in hashes.chunks(RECEIVED_HASHES_CHUNK) {
            let rule_marks = vec!["?"; rules.len()].join(", ");
            let hash_marks = vec!["?"; hashes.len()].join(", ");
            let mut stmt = conn.prepare(&format!(
                "SELECT rule_id, torrent_hash, title FROM history_items
                  WHERE result = 'received' AND rule_id IN ({rule_marks})
                    AND torrent_hash IN ({hash_marks})"
            ))?;
            let rows = stmt
                .query_map(params_from_iter(rules.iter().chain(hashes)), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for (rule_id, hash, title) in rows {
                by_rule.entry(rule_id).or_default().push((hash, title));
            }
        }
    }
    Ok(by_rule)
}

/// Marks a cycle as started unless the previous one started less than
/// `min_gap` ago. The check and the mark are one write transaction.
pub fn try_begin_cycle(conn: &mut Connection, now: Millis, min_gap: Millis) -> Result<bool> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let last: Option<Millis> = tx
        .prepare_cached("SELECT started_at FROM collection_cycle WHERE id = 1")?
        .query_row([], |row| row.get(0))
        .optional()?;
    // `now < last` means the clock went backwards; do not wait for it to catch up.
    if last.is_some_and(|last| now >= last && now - last < min_gap) {
        return Ok(false);
    }
    tx.prepare_cached(
        "INSERT INTO collection_cycle (id, started_at, finished_at) VALUES (1, ?1, NULL)
         ON CONFLICT (id) DO UPDATE SET started_at = ?1, finished_at = NULL",
    )?
    .execute([now])?;
    tx.commit()?;
    Ok(true)
}

pub fn finish_cycle(conn: &Connection, now: Millis) -> Result<()> {
    conn.prepare_cached("UPDATE collection_cycle SET finished_at = ?1 WHERE id = 1")?
        .execute([now])?;
    Ok(())
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<HistoryItem>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {ITEM_COLUMNS} FROM history_items WHERE id = ?1"
        ))?
        .query_row([id], item_from_row)
        .optional()?)
}

/// The item `identity_key` of the channel `channel_id`.
pub fn item_by_key(
    conn: &Connection,
    channel_id: &str,
    identity_key: &str,
) -> Result<Option<HistoryItem>> {
    Ok(conn
        .prepare_cached(&format!(
            "SELECT {ITEM_COLUMNS} FROM history_items
                  WHERE channel_id = ?1 AND identity_key = ?2"
        ))?
        .query_row([channel_id, identity_key], item_from_row)
        .optional()?)
}

/// The items that record the torrent `hash`, newest record first.
pub fn items_of_hash(conn: &Connection, hash: &str) -> Result<Vec<HistoryItem>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {ITEM_COLUMNS} FROM history_items WHERE torrent_hash = ?1 ORDER BY id DESC"
    ))?;
    let items = stmt
        .query_map([hash], item_from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(items)
}

/// The ID and title of every item of the channel `channel_id`.
pub fn titles_of_channel(conn: &Connection, channel_id: &str) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn
        .prepare_cached("SELECT id, title FROM history_items WHERE channel_id = ?1 ORDER BY id")?;
    let titles = stmt
        .query_map([channel_id], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(titles)
}

/// The ID and title of the `limit` newest records of the channel, newest first.
pub fn recent_titles_of_channel(
    conn: &Connection,
    channel_id: &str,
    limit: usize,
) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT id, title FROM history_items WHERE channel_id = ?1 ORDER BY id DESC LIMIT ?2",
    )?;
    let titles = stmt
        .query_map(params![channel_id, limit as i64], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(titles)
}

/// How many items have each result, optionally within one channel.
pub fn counts(conn: &Connection, channel_id: Option<&str>) -> Result<Vec<(HistoryResult, i64)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT result, count(*) FROM history_items
         WHERE (?1 IS NULL OR channel_id = ?1) GROUP BY result",
    )?;
    let rows = stmt
        .query_map([channel_id], |row| {
            let code: String = row.get(0)?;
            Ok((parse_result(0, &code)?, row.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Sets the note of a `received` item that has none (its `reason` column);
/// any other item is left alone. Returns whether a note was written.
pub fn note_received(conn: &Connection, item_id: i64, note: &str) -> Result<bool> {
    let changed = conn
        .prepare_cached(
            "UPDATE history_items SET reason = ?2
         WHERE id = ?1 AND result = 'received' AND reason IS NULL",
        )?
        .execute(params![item_id, note])?;
    Ok(changed > 0)
}

pub fn received_by_hand(conn: &Connection, hash: &str) -> Result<bool> {
    Ok(conn
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM history_items
                        WHERE torrent_hash = ?1 AND result = 'received' AND rule_id IS NULL)",
        )?
        .query_row([hash], |row| row.get(0))?)
}

/// The torrent hashes of the items among the given `(channel_id,
/// identity_key)` pairs that Transmission holds a torrent for (`received` or
/// `duplicate`).
pub fn held_hashes_of_items(conn: &Connection, items: &[(String, String)]) -> Result<Vec<String>> {
    // Keeps the number of bound values well under SQLite's limit.
    const CHUNK: usize = 400;

    let mut by_channel: std::collections::BTreeMap<&str, Vec<&str>> = Default::default();
    for (channel_id, key) in items {
        by_channel.entry(channel_id).or_default().push(key);
    }

    let mut hashes = Vec::new();
    for (channel_id, keys) in by_channel {
        for chunk in keys.chunks(CHUNK) {
            let placeholders = vec!["?"; chunk.len()].join(", ");
            let mut stmt = conn.prepare(&format!(
                "SELECT DISTINCT torrent_hash FROM history_items
                 WHERE result IN ('received', 'duplicate') AND torrent_hash IS NOT NULL
                   AND channel_id = ? AND identity_key IN ({placeholders})"
            ))?;
            let args = std::iter::once(channel_id).chain(chunk.iter().copied());
            let found = stmt
                .query_map(params_from_iter(args), |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            hashes.extend(found);
        }
    }
    Ok(hashes)
}

/// When each of the given items of a channel was first seen, what became of it
/// and whether the first read recorded it, by identity key. Keys the channel
/// has no record of are left out.
pub fn known_items(
    conn: &Connection,
    channel_id: &str,
    keys: &[String],
) -> Result<std::collections::HashMap<String, KnownItem>> {
    // Keeps the number of bound values well under SQLite's limit.
    const CHUNK: usize = 400;

    let mut known = std::collections::HashMap::new();
    for chunk in keys.chunks(CHUNK) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let mut stmt = conn.prepare(&format!(
            "SELECT identity_key, first_seen_at, result, first_read FROM history_items
             WHERE channel_id = ? AND identity_key IN ({placeholders})"
        ))?;
        let args = std::iter::once(channel_id).chain(chunk.iter().map(String::as_str));
        let rows = stmt
            .query_map(params_from_iter(args), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Millis>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (key, first_seen_at, code, first_read) in rows {
            known.insert(
                key,
                KnownItem {
                    first_seen_at,
                    result: parse_result(2, &code)?,
                    first_read,
                },
            );
        }
    }
    Ok(known)
}

/// When each of the given channels was first read, by channel ID: the time
/// of its first record, stored once and never changed (`history_first_reads`).
/// A channel with no record is left out. One key lookup per channel.
pub fn first_sightings(
    conn: &Connection,
    channel_ids: &[String],
) -> Result<std::collections::HashMap<String, Millis>> {
    let mut stmt =
        conn.prepare_cached("SELECT first_read_at FROM history_first_reads WHERE channel_id = ?1")?;
    let mut found = std::collections::HashMap::new();
    for channel_id in channel_ids {
        let first: Option<Millis> = stmt.query_row([channel_id], |row| row.get(0)).optional()?;
        if let Some(first) = first {
            found.insert(channel_id.clone(), first);
        }
    }
    Ok(found)
}

/// The query of [`last_received_of_rules`] for one rule.
pub(super) const LAST_RECEIVED_SQL: &str =
    "SELECT MAX(result_at) FROM history_items WHERE rule_id = ?1 AND result = 'received'";

/// The query of [`titles_since`].
pub(super) const TITLES_SINCE_SQL: &str = "SELECT title FROM history_items
      WHERE channel_id = ?1 AND first_seen_at > ?2
      ORDER BY first_seen_at DESC, id DESC LIMIT ?3";

/// When each of the given rules last got an item into Transmission (the newest
/// `result_at` of its `received` items), by rule ID. A rule that received
/// nothing is not in the map. One lookup of `history_items_by_rule` per rule,
/// however long the history is.
pub fn last_received_of_rules(
    conn: &Connection,
    rule_ids: &[String],
) -> Result<std::collections::HashMap<String, Millis>> {
    let mut stmt = conn.prepare_cached(LAST_RECEIVED_SQL)?;
    let mut found = std::collections::HashMap::new();
    for rule_id in rule_ids {
        let last: Option<Millis> = stmt.query_row([rule_id], |row| row.get(0))?;
        if let Some(last) = last {
            found.insert(rule_id.clone(), last);
        }
    }
    Ok(found)
}

/// The titles of the channel's items first seen after `since`, newest
/// first, at most `limit` of them, and whether there were more. Reads through
/// `history_items_by_channel`, so its cost follows the window, not the history.
pub fn titles_since(
    conn: &Connection,
    channel_id: &str,
    since: Millis,
    limit: usize,
) -> Result<(Vec<String>, bool)> {
    let mut stmt = conn.prepare_cached(TITLES_SINCE_SQL)?;
    let mut titles = stmt
        .query_map(params![channel_id, since, limit as i64 + 1], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let truncated = titles.len() > limit;
    titles.truncate(limit);
    Ok((titles, truncated))
}

/// Sets an item's result from something done to it outside a collection cycle
/// (a command from the web), by the same transition rules as [`record`]. The
/// item was not seen in a feed, so `last_seen_at`, the title and the link stay.
/// Returns the item's result afterwards, or `None` when there is no such item.
pub fn record_outcome(
    conn: &mut Connection,
    item_id: i64,
    at: Millis,
    result: HistoryResult,
    rule_id: Option<&str>,
    reason: Option<&str>,
    torrent_hash: Option<&str>,
) -> Result<Option<HistoryResult>> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let after = record_outcome_in(&tx, item_id, at, result, rule_id, reason, torrent_hash)?;
    tx.commit()?;
    Ok(after)
}

/// [`record_outcome`] inside the caller's transaction `tx`.
pub fn record_outcome_in(
    tx: &Connection,
    item_id: i64,
    at: Millis,
    result: HistoryResult,
    rule_id: Option<&str>,
    reason: Option<&str>,
    torrent_hash: Option<&str>,
) -> Result<Option<HistoryResult>> {
    let stored: Option<String> = tx
        .prepare_cached("SELECT result FROM history_items WHERE id = ?1")?
        .query_row([item_id], |row| row.get(0))
        .optional()?;
    let Some(code) = stored else {
        return Ok(None);
    };
    let stored_result = parse_result(0, &code)?;

    let after = match Transition::between(stored_result, result) {
        Transition::Keep => {
            if torrent_hash.is_some() && stored_result.is_settled() {
                tx.prepare_cached(
                    "UPDATE history_items SET torrent_hash = ?2
                     WHERE id = ?1 AND torrent_hash IS NULL",
                )?
                .execute(params![item_id, torrent_hash])?;
            }
            stored_result
        }
        Transition::Refresh => {
            tx.prepare_cached("UPDATE history_items SET rule_id = ?2, reason = ?3 WHERE id = ?1")?
                .execute(params![item_id, rule_id, reason])?;
            stored_result
        }
        Transition::Change => {
            tx.prepare_cached(
                "UPDATE history_items
                 SET result = ?2, result_at = ?3, rule_id = ?4, reason = ?5,
                     torrent_hash = COALESCE(?6, torrent_hash)
                 WHERE id = ?1",
            )?
            .execute(params![
                item_id,
                result.code(),
                at,
                rule_id,
                reason,
                torrent_hash
            ])?;
            tx.prepare_cached(
                "INSERT INTO history_changes
                     (item_id, changed_at, from_result, to_result, rule_id, reason, torrent_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?
            .execute(params![
                item_id,
                at,
                stored_result.code(),
                result.code(),
                rule_id,
                reason,
                torrent_hash
            ])?;
            result
        }
    };

    Ok(Some(after))
}

pub fn last_cycle(conn: &Connection) -> Result<Option<CycleState>> {
    Ok(conn
        .prepare_cached("SELECT started_at, finished_at FROM collection_cycle WHERE id = 1")?
        .query_row([], |row| {
            Ok(CycleState {
                started_at: row.get(0)?,
                finished_at: row.get(1)?,
            })
        })
        .optional()?)
}
