use super::*;
use crate::store::channels::MASK;

async fn store() -> (tempfile::TempDir, Db, HistoryStore) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    let history = HistoryStore::new(db.clone());
    (dir, db, history)
}

fn obs(key: &str, result: HistoryResult) -> Observation {
    Observation {
        channel_id: "c1".into(),
        channel_label: "http://feed.test/rss?token=***".into(),
        identity_key: key.into(),
        title: format!("title of {key}"),
        link: format!("magnet:?xt=urn:btih:{key}"),
        result,
        rule_id: None,
        torrent_hash: None,
        reason: None,
    }
}

fn received(key: &str, rule: &str, hash: &str) -> Observation {
    Observation {
        rule_id: Some(rule.into()),
        torrent_hash: Some(hash.into()),
        ..obs(key, HistoryResult::Received)
    }
}

async fn all(history: &HistoryStore) -> Vec<HistoryItem> {
    history
        .list(HistoryQuery {
            limit: MAX_PAGE_SIZE,
            ..Default::default()
        })
        .await
        .unwrap()
        .items
}

// --- identity ---------------------------------------------------------------

fn sha256_hex(text: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

#[test]
fn identity_is_the_hash_of_the_guid_then_the_link_then_the_title() {
    assert_eq!(
        identity_key(Some("g-1"), Some("http://x/1"), Some("T")),
        format!("guid:{}", sha256_hex("g-1"))
    );
    assert_eq!(
        identity_key(None, Some("http://x/1"), Some("T")),
        format!("link:{}", sha256_hex("http://x/1"))
    );
    // A blank GUID is no GUID.
    assert_eq!(
        identity_key(Some("  "), Some("http://x/1"), Some("T")),
        format!("link:{}", sha256_hex("http://x/1"))
    );
    assert_eq!(
        identity_key(Some(""), Some(""), Some("T")),
        format!("title:{}", sha256_hex("T"))
    );
    assert_eq!(
        identity_key(None, None, None),
        format!("title:{}", sha256_hex(""))
    );
}

#[test]
fn identity_source_is_part_of_the_key() {
    assert_ne!(
        identity_key(Some("http://x/1"), None, None),
        identity_key(None, Some("http://x/1"), None)
    );
}

#[test]
fn identity_keeps_items_apart_that_differ_only_in_a_secret_looking_value() {
    // The value is hashed as given, so a channel's secret query names do not
    // enter into it: these are different items whatever is secret.
    assert_ne!(
        identity_key(Some("https://t.test/details.php?id=101"), None, None),
        identity_key(Some("https://t.test/details.php?id=102"), None, None)
    );
    assert_ne!(
        identity_key(None, Some("https://t.test/dl?id=1&token=a"), None),
        identity_key(None, Some("https://t.test/dl?id=1&token=b"), None)
    );
}

#[test]
fn identity_holds_nothing_of_the_value() {
    let key = identity_key(None, Some("https://t.test/dl?token=hunter2"), None);
    assert!(!key.contains("hunter2") && !key.contains("t.test"), "{key}");

    // `<source>:` and 64 lowercase hex digits.
    let (source, digest) = key.split_once(':').unwrap();
    assert_eq!(source, "link");
    assert_eq!(digest.len(), 64);
    assert!(digest
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
}

#[test]
fn stored_link_masks_secret_query_values() {
    let secret = vec!["token".to_owned()];
    let link = stored_link(Some("https://tracker.test/dl?id=7&token=hunter2"), &secret);
    assert_eq!(link, format!("https://tracker.test/dl?id=7&token={MASK}"));
    assert_eq!(stored_link(None, &secret), "");
    // Links without secret parameters are kept as they are.
    assert_eq!(
        stored_link(Some("magnet:?xt=urn:btih:AAAA&dn=x"), &secret),
        "magnet:?xt=urn:btih:AAAA&dn=x"
    );
}

// --- results ----------------------------------------------------------------

#[test]
fn result_codes_are_stable() {
    let codes: Vec<_> = HistoryResult::ALL.iter().map(|r| r.code()).collect();
    assert_eq!(
        codes,
        [
            "received",
            "no_match",
            "excluded",
            "duplicate",
            "add_failed"
        ]
    );
    for r in HistoryResult::ALL {
        assert_eq!(HistoryResult::parse(r.code()), Some(r));
    }
    assert_eq!(HistoryResult::parse("nope"), None);
    let labels: Vec<_> = HistoryResult::ALL.iter().map(|r| r.label()).collect();
    assert_eq!(labels, ["받음", "규칙 불일치", "제외", "중복", "추가 실패"]);
}

#[test]
fn transition_table() {
    use HistoryResult::*;
    use Transition::*;

    let expected = [
        // (stored, new, transition)
        (Received, Received, Keep),
        (Received, Duplicate, Keep),
        (Received, AddFailed, Keep),
        (Received, NoMatch, Keep),
        (Received, Excluded, Keep),
        (Duplicate, Duplicate, Keep),
        (Duplicate, Received, Change),
        (Duplicate, AddFailed, Keep),
        (Duplicate, NoMatch, Keep),
        (Duplicate, Excluded, Keep),
        (NoMatch, NoMatch, Keep),
        (NoMatch, Received, Change),
        (NoMatch, Duplicate, Change),
        (NoMatch, AddFailed, Change),
        (NoMatch, Excluded, Change),
        (Excluded, Excluded, Keep),
        (Excluded, Received, Change),
        (Excluded, NoMatch, Change),
        (AddFailed, AddFailed, Refresh),
        (AddFailed, Received, Change),
        (AddFailed, Duplicate, Change),
        (AddFailed, NoMatch, Change),
        (AddFailed, Excluded, Change),
    ];
    for (stored, new, transition) in expected {
        assert_eq!(
            Transition::between(stored, new),
            transition,
            "{stored} -> {new}"
        );
    }
}

// --- recording --------------------------------------------------------------

#[tokio::test]
async fn first_sighting_creates_a_record() {
    let (_dir, _db, history) = store().await;

    let out = history
        .record(1_000, vec![received("a", "rule-1", "hash-a")])
        .await
        .unwrap();
    assert_eq!(out, [Recorded::New]);

    let items = all(&history).await;
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.channel_id, "c1");
    assert_eq!(item.identity_key, "a");
    assert_eq!(item.title, "title of a");
    assert_eq!(item.result, HistoryResult::Received);
    assert_eq!(item.rule_id.as_deref(), Some("rule-1"));
    assert_eq!(item.torrent_hash.as_deref(), Some("hash-a"));
    assert_eq!(
        (item.first_seen_at, item.last_seen_at, item.result_at),
        (1_000, 1_000, 1_000)
    );
    assert!(history.changes(item.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn seeing_an_item_again_adds_no_record_and_keeps_the_first_seen_time() {
    let (_dir, _db, history) = store().await;

    history
        .record(
            1_000,
            vec![
                obs("a", HistoryResult::NoMatch),
                obs("b", HistoryResult::Excluded),
            ],
        )
        .await
        .unwrap();
    let out = history
        .record(
            2_000,
            vec![
                obs("a", HistoryResult::NoMatch),
                obs("b", HistoryResult::Excluded),
            ],
        )
        .await
        .unwrap();
    assert_eq!(out, [Recorded::Unchanged, Recorded::Unchanged]);

    let items = all(&history).await;
    assert_eq!(items.len(), 2);
    for item in &items {
        assert_eq!(item.first_seen_at, 1_000, "first-seen time stays");
        assert_eq!(item.last_seen_at, 2_000, "last sighting moves");
        assert_eq!(item.result_at, 1_000);
        assert!(history.changes(item.id).await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn the_same_key_in_two_channels_is_two_items() {
    let (_dir, _db, history) = store().await;
    let mut other = obs("a", HistoryResult::NoMatch);
    other.channel_id = "c2".into();

    history
        .record(1_000, vec![obs("a", HistoryResult::NoMatch), other])
        .await
        .unwrap();
    assert_eq!(all(&history).await.len(), 2);
}

#[tokio::test]
async fn a_changed_result_is_recorded_as_a_change() {
    let (_dir, _db, history) = store().await;

    history
        .record(1_000, vec![obs("a", HistoryResult::NoMatch)])
        .await
        .unwrap();
    // A rule now matches and Transmission takes the item.
    let out = history
        .record(2_000, vec![received("a", "rule-9", "hash-a")])
        .await
        .unwrap();
    assert_eq!(
        out,
        [Recorded::Changed {
            from: HistoryResult::NoMatch
        }]
    );

    let items = all(&history).await;
    assert_eq!(items.len(), 1, "no second record");
    let item = &items[0];
    assert_eq!(item.result, HistoryResult::Received);
    assert_eq!(item.first_seen_at, 1_000);
    assert_eq!(item.result_at, 2_000);
    assert_eq!(item.rule_id.as_deref(), Some("rule-9"));
    assert_eq!(item.torrent_hash.as_deref(), Some("hash-a"));

    let changes = history.changes(item.id).await.unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].from, HistoryResult::NoMatch);
    assert_eq!(changes[0].to, HistoryResult::Received);
    assert_eq!(changes[0].changed_at, 2_000);
    assert_eq!(changes[0].rule_id.as_deref(), Some("rule-9"));
}

#[tokio::test]
async fn received_is_final_even_when_later_evaluations_disagree() {
    let (_dir, _db, history) = store().await;

    history
        .record(1_000, vec![received("a", "rule-1", "hash-a")])
        .await
        .unwrap();

    // The worker asks Transmission again every cycle; it answers "duplicate".
    // Transmission may also be down, or the rule may have been edited away.
    for (i, result) in [
        HistoryResult::Duplicate,
        HistoryResult::AddFailed,
        HistoryResult::NoMatch,
        HistoryResult::Excluded,
    ]
    .into_iter()
    .enumerate()
    {
        let mut later = obs("a", result);
        later.reason = Some("boom".into());
        let out = history.record(2_000 + i as i64, vec![later]).await.unwrap();
        assert_eq!(out, [Recorded::Unchanged], "{result}");
    }

    let item = &all(&history).await[0];
    assert_eq!(item.result, HistoryResult::Received);
    assert_eq!(item.rule_id.as_deref(), Some("rule-1"));
    assert_eq!(item.torrent_hash.as_deref(), Some("hash-a"));
    assert_eq!(item.reason, None);
    assert!(history.changes(item.id).await.unwrap().is_empty());
}

#[tokio::test]
async fn add_failed_keeps_the_latest_reason_and_recovers_to_received() {
    let (_dir, _db, history) = store().await;

    let failed = |reason: &str| Observation {
        rule_id: Some("rule-1".into()),
        reason: Some(reason.into()),
        ..obs("a", HistoryResult::AddFailed)
    };

    history
        .record(1_000, vec![failed("connection refused")])
        .await
        .unwrap();
    let out = history
        .record(2_000, vec![failed("timed out")])
        .await
        .unwrap();
    assert_eq!(out, [Recorded::Unchanged], "same result again is no change");

    let item = &all(&history).await[0];
    assert_eq!(item.result, HistoryResult::AddFailed);
    assert_eq!(item.reason.as_deref(), Some("timed out"));
    assert_eq!(item.result_at, 1_000);
    assert!(history.changes(item.id).await.unwrap().is_empty());

    let out = history
        .record(3_000, vec![received("a", "rule-1", "hash-a")])
        .await
        .unwrap();
    assert_eq!(
        out,
        [Recorded::Changed {
            from: HistoryResult::AddFailed
        }]
    );
    let item = &all(&history).await[0];
    assert_eq!(item.result, HistoryResult::Received);
    assert_eq!(item.reason, None, "the failure reason is cleared");
    let changes = history.changes(item.id).await.unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].from, HistoryResult::AddFailed);
}

#[tokio::test]
async fn duplicate_becomes_received_but_not_the_other_way() {
    let (_dir, _db, history) = store().await;

    history
        .record(
            1_000,
            vec![Observation {
                torrent_hash: Some("hash-a".into()),
                ..obs("a", HistoryResult::Duplicate)
            }],
        )
        .await
        .unwrap();
    history
        .record(2_000, vec![received("a", "rule-1", "hash-a")])
        .await
        .unwrap();
    history
        .record(3_000, vec![obs("a", HistoryResult::Duplicate)])
        .await
        .unwrap();

    let item = &all(&history).await[0];
    assert_eq!(item.result, HistoryResult::Received);
    assert_eq!(history.changes(item.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_settled_item_learns_a_missing_hash() {
    let (_dir, _db, history) = store().await;
    history
        .record(1_000, vec![obs("a", HistoryResult::Duplicate)])
        .await
        .unwrap();
    history
        .record(
            2_000,
            vec![Observation {
                torrent_hash: Some("hash-a".into()),
                ..obs("a", HistoryResult::Duplicate)
            }],
        )
        .await
        .unwrap();
    assert_eq!(
        all(&history).await[0].torrent_hash.as_deref(),
        Some("hash-a")
    );
}

#[tokio::test]
async fn title_and_link_follow_the_latest_sighting() {
    let (_dir, _db, history) = store().await;
    history
        .record(1_000, vec![obs("a", HistoryResult::NoMatch)])
        .await
        .unwrap();

    let mut edited = obs("a", HistoryResult::NoMatch);
    edited.title = "edited".into();
    edited.link = "magnet:?xt=urn:btih:new".into();
    history.record(2_000, vec![edited]).await.unwrap();

    let item = &all(&history).await[0];
    assert_eq!(item.title, "edited");
    assert_eq!(item.link, "magnet:?xt=urn:btih:new");
    assert_eq!(item.first_seen_at, 1_000);
}

#[tokio::test]
async fn a_batch_is_applied_in_order_and_atomically() {
    let (_dir, db, history) = store().await;

    // The second row violates a NOT NULL constraint through a corrupt value.
    let bad = Observation {
        identity_key: String::new(), // CHECK (identity_key <> '') fails
        ..obs("x", HistoryResult::NoMatch)
    };
    let err = history
        .record(1_000, vec![obs("a", HistoryResult::NoMatch), bad])
        .await;
    assert!(err.is_err());

    let count: i64 = db
        .run::<_, HistoryError, _>(|c| {
            Ok(c.query_row("SELECT count(*) FROM history_items", [], |r| r.get(0))?)
        })
        .await
        .unwrap();
    assert_eq!(count, 0, "the first row was rolled back with the batch");
}

#[tokio::test]
async fn duplicate_observation_within_one_batch_is_one_record() {
    let (_dir, _db, history) = store().await;
    let out = history
        .record(
            1_000,
            vec![
                obs("a", HistoryResult::NoMatch),
                obs("a", HistoryResult::NoMatch),
            ],
        )
        .await
        .unwrap();
    assert_eq!(out, [Recorded::New, Recorded::Unchanged]);
    assert_eq!(all(&history).await.len(), 1);
}

// --- surviving deletions ------------------------------------------------------

#[tokio::test]
async fn history_survives_channel_and_rule_deletion() {
    use crate::store::channels::{ChannelInput, ChannelStore, RuleInput};

    let (_dir, db, history) = store().await;
    let channels = ChannelStore::new(db.clone());
    let channel = channels
        .create_channel(ChannelInput::new(
            "http://feed.test/rss?token=abc",
            "/media",
        ))
        .await
        .unwrap();
    let rule = channels
        .create_rule(&channel.id, RuleInput::default())
        .await
        .unwrap();

    history
        .record(
            1_000,
            vec![Observation {
                channel_id: channel.id.clone(),
                channel_label: channel.masked_url(),
                rule_id: Some(rule.id.clone()),
                ..received("a", &rule.id, "hash-a")
            }],
        )
        .await
        .unwrap();

    // Ticket 0007 adds channel deletion; a raw delete stands in for it here.
    db.run::<_, HistoryError, _>(move |c| {
        c.execute("DELETE FROM rules", [])?;
        c.execute("DELETE FROM channels", [])?;
        Ok(())
    })
    .await
    .unwrap();

    let items = all(&history).await;
    assert_eq!(
        items.len(),
        1,
        "deleting channels and rules does not touch history"
    );
    assert_eq!(items[0].channel_id, channel.id);
    assert_eq!(items[0].rule_id.as_deref(), Some(rule.id.as_str()));
    assert!(items[0].channel_label.contains(MASK));
    assert!(!items[0].channel_label.contains("abc"));
}

// --- listing ----------------------------------------------------------------

async fn seeded(history: &HistoryStore, n: usize) {
    // Three per sighting time, so paging crosses equal first-seen times.
    let mut batch = Vec::new();
    for i in 0..n {
        let result = match i % 3 {
            0 => HistoryResult::NoMatch,
            1 => HistoryResult::Excluded,
            _ => HistoryResult::AddFailed,
        };
        batch.push(obs(&format!("k{i:03}"), result));
    }
    for (i, chunk) in batch.chunks(3).enumerate() {
        history
            .record(1_000 + i as i64, chunk.to_vec())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn list_is_newest_first_and_pages_without_gaps_or_repeats() {
    let (_dir, _db, history) = store().await;
    seeded(&history, 20).await;

    let mut seen = Vec::new();
    let mut after = None;
    let mut pages = 0;
    loop {
        let page = history
            .list(HistoryQuery {
                after,
                limit: 4,
                ..Default::default()
            })
            .await
            .unwrap();
        pages += 1;
        seen.extend(page.items.iter().map(|i| i.identity_key.clone()));
        match page.next {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    assert_eq!(pages, 5);
    assert_eq!(seen.len(), 20);

    let mut expected: Vec<String> = (0..20).map(|i| format!("k{i:03}")).collect();
    // Newest sighting time first; within one time, the later record first.
    expected.sort_by_key(|k| {
        let i: usize = k[1..].parse().unwrap();
        std::cmp::Reverse((i / 3, i))
    });
    assert_eq!(seen, expected);
}

#[tokio::test]
async fn cursor_stays_valid_when_newer_items_arrive() {
    let (_dir, _db, history) = store().await;
    seeded(&history, 9).await;

    let first = history
        .list(HistoryQuery {
            limit: 3,
            ..Default::default()
        })
        .await
        .unwrap();
    let cursor = first.next.unwrap();

    history
        .record(99_000, vec![obs("newest", HistoryResult::NoMatch)])
        .await
        .unwrap();

    let second = history
        .list(HistoryQuery {
            after: Some(cursor),
            limit: 3,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(second.items.len(), 3);
    assert!(second.items.iter().all(|i| i.identity_key != "newest"));
    assert!(first
        .items
        .iter()
        .all(|f| second.items.iter().all(|s| s.id != f.id)));
}

#[tokio::test]
async fn list_filters_by_result_and_channel() {
    let (_dir, _db, history) = store().await;
    seeded(&history, 12).await;
    let mut other = obs("other", HistoryResult::AddFailed);
    other.channel_id = "c2".into();
    history.record(5_000, vec![other]).await.unwrap();

    let failed = history
        .list(HistoryQuery {
            result: Some(HistoryResult::AddFailed),
            limit: MAX_PAGE_SIZE,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(failed.items.len(), 5);
    assert!(failed
        .items
        .iter()
        .all(|i| i.result == HistoryResult::AddFailed));
    assert_eq!(failed.items[0].identity_key, "other", "newest first");

    let c2 = history
        .list(HistoryQuery {
            channel_id: Some("c2".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(c2.items.len(), 1);

    let none = history
        .list(HistoryQuery {
            result: Some(HistoryResult::Received),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(none.items.is_empty() && none.next.is_none());
}

#[tokio::test]
async fn filtered_pages_continue_from_the_cursor() {
    let (_dir, _db, history) = store().await;
    seeded(&history, 30).await;

    let mut ids = Vec::new();
    let mut after = None;
    loop {
        let page = history
            .list(HistoryQuery {
                result: Some(HistoryResult::AddFailed),
                after,
                limit: 3,
                ..Default::default()
            })
            .await
            .unwrap();
        ids.extend(page.items.iter().map(|i| i.id));
        match page.next {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    assert_eq!(ids.len(), 10);
    let mut sorted = ids.clone();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    sorted.dedup();
    assert_eq!(sorted, ids, "strictly descending, no repeats");
}

#[test]
fn cursor_round_trips_through_text() {
    let c = HistoryCursor {
        first_seen_at: 1_700_000_000_123,
        id: 42,
    };
    assert_eq!(c.to_string().parse::<HistoryCursor>(), Ok(c));
    assert!("garbage".parse::<HistoryCursor>().is_err());
    assert!("1.x".parse::<HistoryCursor>().is_err());
}

// --- cycle marker -----------------------------------------------------------

#[tokio::test]
async fn cycle_marker_admits_one_start_per_gap() {
    let (_dir, _db, history) = store().await;
    assert_eq!(history.last_cycle().await.unwrap(), None);

    assert!(history.try_begin_cycle(10_000, 5_000).await.unwrap());
    assert!(!history.try_begin_cycle(12_000, 5_000).await.unwrap());
    assert_eq!(
        history.last_cycle().await.unwrap(),
        Some(CycleState {
            started_at: 10_000,
            finished_at: None
        })
    );

    history.finish_cycle(11_000).await.unwrap();
    assert_eq!(
        history.last_cycle().await.unwrap().unwrap().finished_at,
        Some(11_000)
    );

    assert!(history.try_begin_cycle(15_000, 5_000).await.unwrap());
    // A clock that went backwards does not block the worker.
    assert!(history.try_begin_cycle(1_000, 5_000).await.unwrap());
}

#[tokio::test]
async fn two_handles_race_for_one_cycle_start() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    let a = HistoryStore::new(Db::open(&path).await.unwrap());
    let b = HistoryStore::new(Db::open(&path).await.unwrap());

    let (ra, rb) = tokio::join!(
        a.try_begin_cycle(50_000, 10_000),
        b.try_begin_cycle(50_000, 10_000)
    );
    let started = [ra.unwrap(), rb.unwrap()];
    assert_eq!(started.iter().filter(|s| **s).count(), 1);
}

#[tokio::test]
async fn migration_adds_history_to_a_database_that_only_has_channels() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    {
        // A database as version 1 left it.
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(include_str!("../channels/schema.sql"))
            .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
    }
    let db = Db::open(&path).await.unwrap();
    let history = HistoryStore::new(db);
    history
        .record(1_000, vec![obs("a", HistoryResult::NoMatch)])
        .await
        .unwrap();
    assert_eq!(all(&history).await.len(), 1);
}
