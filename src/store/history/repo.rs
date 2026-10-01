//! Synchronous SQL for the collection history. Every write runs in one
//! `BEGIN IMMEDIATE` transaction.

use rusqlite::types::Type;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row, TransactionBehavior};

use super::model::{
    CycleState, HistoryChange, HistoryCursor, HistoryItem, HistoryPage, HistoryQuery,
    HistoryResult, Millis, Observation, Recorded, Transition, MAX_PAGE_SIZE,
};
use super::HistoryError;

type Result<T> = std::result::Result<T, HistoryError>;

const ITEM_COLUMNS: &str = "id, channel_id, channel_label, identity_key, title, link, \
     first_seen_at, last_seen_at, result, result_at, rule_id, reason, torrent_hash";

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
    })
}

/// Records all observations in one transaction, in order. The outcome list
/// matches the input list.
pub fn record(
    conn: &mut Connection,
    at: Millis,
    observations: &[Observation],
) -> Result<Vec<Recorded>> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut out = Vec::with_capacity(observations.len());

    for obs in observations {
        let stored: Option<(i64, String)> = tx
            .query_row(
                "SELECT id, result FROM history_items WHERE channel_id = ?1 AND identity_key = ?2",
                params![obs.channel_id, obs.identity_key],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;

        let Some((id, code)) = stored else {
            tx.execute(
                "INSERT INTO history_items (channel_id, channel_label, identity_key, title, link,
                     first_seen_at, last_seen_at, result, result_at, rule_id, reason, torrent_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7, ?6, ?8, ?9, ?10)",
                params![
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
                ],
            )?;
            out.push(Recorded::New);
            continue;
        };

        let stored_result = parse_result(1, &code)?;

        // Every sighting refreshes when the item was last in the feed and its
        // latest title and link; `first_seen_at` is never touched.
        tx.execute(
            "UPDATE history_items
             SET last_seen_at = max(last_seen_at, ?2), title = ?3, link = ?4, channel_label = ?5
             WHERE id = ?1",
            params![id, at, obs.title, obs.link, obs.channel_label],
        )?;

        match Transition::between(stored_result, obs.result) {
            Transition::Keep => {
                // A settled item whose hash was unknown learns it.
                if obs.torrent_hash.is_some() && stored_result.is_settled() {
                    tx.execute(
                        "UPDATE history_items SET torrent_hash = ?2
                         WHERE id = ?1 AND torrent_hash IS NULL",
                        params![id, obs.torrent_hash],
                    )?;
                }
                out.push(Recorded::Unchanged);
            }
            Transition::Refresh => {
                tx.execute(
                    "UPDATE history_items SET rule_id = ?2, reason = ?3 WHERE id = ?1",
                    params![id, obs.rule_id, obs.reason],
                )?;
                out.push(Recorded::Unchanged);
            }
            Transition::Change => {
                tx.execute(
                    "UPDATE history_items
                     SET result = ?2, result_at = ?3, rule_id = ?4, reason = ?5,
                         torrent_hash = COALESCE(?6, torrent_hash)
                     WHERE id = ?1",
                    params![
                        id,
                        obs.result.code(),
                        at,
                        obs.rule_id,
                        obs.reason,
                        obs.torrent_hash
                    ],
                )?;
                tx.execute(
                    "INSERT INTO history_changes
                         (item_id, changed_at, from_result, to_result, rule_id, reason, torrent_hash)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
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

    tx.commit()?;
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
    let mut stmt = conn.prepare(
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

/// Marks a cycle as started unless the previous one started less than
/// `min_gap` ago. The check and the mark are one write transaction.
pub fn try_begin_cycle(conn: &mut Connection, now: Millis, min_gap: Millis) -> Result<bool> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let last: Option<Millis> = tx
        .query_row(
            "SELECT started_at FROM collection_cycle WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    // `now < last` means the clock went backwards; do not wait for it to catch up.
    if last.is_some_and(|last| now >= last && now - last < min_gap) {
        return Ok(false);
    }
    tx.execute(
        "INSERT INTO collection_cycle (id, started_at, finished_at) VALUES (1, ?1, NULL)
         ON CONFLICT (id) DO UPDATE SET started_at = ?1, finished_at = NULL",
        [now],
    )?;
    tx.commit()?;
    Ok(true)
}

pub fn finish_cycle(conn: &Connection, now: Millis) -> Result<()> {
    conn.execute(
        "UPDATE collection_cycle SET finished_at = ?1 WHERE id = 1",
        [now],
    )?;
    Ok(())
}

pub fn get(conn: &Connection, id: i64) -> Result<Option<HistoryItem>> {
    Ok(conn
        .query_row(
            &format!("SELECT {ITEM_COLUMNS} FROM history_items WHERE id = ?1"),
            [id],
            item_from_row,
        )
        .optional()?)
}

/// How many items have each result, optionally within one channel.
pub fn counts(conn: &Connection, channel_id: Option<&str>) -> Result<Vec<(HistoryResult, i64)>> {
    let mut stmt = conn.prepare(
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
    let changed = conn.execute(
        "UPDATE history_items SET reason = ?2
         WHERE id = ?1 AND result = 'received' AND reason IS NULL",
        params![item_id, note],
    )?;
    Ok(changed > 0)
}

pub fn received_by_hand(conn: &Connection, hash: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM history_items
                        WHERE torrent_hash = ?1 AND result = 'received' AND rule_id IS NULL)",
        [hash],
        |row| row.get(0),
    )?)
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

/// When each of the given items of a channel was first seen and what became
/// of it, by identity key. Keys the channel has no record of are left out.
pub fn known_items(
    conn: &Connection,
    channel_id: &str,
    keys: &[String],
) -> Result<std::collections::HashMap<String, (Millis, HistoryResult)>> {
    // Keeps the number of bound values well under SQLite's limit.
    const CHUNK: usize = 400;

    let mut known = std::collections::HashMap::new();
    for chunk in keys.chunks(CHUNK) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        let mut stmt = conn.prepare(&format!(
            "SELECT identity_key, first_seen_at, result FROM history_items
             WHERE channel_id = ? AND identity_key IN ({placeholders})"
        ))?;
        let args = std::iter::once(channel_id).chain(chunk.iter().map(String::as_str));
        let rows = stmt
            .query_map(params_from_iter(args), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Millis>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (key, first_seen_at, code) in rows {
            known.insert(key, (first_seen_at, parse_result(2, &code)?));
        }
    }
    Ok(known)
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
    let stored: Option<String> = tx
        .query_row(
            "SELECT result FROM history_items WHERE id = ?1",
            [item_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(code) = stored else {
        return Ok(None);
    };
    let stored_result = parse_result(0, &code)?;

    let after = match Transition::between(stored_result, result) {
        Transition::Keep => {
            if torrent_hash.is_some() && stored_result.is_settled() {
                tx.execute(
                    "UPDATE history_items SET torrent_hash = ?2
                     WHERE id = ?1 AND torrent_hash IS NULL",
                    params![item_id, torrent_hash],
                )?;
            }
            stored_result
        }
        Transition::Refresh => {
            tx.execute(
                "UPDATE history_items SET rule_id = ?2, reason = ?3 WHERE id = ?1",
                params![item_id, rule_id, reason],
            )?;
            stored_result
        }
        Transition::Change => {
            tx.execute(
                "UPDATE history_items
                 SET result = ?2, result_at = ?3, rule_id = ?4, reason = ?5,
                     torrent_hash = COALESCE(?6, torrent_hash)
                 WHERE id = ?1",
                params![item_id, result.code(), at, rule_id, reason, torrent_hash],
            )?;
            tx.execute(
                "INSERT INTO history_changes
                     (item_id, changed_at, from_result, to_result, rule_id, reason, torrent_hash)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    item_id,
                    at,
                    stored_result.code(),
                    result.code(),
                    rule_id,
                    reason,
                    torrent_hash
                ],
            )?;
            result
        }
    };

    tx.commit()?;
    Ok(Some(after))
}

pub fn last_cycle(conn: &Connection) -> Result<Option<CycleState>> {
    Ok(conn
        .query_row(
            "SELECT started_at, finished_at FROM collection_cycle WHERE id = 1",
            [],
            |row| {
                Ok(CycleState {
                    started_at: row.get(0)?,
                    finished_at: row.get(1)?,
                })
            },
        )
        .optional()?)
}
