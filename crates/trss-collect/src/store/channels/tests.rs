use std::path::PathBuf;

use tempfile::TempDir;

use crate::store::channels::*;

const SECRET: &str = "s3cr3t-abc";

struct Fixture {
    _dir: TempDir,
    path: PathBuf,
    store: ChannelStore,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    let db = Db::open(&path).await.unwrap();
    Fixture {
        _dir: dir,
        path,
        store: ChannelStore::new(db),
    }
}

impl Fixture {
    /// A second, independent connection to the same file, standing in for the
    /// other process, used to inject failures.
    fn raw(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(&self.path).unwrap()
    }
}

fn rule(phrase: &str, directory: &str) -> RuleInput {
    RuleInput {
        r#match: Some(phrase.to_owned()),
        directory: directory.to_owned(),
        ..RuleInput::default()
    }
}

fn order(rules: &[Rule]) -> Vec<String> {
    rules.iter().map(|r| r.id.clone()).collect()
}

fn item(r: &Rule) -> OrderItem {
    OrderItem {
        id: r.id.clone(),
        version: r.version,
    }
}

/// One channel with four rules "r0".."r3".
async fn channel_with_four_rules(f: &Fixture) -> ChannelWithRules {
    let rules = (0..4)
        .map(|i| rule(&format!("r{i}"), &format!("d{i}")))
        .collect();
    f.store
        .create_channel_with_rules(ChannelInput::new("https://example.com/rss?r=1080"), rules)
        .await
        .unwrap()
}

#[tokio::test]
async fn two_channels_five_rules_survive_reopen_with_every_field() {
    let f = fixture().await;

    let mut input_a = ChannelInput::new("https://a.example/rss?r=1080&token=abc");
    input_a.excludes = vec!["[Batch]".into(), "HEVC".into()];
    input_a.secret_query = vec!["token".into()];
    input_a.past_search = Some("[SubsPlease] {match} 1080p".into());
    let input_b = ChannelInput::new("https://b.example/feed");

    let a = f.store.create_channel(input_a.clone()).await.unwrap();
    let b = f.store.create_channel(input_b.clone()).await.unwrap();
    assert_eq!((a.position, b.position), (0, 1));
    assert_eq!((a.version, b.version), (1, 1));
    assert!(!a.id.is_empty() && !b.id.is_empty() && a.id != b.id);

    let inputs = [
        (
            &a,
            RuleInput {
                r#match: Some("Frieren".into()),
                regex: true,
                case_insensitive: true,
                directory: "Frieren/S01".into(),
                episode: -12,
                episode_auto: true,
                state: RuleState::Archived,
            },
        ),
        (
            &a,
            RuleInput {
                r#match: None,
                directory: "Waiting".into(),
                ..RuleInput::default()
            },
        ),
        (&a, rule("Dandadan", "Dandadan")),
        (&b, rule("한글 제목", "한글/시즌1")),
        (
            &b,
            RuleInput {
                episode: 0,
                ..rule("Zero", "")
            },
        ),
    ];
    let mut created = Vec::new();
    for (channel, input) in &inputs {
        created.push(
            f.store
                .create_rule(&channel.id, input.clone())
                .await
                .unwrap(),
        );
    }
    assert_eq!(
        created.iter().map(|r| r.position).collect::<Vec<_>>(),
        [0, 1, 2, 0, 1]
    );

    // Reopen the file with a fresh handle.
    let reopened = ChannelStore::new(Db::open(&f.path).await.unwrap());

    let channels = reopened.list_channels().await.unwrap();
    assert_eq!(channels, vec![a.clone(), b.clone()]);
    assert_eq!(channels[0].to_input(), input_a);
    assert_eq!(channels[1].to_input(), input_b);

    let rules_a = reopened.list_rules(&a.id).await.unwrap();
    let rules_b = reopened.list_rules(&b.id).await.unwrap();
    assert_eq!([rules_a.as_slice(), rules_b.as_slice()].concat(), created);
    for (stored, (channel, input)) in created.iter().zip(&inputs) {
        assert_eq!(&stored.to_input(), input);
        assert_eq!(stored.channel_id, channel.id);
        assert_eq!(stored.version, 1);
    }
    assert_eq!(rules_a[1].r#match, None);
    assert_eq!(rules_a[0].episode, -12);
    assert_eq!(rules_a[0].state, RuleState::Archived);
    assert_eq!(
        reopened.get_rule(&created[3].id).await.unwrap(),
        Some(created[3].clone())
    );

    let snapshot = reopened.list_channels_with_rules().await.unwrap();
    assert_eq!(snapshot.len(), 2);
    assert_eq!(snapshot[0].rules, rules_a);
    assert_eq!(snapshot[1].rules, rules_b);
}

#[tokio::test]
async fn channel_name_is_trimmed_and_blank_means_unnamed() {
    let f = fixture().await;

    let mut input = ChannelInput::new("https://a.example/rss");
    input.name = Some("  Weekly anime \t".into());
    let created = f.store.create_channel(input).await.unwrap();
    assert_eq!(created.name.as_deref(), Some("Weekly anime"));
    assert_eq!(
        created.to_input().name.as_deref(),
        Some("Weekly anime"),
        "to_input round-trips the stored name"
    );

    // Unnamed by default.
    let plain = f
        .store
        .create_channel(ChannelInput::new("https://b.example/rss"))
        .await
        .unwrap();
    assert_eq!(plain.name, None);

    // Blank on update clears the name; a value sets it. Both survive a reopen.
    let mut edit = created.to_input();
    edit.name = Some("   ".into());
    let cleared = f
        .store
        .update_channel(&created.id, created.version, edit)
        .await
        .unwrap();
    assert_eq!(cleared.name, None);
    let mut edit = plain.to_input();
    edit.name = Some(" Second ".into());
    let named = f
        .store
        .update_channel(&plain.id, plain.version, edit)
        .await
        .unwrap();
    assert_eq!(named.name.as_deref(), Some("Second"));

    let reopened = ChannelStore::new(Db::open(&f.path).await.unwrap());
    let stored = reopened.list_channels().await.unwrap();
    assert_eq!(stored, vec![cleared, named]);
}

#[tokio::test]
async fn legacy_episode_default_is_one() {
    assert_eq!(RuleInput::default().episode, 1);
}

#[tokio::test]
async fn second_update_with_the_same_version_conflicts_and_changes_nothing() {
    let f = fixture().await;
    let channel = f
        .store
        .create_channel(ChannelInput::new("https://x.example/rss"))
        .await
        .unwrap();
    let mut r = f
        .store
        .create_rule(&channel.id, rule("A", "a"))
        .await
        .unwrap();
    // Bring the rule to version 3.
    for phrase in ["B", "C"] {
        r = f
            .store
            .update_rule(&r.id, r.version, &channel.id, rule(phrase, "a"))
            .await
            .unwrap();
    }
    assert_eq!(r.version, 3);

    let first = f
        .store
        .update_rule(&r.id, 3, &channel.id, rule("first", "a"))
        .await
        .unwrap();
    assert_eq!(first.version, 4);

    let err = f
        .store
        .update_rule(&r.id, 3, &channel.id, rule("second", "a"))
        .await
        .unwrap_err();
    assert!(err.is_conflict(), "{err:?}");
    assert!(matches!(
        err,
        ChannelError::Conflict {
            expected: 3,
            actual: 4,
            ..
        }
    ));
    assert_eq!(f.store.get_rule(&r.id).await.unwrap(), Some(first));
}

#[tokio::test]
async fn stale_channel_update_conflicts_across_handles() {
    // The web and the worker each hold their own handle on the file.
    let f = fixture().await;
    let other = ChannelStore::new(Db::open(&f.path).await.unwrap());
    let channel = f
        .store
        .create_channel(ChannelInput::new("https://x.example/rss"))
        .await
        .unwrap();

    let mut edited = channel.to_input();
    edited.name = Some("Elsewhere".into());
    other.update_channel(&channel.id, 1, edited).await.unwrap();

    let mut stale = channel.to_input();
    stale.name = Some("Stale".into());
    let err = f
        .store
        .update_channel(&channel.id, 1, stale)
        .await
        .unwrap_err();
    assert!(err.is_conflict());
    let now = f.store.get_channel(&channel.id).await.unwrap().unwrap();
    assert_eq!((now.name.as_deref(), now.version), (Some("Elsewhere"), 2));
}

#[tokio::test]
async fn reorder_rules_applies_the_new_order_and_versions_moved_rules() {
    let f = fixture().await;
    let created = channel_with_four_rules(&f).await;
    let r = &created.rules;

    let new_order = [&r[1], &r[0], &r[2], &r[3]];
    let out = f
        .store
        .reorder_rules(
            &created.channel.id,
            new_order.iter().map(|r| item(r)).collect(),
        )
        .await
        .unwrap();
    assert_eq!(
        order(&out),
        order(&[r[1].clone(), r[0].clone(), r[2].clone(), r[3].clone()])
    );
    assert_eq!(
        out.iter().map(|r| r.position).collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    assert_eq!(
        out.iter().map(|r| r.version).collect::<Vec<_>>(),
        [2, 2, 1, 1]
    );
    assert_eq!(f.store.list_rules(&created.channel.id).await.unwrap(), out);

    // The same request again carries stale versions.
    let err = f
        .store
        .reorder_rules(
            &created.channel.id,
            new_order.iter().map(|r| item(r)).collect(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, ChannelError::Conflict { .. }));
}

#[tokio::test]
async fn reorder_rules_rejects_partial_duplicate_and_foreign_lists() {
    let f = fixture().await;
    let created = channel_with_four_rules(&f).await;
    let other = f
        .store
        .create_channel_with_rules(
            ChannelInput::new("https://o.example/"),
            vec![rule("o", "o")],
        )
        .await
        .unwrap();
    let r = &created.rules;
    let id = &created.channel.id;

    let missing: Vec<_> = r[..3].iter().map(item).collect();
    let mut duplicate: Vec<_> = r.iter().map(item).collect();
    duplicate[3] = item(&r[0]);
    let mut foreign: Vec<_> = r.iter().map(item).collect();
    foreign[3] = item(&other.rules[0]);
    for bad in [missing, duplicate, foreign] {
        let err = f.store.reorder_rules(id, bad).await.unwrap_err();
        assert!(matches!(err, ChannelError::OrderMismatch { .. }), "{err:?}");
        assert!(err.is_conflict());
    }
    assert_eq!(f.store.list_rules(id).await.unwrap(), created.rules);
    assert!(matches!(
        f.store.reorder_rules("nope", vec![]).await.unwrap_err(),
        ChannelError::NotFound { .. }
    ));
}

#[tokio::test]
async fn failure_during_reorder_keeps_the_previous_order() {
    let f = fixture().await;
    let created = channel_with_four_rules(&f).await;
    let r = &created.rules;

    // Rules are rewritten in order: r1 -> 0, r0 -> 1, then r3 -> 2 fails, so the
    // first two writes have already happened when the failure hits.
    f.raw()
        .execute_batch(
            "CREATE TRIGGER inject_failure BEFORE UPDATE OF position ON rules
             WHEN NEW.position = 2
             BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
        )
        .unwrap();
    let new_order = [&r[1], &r[0], &r[3], &r[2]];
    let err = f
        .store
        .reorder_rules(
            &created.channel.id,
            new_order.iter().map(|r| item(r)).collect(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, ChannelError::Db(_)), "{err:?}");

    let after = f.store.list_rules(&created.channel.id).await.unwrap();
    assert_eq!(
        after, created.rules,
        "order, positions and versions are untouched"
    );

    // The store still works afterwards and the same change goes through once
    // the failure is gone.
    f.raw()
        .execute_batch("DROP TRIGGER inject_failure")
        .unwrap();
    let out = f
        .store
        .reorder_rules(
            &created.channel.id,
            new_order.iter().map(|r| item(r)).collect(),
        )
        .await
        .unwrap();
    assert_eq!(
        order(&out),
        [&r[1], &r[0], &r[3], &r[2]].map(|r| r.id.clone())
    );
}

#[tokio::test]
async fn reorder_channels_is_versioned_and_atomic() {
    let f = fixture().await;
    let mut chans = Vec::new();
    for i in 0..3 {
        chans.push(
            f.store
                .create_channel(ChannelInput::new(format!("https://c{i}.example/")))
                .await
                .unwrap(),
        );
    }
    let it = |c: &Channel| OrderItem {
        id: c.id.clone(),
        version: c.version,
    };

    f.raw()
        .execute_batch(
            "CREATE TRIGGER inject_failure BEFORE UPDATE OF position ON channels
             WHEN NEW.position = 1
             BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
        )
        .unwrap();
    let want = vec![it(&chans[2]), it(&chans[0]), it(&chans[1])];
    assert!(f.store.reorder_channels(want.clone()).await.is_err());
    assert_eq!(f.store.list_channels().await.unwrap(), chans);

    f.raw()
        .execute_batch("DROP TRIGGER inject_failure")
        .unwrap();
    let out = f.store.reorder_channels(want.clone()).await.unwrap();
    assert_eq!(
        out.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
        [&chans[2], &chans[0], &chans[1]].map(|c| c.id.clone())
    );
    assert!(f
        .store
        .reorder_channels(want)
        .await
        .unwrap_err()
        .is_conflict());
}

#[tokio::test]
async fn secret_query_is_stored_verbatim_but_masked_for_display() {
    let f = fixture().await;
    let url = format!("https://example.com/rss?r=1080&token={SECRET}");
    let mut input = ChannelInput::new(url.clone());
    assert_eq!(
        input.secret_query,
        ["r", "token"],
        "new channels start all-secret"
    );
    input.secret_query = vec!["token".into()];

    let channel = f.store.create_channel(input.clone()).await.unwrap();
    let read = f.store.get_channel(&channel.id).await.unwrap().unwrap();
    assert_eq!(read.url, url);
    assert_eq!(
        read.masked_url(),
        "https://example.com/rss?r=1080&token=***"
    );

    for shown in [
        format!("{read:?}"),
        format!("{input:?}"),
        format!("{:?}", read.to_input()),
    ] {
        assert!(!shown.contains(SECRET), "{shown}");
        assert!(shown.contains("token=***"), "{shown}");
    }
    let with_rules = ChannelWithRules {
        channel: read.clone(),
        rules: vec![],
    };
    assert!(!format!("{with_rules:?}").contains(SECRET));

    // The value is raw in the database, so a stored value is what connects.
    let raw: String = f
        .raw()
        .query_row("SELECT url FROM channels", [], |r| r.get(0))
        .unwrap();
    assert_eq!(raw, url);
}

#[tokio::test]
async fn error_messages_do_not_contain_secret_values() {
    let f = fixture().await;
    let url = format!("https://example.com/rss?token={SECRET}");
    let channel = f
        .store
        .create_channel(ChannelInput::new(url.clone()))
        .await
        .unwrap();

    let mut errors = Vec::new();
    // Conflict, not found, invalid (secret name missing from the URL).
    errors.push(
        f.store
            .update_channel(&channel.id, 9, channel.to_input())
            .await
            .unwrap_err(),
    );
    errors.push(
        f.store
            .update_channel("missing", 1, channel.to_input())
            .await
            .unwrap_err(),
    );
    let mut bad = channel.to_input();
    bad.secret_query.push("other".into());
    errors.push(
        f.store
            .update_channel(&channel.id, 1, bad)
            .await
            .unwrap_err(),
    );
    let mut bad_url = channel.to_input();
    bad_url.url = format!("not a url {SECRET}");
    errors.push(
        f.store
            .update_channel(&channel.id, 1, bad_url)
            .await
            .unwrap_err(),
    );
    // A database-level failure.
    f.raw()
        .execute_batch(
            "CREATE TRIGGER t BEFORE UPDATE ON channels BEGIN SELECT RAISE(ABORT, 'boom'); END;",
        )
        .unwrap();
    errors.push(
        f.store
            .update_channel(&channel.id, 1, channel.to_input())
            .await
            .unwrap_err(),
    );
    assert_eq!(errors.len(), 5);
    for e in errors {
        assert!(!format!("{e}").contains(SECRET), "{e}");
        assert!(!format!("{e:?}").contains(SECRET), "{e:?}");
    }
}

#[tokio::test]
async fn changing_a_rules_channel_is_rejected() {
    let f = fixture().await;
    let a = f
        .store
        .create_channel(ChannelInput::new("https://a.example/"))
        .await
        .unwrap();
    let b = f
        .store
        .create_channel(ChannelInput::new("https://b.example/"))
        .await
        .unwrap();
    let r = f.store.create_rule(&a.id, rule("x", "x")).await.unwrap();

    let err = f
        .store
        .update_rule(&r.id, r.version, &b.id, rule("changed", "y"))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ChannelError::RuleChannelChange { .. }),
        "{err:?}"
    );
    assert!(!err.is_conflict());
    assert_eq!(f.store.get_rule(&r.id).await.unwrap(), Some(r.clone()));

    // Same channel is fine.
    let ok = f
        .store
        .update_rule(&r.id, r.version, &a.id, rule("changed", "y"))
        .await
        .unwrap();
    assert_eq!(
        (ok.channel_id.as_str(), ok.r#match.as_deref()),
        (a.id.as_str(), Some("changed"))
    );
}

#[tokio::test]
async fn replace_channel_swaps_the_whole_rule_list_atomically() {
    let f = fixture().await;
    let created = channel_with_four_rules(&f).await;
    let id = &created.channel.id;
    let mut input = created.channel.to_input();
    input.name = Some("Replaced".into());
    let new_rules = vec![
        rule("n0", "x"),
        RuleInput {
            r#match: None,
            ..rule("", "y")
        },
    ];

    // Stale version: nothing changes.
    let err = f
        .store
        .replace_channel(id, 7, input.clone(), new_rules.clone())
        .await
        .unwrap_err();
    assert!(err.is_conflict());

    // Failure while inserting the second new rule: the old channel and rules stay.
    f.raw()
        .execute_batch(
            "CREATE TRIGGER inject_failure BEFORE INSERT ON rules
             WHEN NEW.position = 1
             BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
        )
        .unwrap();
    assert!(f
        .store
        .replace_channel(id, 1, input.clone(), new_rules.clone())
        .await
        .is_err());
    let unchanged = f.store.list_channels_with_rules().await.unwrap();
    assert_eq!(unchanged, vec![created.clone()]);

    f.raw()
        .execute_batch("DROP TRIGGER inject_failure")
        .unwrap();
    let replaced = f
        .store
        .replace_channel(id, 1, input, new_rules)
        .await
        .unwrap();
    assert_eq!(replaced.channel.id, *id);
    assert_eq!(replaced.channel.version, 2);
    assert_eq!(replaced.channel.name.as_deref(), Some("Replaced"));
    assert_eq!(replaced.rules.len(), 2);
    assert_eq!(replaced.rules[0].r#match.as_deref(), Some("n0"));
    assert_eq!(replaced.rules[1].r#match, None);
    assert_eq!(
        replaced
            .rules
            .iter()
            .map(|r| r.position)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert!(replaced
        .rules
        .iter()
        .all(|r| created.rules.iter().all(|old| old.id != r.id)));
    assert_eq!(
        f.store.list_channels_with_rules().await.unwrap(),
        vec![replaced]
    );
}

#[tokio::test]
async fn invalid_input_is_rejected_before_anything_is_written() {
    let f = fixture().await;
    let good = ChannelInput::new("https://x.example/?a=1");

    let mut bad = good.clone();
    bad.url = "not a url".into();
    let mut unknown_secret = good.clone();
    unknown_secret.secret_query = vec!["nope".into()];
    let mut dup_secret = good.clone();
    dup_secret.secret_query = vec!["a".into(), "a".into()];
    for input in [bad, unknown_secret, dup_secret] {
        assert!(matches!(
            f.store.create_channel(input).await.unwrap_err(),
            ChannelError::Invalid(_)
        ));
    }
    assert!(f.store.list_channels().await.unwrap().is_empty());

    let c = f.store.create_channel(good).await.unwrap();
    let empty_match = RuleInput {
        r#match: Some(String::new()),
        ..RuleInput::default()
    };
    let absolute = RuleInput {
        directory: "/abs".into(),
        ..RuleInput::default()
    };
    for input in [empty_match, absolute] {
        assert!(matches!(
            f.store.create_rule(&c.id, input).await.unwrap_err(),
            ChannelError::Invalid(_)
        ));
    }
    assert!(matches!(
        f.store
            .create_rule("missing", RuleInput::default())
            .await
            .unwrap_err(),
        ChannelError::NotFound { .. }
    ));
    assert!(f.store.list_rules(&c.id).await.unwrap().is_empty());
}

#[test]
fn mask_url_masks_only_secret_values() {
    let secret = |names: &[&str]| names.iter().map(|s| s.to_string()).collect::<Vec<_>>();

    assert_eq!(
        mask_url("https://h/rss?r=1080&token=abc", &secret(&["token"])),
        "https://h/rss?r=1080&token=***"
    );
    assert_eq!(
        mask_url("https://h/rss?r=1080&token=abc", &secret(&["r", "token"])),
        "https://h/rss?r=***&token=***"
    );
    // Not-yet-entered values stay empty; other text is kept as is.
    assert_eq!(
        mask_url(
            "https://h/rss?token=&flag&x=a%20b#frag",
            &secret(&["token", "flag", "x"])
        ),
        "https://h/rss?token=&flag&x=***#frag"
    );
    // Percent-encoded names match by decoded name.
    assert_eq!(
        mask_url("https://h/rss?to%6Ben=abc", &secret(&["token"])),
        "https://h/rss?to%6Ben=***"
    );
    // A name that repeats is masked at each place, around a fragment too.
    assert_eq!(
        mask_url(
            "https://h/rss?r=1080&token=abc&r=2#f",
            &secret(&["r", "token"])
        ),
        "https://h/rss?r=***&token=***&r=***#f"
    );
    assert_eq!(
        mask_url("https://h/rss?r=1080&token=abc&r=2#f", &secret(&["token"])),
        "https://h/rss?r=1080&token=***&r=2#f"
    );
    assert_eq!(
        mask_url("https://h/rss", &secret(&["token"])),
        "https://h/rss"
    );
    assert_eq!(
        mask_url("https://h/rss?token=abc", &[]),
        "https://h/rss?token=abc"
    );
    assert_eq!(query_names("https://h/?a=1&b=2&a=3"), ["a", "b"]);
    assert!(query_names("garbage").is_empty());
}
