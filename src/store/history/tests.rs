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
    assert_eq!(
        labels,
        ["추가함", "규칙 불일치", "제외", "중복", "추가 실패"]
    );
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
async fn a_channels_first_sighting_is_its_earliest_first_seen_time() {
    let (_dir, _db, history) = store().await;
    history
        .record(2_000, vec![obs("a", HistoryResult::NoMatch)])
        .await
        .unwrap();
    history
        .record(
            1_000,
            vec![Observation {
                channel_id: "c2".into(),
                ..obs("b", HistoryResult::NoMatch)
            }],
        )
        .await
        .unwrap();
    // Seeing "a" again, and a newer item, move nothing.
    history
        .record(
            3_000,
            vec![
                obs("a", HistoryResult::NoMatch),
                obs("c", HistoryResult::NoMatch),
            ],
        )
        .await
        .unwrap();

    let found = history
        .first_sightings(vec!["c1".into(), "c2".into(), "unknown".into()])
        .await
        .unwrap();
    assert_eq!(found.get("c1"), Some(&2_000));
    assert_eq!(found.get("c2"), Some(&1_000));
    assert!(!found.contains_key("unknown"), "{found:?}");
    assert!(history.first_sightings(vec![]).await.unwrap().is_empty());
}

#[tokio::test]
async fn the_last_receive_of_a_rule_is_the_newest_time_it_got_a_torrent_added() {
    let (_dir, _db, history) = store().await;
    history
        .record(
            1_000,
            vec![received("a", "r1", "h1"), received("x", "r2", "h3")],
        )
        .await
        .unwrap();
    history
        .record(
            3_000,
            vec![
                received("b", "r1", "h2"),
                // Seen, and failed to add: not a receive.
                Observation {
                    rule_id: Some("r1".into()),
                    ..obs("c", HistoryResult::AddFailed)
                },
            ],
        )
        .await
        .unwrap();
    // A later duplicate is not a receive either.
    history
        .record(
            5_000,
            vec![Observation {
                rule_id: Some("r1".into()),
                ..obs("d", HistoryResult::Duplicate)
            }],
        )
        .await
        .unwrap();

    let found = history
        .last_received_of_rules(vec!["r1".into(), "r2".into(), "never".into()])
        .await
        .unwrap();
    assert_eq!(found.get("r1"), Some(&3_000));
    assert_eq!(found.get("r2"), Some(&1_000));
    assert!(!found.contains_key("never"), "{found:?}");
    assert!(history
        .last_received_of_rules(vec![])
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn the_titles_of_a_window_are_the_channels_recent_ones_newest_first_and_bounded() {
    let (_dir, _db, history) = store().await;
    for (at, key) in [(1_000, "old"), (2_000, "a"), (3_000, "b"), (4_000, "c")] {
        history
            .record(at, vec![obs(key, HistoryResult::NoMatch)])
            .await
            .unwrap();
    }
    history
        .record(
            3_500,
            vec![Observation {
                channel_id: "c2".into(),
                ..obs("other", HistoryResult::NoMatch)
            }],
        )
        .await
        .unwrap();

    // After the boundary (an item at the boundary is not in), this channel's only.
    let (titles, truncated) = history.titles_since("c1".into(), 1_999, 10).await.unwrap();
    assert_eq!(titles, ["title of c", "title of b", "title of a"]);
    assert!(!truncated);
    let (titles, _) = history.titles_since("c1".into(), 2_000, 10).await.unwrap();
    assert_eq!(titles, ["title of c", "title of b"]);

    // Cut short, the newest stay and it says so.
    let (titles, truncated) = history.titles_since("c1".into(), 1_999, 2).await.unwrap();
    assert_eq!(titles, ["title of c", "title of b"]);
    assert!(truncated);

    // Exactly the limit is not cut short.
    let (titles, truncated) = history.titles_since("c1".into(), 2_000, 2).await.unwrap();
    assert_eq!(titles.len(), 2);
    assert!(!truncated);

    let (titles, _) = history.titles_since("c1".into(), 9_000, 10).await.unwrap();
    assert!(titles.is_empty());
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
async fn torrent_hashes_are_looked_up_by_channel() {
    let (_dir, _db, history) = store().await;
    let other = |key: &str, hash: Option<&str>| Observation {
        channel_id: "c2".into(),
        torrent_hash: hash.map(str::to_owned),
        ..obs(key, HistoryResult::Received)
    };
    history
        .record(
            1,
            vec![
                received("a", "r1", "hash-a"),
                received("b", "r1", "hash-b"),
                obs("c", HistoryResult::NoMatch), // no torrent
                other("d", Some("hash-d")),
                other("e", Some("hash-a")), // the same torrent seen through another channel
            ],
        )
        .await
        .unwrap();

    let hashes = |ids: &[&str]| {
        let history = history.clone();
        let ids: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
        async move {
            let mut hashes: Vec<_> = history
                .torrent_hashes_of_channels(ids)
                .await
                .unwrap()
                .into_iter()
                .collect();
            hashes.sort();
            hashes
        }
    };
    assert_eq!(hashes(&["c1"]).await, ["hash-a", "hash-b"]);
    assert_eq!(hashes(&["c2"]).await, ["hash-a", "hash-d"]);
    assert_eq!(hashes(&["c1", "c2"]).await, ["hash-a", "hash-b", "hash-d"]);
    assert!(hashes(&["nobody"]).await.is_empty());
    assert!(hashes(&[]).await.is_empty());
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
        .create_channel(ChannelInput::new("http://feed.test/rss?token=abc"))
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

// --- reads for the screen and the cycle ------------------------------------------------

#[tokio::test]
async fn an_item_is_read_by_its_id() {
    let (_dir, _db, history) = store().await;
    history
        .record(1_000, vec![obs("a", HistoryResult::NoMatch)])
        .await
        .unwrap();
    let stored = all(&history).await.remove(0);

    assert_eq!(history.get(stored.id).await.unwrap(), Some(stored));
    assert_eq!(history.get(9_999).await.unwrap(), None);
}

#[tokio::test]
async fn the_list_can_be_limited_to_several_results_at_once() {
    let (_dir, _db, history) = store().await;
    for (n, result) in [
        HistoryResult::Received,
        HistoryResult::NoMatch,
        HistoryResult::Duplicate,
        HistoryResult::AddFailed,
    ]
    .into_iter()
    .enumerate()
    {
        history
            .record(1_000 + n as i64, vec![obs(&format!("k{n}"), result)])
            .await
            .unwrap();
    }

    let list = |results: Vec<HistoryResult>| {
        let history = history.clone();
        async move {
            history
                .list(HistoryQuery {
                    results,
                    ..Default::default()
                })
                .await
                .unwrap()
                .items
                .into_iter()
                .map(|i| i.result)
                .collect::<Vec<_>>()
        }
    };

    assert_eq!(
        list(vec![HistoryResult::AddFailed, HistoryResult::Duplicate]).await,
        [HistoryResult::AddFailed, HistoryResult::Duplicate]
    );
    assert_eq!(
        list(vec![HistoryResult::Received]).await,
        [HistoryResult::Received]
    );
    assert_eq!(list(vec![]).await.len(), 4);
    // Together with the single-result filter, both must hold.
    let both = history
        .list(HistoryQuery {
            result: Some(HistoryResult::Received),
            results: vec![HistoryResult::AddFailed],
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(both.items.is_empty());
}

#[tokio::test]
async fn counts_are_per_result_and_can_be_limited_to_a_channel() {
    let (_dir, _db, history) = store().await;
    let mut other = obs("x", HistoryResult::NoMatch);
    other.channel_id = "c2".into();
    history
        .record(
            1_000,
            vec![
                obs("a", HistoryResult::NoMatch),
                obs("b", HistoryResult::NoMatch),
                obs("c", HistoryResult::AddFailed),
                other,
            ],
        )
        .await
        .unwrap();

    let mut everything = history.counts(None).await.unwrap();
    everything.sort_by_key(|(result, _)| result.code());
    assert_eq!(
        everything,
        [(HistoryResult::AddFailed, 1), (HistoryResult::NoMatch, 3)]
    );
    let mut c1 = history.counts(Some("c1".into())).await.unwrap();
    c1.sort_by_key(|(result, _)| result.code());
    assert_eq!(
        c1,
        [(HistoryResult::AddFailed, 1), (HistoryResult::NoMatch, 2)]
    );
    assert!(history
        .counts(Some("nope".into()))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn held_hashes_are_those_of_received_or_duplicate_items_among_the_given_ones() {
    let (_dir, _db, history) = store().await;
    let mut other_channel = received("k1", "r1", "hash-elsewhere");
    other_channel.channel_id = "c2".into();
    history
        .record(
            1_000,
            vec![
                received("k1", "r1", "hash-1"),
                Observation {
                    torrent_hash: Some("hash-2".into()),
                    ..obs("k2", HistoryResult::Duplicate)
                },
                obs("k3", HistoryResult::NoMatch),
                Observation {
                    torrent_hash: Some("hash-4".into()),
                    ..obs("k4", HistoryResult::AddFailed)
                },
                received("k5", "r1", "hash-5"),
                other_channel,
            ],
        )
        .await
        .unwrap();

    let wanted = |keys: &[&str]| -> Vec<(String, String)> {
        keys.iter()
            .map(|k| ("c1".to_owned(), (*k).to_owned()))
            .collect()
    };
    let held = history
        .held_hashes_of_items(wanted(&["k1", "k2", "k3", "k4", "k6"]))
        .await
        .unwrap();

    assert_eq!(
        held,
        ["hash-1", "hash-2"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
    assert!(history
        .held_hashes_of_items(vec![])
        .await
        .unwrap()
        .is_empty());

    // Many items at once (more than one query holds).
    let many: Vec<(String, String)> = (0..1_000)
        .map(|n| ("c1".to_owned(), format!("absent-{n}")))
        .chain(wanted(&["k5"]))
        .collect();
    assert_eq!(
        history.held_hashes_of_items(many).await.unwrap(),
        ["hash-5"].into_iter().map(str::to_owned).collect()
    );
}

// --- results set from outside a cycle --------------------------------------------------

#[tokio::test]
async fn an_outcome_changes_the_result_and_the_trail_but_not_when_the_item_was_seen() {
    let (_dir, _db, history) = store().await;
    history
        .record(1_000, vec![obs("a", HistoryResult::NoMatch)])
        .await
        .unwrap();
    history
        .record(2_000, vec![obs("a", HistoryResult::NoMatch)])
        .await
        .unwrap();
    let before = all(&history).await.remove(0);
    assert_eq!((before.first_seen_at, before.last_seen_at), (1_000, 2_000));

    let after = history
        .record_outcome(
            before.id,
            9_000,
            HistoryResult::Received,
            None,
            None,
            Some("hash-1".into()),
        )
        .await
        .unwrap();

    assert_eq!(after, Some(HistoryResult::Received));
    let item = history.get(before.id).await.unwrap().unwrap();
    assert_eq!(item.result, HistoryResult::Received);
    assert_eq!((item.result_at, item.last_seen_at), (9_000, 2_000));
    assert_eq!(item.torrent_hash.as_deref(), Some("hash-1"));
    assert_eq!(item.rule_id, None);
    assert_eq!((item.title, item.link), (before.title, before.link));
    let trail = history.changes(item.id).await.unwrap();
    assert_eq!(trail.len(), 1);
    assert_eq!(
        (trail[0].from, trail[0].to, trail[0].changed_at),
        (HistoryResult::NoMatch, HistoryResult::Received, 9_000)
    );
}

#[tokio::test]
async fn an_outcome_follows_the_same_transition_rules_as_a_cycle() {
    let (_dir, _db, history) = store().await;
    history
        .record(1_000, vec![received("done", "r1", "hash-1")])
        .await
        .unwrap();
    history
        .record(1_000, vec![obs("open", HistoryResult::NoMatch)])
        .await
        .unwrap();
    let items = all(&history).await;
    let done = items.iter().find(|i| i.identity_key == "done").unwrap().id;
    let open = items.iter().find(|i| i.identity_key == "open").unwrap().id;

    // A received item stays received, whatever is said afterwards, and the
    // answer tells the caller so.
    let kept = history
        .record_outcome(
            done,
            2_000,
            HistoryResult::AddFailed,
            None,
            Some("late failure".into()),
            None,
        )
        .await
        .unwrap();
    assert_eq!(kept, Some(HistoryResult::Received));
    let item = history.get(done).await.unwrap().unwrap();
    assert_eq!(
        (item.result, item.reason, item.result_at),
        (HistoryResult::Received, None, 1_000)
    );
    assert!(history.changes(done).await.unwrap().is_empty());

    // A failure is refreshed by another failure with a new reason.
    history
        .record_outcome(
            open,
            3_000,
            HistoryResult::AddFailed,
            None,
            Some("first".into()),
            None,
        )
        .await
        .unwrap();
    history
        .record_outcome(
            open,
            4_000,
            HistoryResult::AddFailed,
            None,
            Some("second".into()),
            None,
        )
        .await
        .unwrap();
    let item = history.get(open).await.unwrap().unwrap();
    assert_eq!(
        (item.result, item.reason.as_deref()),
        (HistoryResult::AddFailed, Some("second"))
    );
    assert_eq!(history.changes(open).await.unwrap().len(), 1);

    // A failed item can then be received; the reason is dropped.
    history
        .record_outcome(
            open,
            5_000,
            HistoryResult::Received,
            None,
            None,
            Some("hash-2".into()),
        )
        .await
        .unwrap();
    let item = history.get(open).await.unwrap().unwrap();
    assert_eq!((item.result, item.reason), (HistoryResult::Received, None));

    assert_eq!(
        history
            .record_outcome(9_999, 6_000, HistoryResult::Received, None, None, None)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn an_outcome_keeps_the_rule_it_is_given_through_failure_and_success() {
    let (_dir, _db, history) = store().await;
    history
        .record(1_000, vec![obs("a", HistoryResult::AddFailed)])
        .await
        .unwrap();
    let id = all(&history).await.remove(0).id;

    // A retry that fails again keeps the item tied to the rule that picked it,
    // so it can be retried once more.
    for (at, reason) in [(2_000, "first"), (3_000, "second")] {
        history
            .record_outcome(
                id,
                at,
                HistoryResult::AddFailed,
                Some("rule-1".into()),
                Some(reason.into()),
                None,
            )
            .await
            .unwrap();
        let item = history.get(id).await.unwrap().unwrap();
        assert_eq!(item.rule_id.as_deref(), Some("rule-1"));
        assert_eq!(item.reason.as_deref(), Some(reason));
    }

    // The retry that succeeds leaves the rule on the item and on the trail.
    history
        .record_outcome(
            id,
            4_000,
            HistoryResult::Received,
            Some("rule-1".into()),
            None,
            Some("hash-1".into()),
        )
        .await
        .unwrap();
    let item = history.get(id).await.unwrap().unwrap();
    assert_eq!(item.result, HistoryResult::Received);
    assert_eq!(item.rule_id.as_deref(), Some("rule-1"));
    let trail = history.changes(id).await.unwrap();
    assert_eq!(trail.last().unwrap().rule_id.as_deref(), Some("rule-1"));
    assert!(
        !history.received_by_hand("hash-1").await.unwrap(),
        "a retry for a rule is not a receive by hand"
    );
}

#[tokio::test]
async fn a_note_goes_only_on_a_received_item_that_has_none() {
    let (_dir, _db, history) = store().await;
    history
        .record(
            1_000,
            vec![
                received("a", "rule-1", "hash-a"),
                obs("b", HistoryResult::NoMatch),
            ],
        )
        .await
        .unwrap();
    let items = all(&history).await;
    let of = |key: &str| items.iter().find(|i| i.title.contains(key)).unwrap().id;
    let (a, b) = (of("of a"), of("of b"));

    assert!(history.note_received(a, "first note").await.unwrap());
    assert!(
        !history.note_received(a, "second note").await.unwrap(),
        "an existing note stays"
    );
    assert!(
        !history.note_received(b, "not received").await.unwrap(),
        "only received items take a note"
    );

    let item = history.get(a).await.unwrap().unwrap();
    assert_eq!(
        (item.result, item.reason.as_deref()),
        (HistoryResult::Received, Some("first note"))
    );
    assert!(history.changes(a).await.unwrap().is_empty());
    assert_eq!(history.get(b).await.unwrap().unwrap().reason, None);
}

#[tokio::test]
async fn a_torrent_is_received_by_hand_when_a_received_item_without_a_rule_holds_it() {
    let (_dir, _db, history) = store().await;
    history
        .record(
            1_000,
            vec![
                received("a", "rule-1", "hash-rule"),
                received("b", "rule-1", "hash-both"),
                Observation {
                    torrent_hash: Some("hash-dup".into()),
                    ..obs("c", HistoryResult::Duplicate)
                },
                obs("d", HistoryResult::NoMatch),
                obs("e", HistoryResult::NoMatch),
                obs("f", HistoryResult::NoMatch),
            ],
        )
        .await
        .unwrap();
    let items = all(&history).await;
    let of = |key: &str| items.iter().find(|i| i.title.contains(key)).unwrap().id;
    // Received by hand: `d` alone, and `e` with the same torrent as rule item `b`.
    for (key, hash) in [("d", "hash-hand"), ("e", "hash-both")] {
        history
            .record_outcome(
                of(key),
                2_000,
                HistoryResult::Received,
                None,
                None,
                Some(hash.into()),
            )
            .await
            .unwrap();
    }
    // A command that met a torrent already there did not receive it.
    history
        .record_outcome(
            of("f"),
            2_000,
            HistoryResult::Duplicate,
            None,
            None,
            Some("hash-cmd-dup".into()),
        )
        .await
        .unwrap();

    assert!(history.received_by_hand("hash-hand").await.unwrap());
    assert!(history.received_by_hand("hash-both").await.unwrap());
    assert!(!history.received_by_hand("hash-rule").await.unwrap());
    assert!(!history.received_by_hand("hash-dup").await.unwrap());
    assert!(!history.received_by_hand("hash-cmd-dup").await.unwrap());
    assert!(!history.received_by_hand("hash-unknown").await.unwrap());
}

// --- the hashes a rule received --------------------------------------------

#[tokio::test]
async fn the_hashes_of_many_rules_come_back_per_rule_in_chunks() {
    let (_dir, _db, history) = store().await;
    // More rules than one query binds, so the chunks are all exercised.
    let rules: Vec<String> = (0..850).map(|n| format!("rule-{n}")).collect();
    let mut observations = Vec::new();
    for (n, rule) in rules.iter().enumerate() {
        if n % 2 == 0 {
            // Two items of the rule name the same torrent, and another a second one.
            observations.push(received(&format!("a{n}"), rule, &format!("hash-{n}-b")));
            observations.push(received(&format!("b{n}"), rule, &format!("hash-{n}-b")));
            observations.push(received(&format!("c{n}"), rule, &format!("hash-{n}-a")));
        }
    }
    // Not received: not what the rule brought in.
    observations.push(Observation {
        rule_id: Some("rule-1".into()),
        torrent_hash: Some("hash-failed".into()),
        ..obs("failed", HistoryResult::AddFailed)
    });
    history.record(1_000, observations).await.unwrap();

    let got = history.received_hashes_of_rules(rules).await.unwrap();

    assert_eq!(got.len(), 425);
    assert_eq!(got["rule-0"], ["hash-0-a", "hash-0-b"]);
    assert_eq!(got["rule-848"], ["hash-848-a", "hash-848-b"]);
    assert!(!got.contains_key("rule-1"));
}

/// The plan of the query that reads the hashes must search by rule through an
/// index: a scan would read every received item of the history for every
/// cycle, and the history is kept indefinitely.
#[tokio::test]
async fn the_hashes_of_rules_are_read_through_an_index_on_the_rule() {
    let (_dir, db, _history) = store().await;
    let plan: Vec<String> = db
        .run::<_, DbError, _>(|c| {
            let sql = format!("EXPLAIN QUERY PLAN {}", repo::received_hashes_sql(3));
            let mut stmt = c.prepare(&sql)?;
            let rows = stmt
                .query_map(["a", "b", "c"], |row| row.get::<_, String>(3))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
        .unwrap();

    assert!(
        plan.iter()
            .any(|step| step.contains("SEARCH history_items USING INDEX history_items_by_rule")),
        "{plan:?}"
    );
    assert!(
        plan.iter().all(|step| !step.starts_with("SCAN")),
        "{plan:?}"
    );
}

#[tokio::test]
async fn the_last_receive_and_the_recent_titles_are_read_through_indexes_not_the_whole_history() {
    let (_dir, db, _history) = store().await;
    let plans: Vec<(&str, Vec<String>)> = db
        .run::<_, DbError, _>(|c| {
            let mut plans = Vec::new();
            for (name, sql, params) in [
                (
                    "last receive",
                    repo::LAST_RECEIVED_SQL,
                    rusqlite::params_from_iter(vec!["r1".to_owned()]),
                ),
                (
                    "titles",
                    repo::TITLES_SINCE_SQL,
                    rusqlite::params_from_iter(vec![
                        "c1".to_owned(),
                        "0".to_owned(),
                        "10".to_owned(),
                    ]),
                ),
            ] {
                let mut stmt = c.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
                let rows = stmt
                    .query_map(params, |row| row.get::<_, String>(3))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                plans.push((name, rows));
            }
            Ok(plans)
        })
        .await
        .unwrap();

    for (name, plan) in &plans {
        assert!(
            plan.iter()
                .any(|step| step.starts_with("SEARCH history_items USING INDEX")),
            "{name}: {plan:?}"
        );
        assert!(
            plan.iter().all(|step| !step.starts_with("SCAN")),
            "{name}: {plan:?}"
        );
    }
    assert!(
        plans[0]
            .1
            .iter()
            .any(|s| s.contains("history_items_by_rule")),
        "{plans:?}"
    );
    assert!(
        plans[1]
            .1
            .iter()
            .any(|s| s.contains("history_items_by_channel")),
        "{plans:?}"
    );
}

#[tokio::test]
async fn the_titles_of_what_rules_received_come_back_with_their_torrents() {
    let (_dir, _db, history) = store().await;
    history
        .record(
            1_000,
            vec![
                received("a", "rule-1", "hash-a"),
                received("b", "rule-1", "hash-b"),
                received("c", "rule-2", "hash-c"),
                Observation {
                    rule_id: Some("rule-1".into()),
                    ..obs("no-torrent", HistoryResult::AddFailed)
                },
            ],
        )
        .await
        .unwrap();

    let got = history
        .received_titles_of_rules(
            vec!["rule-1".into(), "rule-3".into()],
            vec!["hash-a".into(), "hash-b".into(), "hash-c".into()],
        )
        .await
        .unwrap();

    assert_eq!(got.len(), 1);
    let mut rule_1 = got["rule-1"].clone();
    rule_1.sort();
    assert_eq!(
        rule_1,
        [
            ("hash-a".to_owned(), "title of a".to_owned()),
            ("hash-b".to_owned(), "title of b".to_owned()),
        ]
    );

    // Only the torrents asked about are read, and none asked about reads none.
    let only_b = history
        .received_titles_of_rules(vec!["rule-1".into()], vec!["hash-b".into(), "other".into()])
        .await
        .unwrap();
    assert_eq!(
        only_b["rule-1"],
        [("hash-b".to_owned(), "title of b".to_owned())]
    );
    let none = history
        .received_titles_of_rules(vec!["rule-1".into()], vec![])
        .await
        .unwrap();
    assert!(none.is_empty());
}
