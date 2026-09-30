use std::path::PathBuf;

use tempfile::TempDir;

use super::import::{match_rules, ImportAction, ImportChannel, ImportedChannel};
use super::{ChannelError, ChannelInput, ChannelStore, ChannelWithRules, Db, RuleInput, RuleState};

struct Fixture {
    _dir: TempDir,
    path: PathBuf,
    store: ChannelStore,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    let store = ChannelStore::new(Db::open(&path).await.unwrap());
    Fixture {
        _dir: dir,
        path,
        store,
    }
}

impl Fixture {
    /// A second connection to the same file, used to inject failures.
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

fn import_channel(url: &str, rules: Vec<RuleInput>) -> ImportChannel {
    ImportChannel {
        input: ChannelInput::new(url),
        rules,
    }
}

async fn existing_channel(f: &Fixture, rules: Vec<RuleInput>) -> ChannelWithRules {
    f.store
        .create_channel_with_rules(ChannelInput::new("https://a.example/rss?token=t"), rules)
        .await
        .unwrap()
}

#[test]
fn rules_match_on_phrase_regex_and_case_flags_only() {
    let stored = |phrase: &str, regex: bool, ci: bool, directory: &str| super::Rule {
        id: format!("{phrase}-{regex}-{ci}"),
        channel_id: "c".into(),
        position: 0,
        version: 3,
        r#match: Some(phrase.into()),
        regex,
        case_insensitive: ci,
        directory: directory.into(),
        episode: 9,
        episode_auto: true,
        state: RuleState::Archived,
    };
    let existing = vec![
        stored("Frieren", false, false, "old dir"),
        stored("Frieren", true, false, "regex"),
        stored("Frieren", false, true, "insensitive"),
    ];

    let incoming = vec![
        // Same phrase and flags; directory, episode and state may differ.
        RuleInput {
            directory: "new dir".into(),
            episode: -24,
            ..rule("Frieren", "")
        },
        // Same phrase but another regex flag is another rule.
        RuleInput {
            regex: true,
            case_insensitive: true,
            ..rule("Frieren", "")
        },
        RuleInput {
            case_insensitive: true,
            ..rule("Frieren", "")
        },
        // The phrase is compared exactly.
        rule("frieren", ""),
    ];
    assert_eq!(
        match_rules(&existing, &incoming),
        vec![Some(0), None, Some(2), None]
    );
}

#[test]
fn repeated_keys_pair_up_in_order_and_the_surplus_is_new_or_removed() {
    let stored = |id: &str, phrase: Option<&str>| super::Rule {
        id: id.into(),
        channel_id: "c".into(),
        position: 0,
        version: 1,
        r#match: phrase.map(str::to_owned),
        regex: false,
        case_insensitive: false,
        directory: String::new(),
        episode: 1,
        episode_auto: false,
        state: RuleState::Active,
    };
    let existing = vec![
        stored("x1", Some("X")),
        stored("w", None),
        stored("x2", Some("X")),
    ];

    // Two existing "X" for three incoming: the first two take x1 and x2 in order.
    let incoming = vec![rule("X", "a"), rule("X", "b"), rule("X", "c")];
    assert_eq!(
        match_rules(&existing, &incoming),
        vec![Some(0), Some(2), None]
    );

    // A rule still waiting for its title has no identity: it neither is matched
    // nor matches, even against another waiting rule.
    let waiting = vec![RuleInput {
        r#match: None,
        ..RuleInput::default()
    }];
    assert_eq!(match_rules(&existing, &waiting), vec![None]);
}

#[tokio::test]
async fn adding_appends_channels_in_order_with_every_rule_value() {
    let f = fixture().await;
    let odd = RuleInput {
        r#match: Some("^Show \\d+$".into()),
        regex: true,
        case_insensitive: true,
        directory: "Show/S01".into(),
        episode: -24,
        episode_auto: false,
        state: RuleState::Active,
    };
    let waiting = RuleInput {
        r#match: None,
        directory: "Later".into(),
        ..RuleInput::default()
    };
    let one = import_channel(
        "https://one.example/rss?token=abc&f=1",
        vec![odd.clone(), waiting.clone()],
    );
    let two = import_channel("https://two.example/rss", vec![rule("Two", "Two")]);

    let results = f
        .store
        .import_channels(vec![ImportAction::Add(one.clone()), ImportAction::Add(two)])
        .await
        .unwrap();
    assert_eq!(results.len(), 2);

    let stored = f.store.list_channels_with_rules().await.unwrap();
    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].channel.position, 0);
    assert_eq!(stored[1].channel.position, 1);
    assert_eq!(stored[0].channel.version, 1);
    assert_eq!(stored[0].channel.to_input(), one.input);
    assert_eq!(stored[0].channel.secret_query, ["token", "f"]);
    assert_eq!(stored[0].rules[0].to_input(), odd);
    assert_eq!(stored[0].rules[1].to_input(), waiting);
    assert_eq!(stored[0].rules[1].r#match, None);
    assert_eq!(results[0].channel(), &stored[0]);
    assert_eq!(results[1].channel(), &stored[1]);
}

#[tokio::test]
async fn adding_stores_no_name_and_replacing_writes_the_planned_name() {
    let f = fixture().await;

    // The legacy file has no name, so an added channel is unnamed.
    let results = f
        .store
        .import_channels(vec![ImportAction::Add(import_channel(
            "https://one.example/rss",
            vec![],
        ))])
        .await
        .unwrap();
    assert_eq!(results[0].channel().channel.name, None);

    // A replacement writes the name the plan carries (the plan keeps the
    // existing one; see `build_actions`).
    let mut named = ChannelInput::new("https://a.example/rss?token=t");
    named.name = Some("Kept".into());
    let a = f
        .store
        .create_channel_with_rules(named.clone(), vec![])
        .await
        .unwrap();
    let mut file = import_channel("https://a.example/rss?token=new", vec![]);
    file.input.name = a.channel.name.clone();
    let results = f
        .store
        .import_channels(vec![ImportAction::Replace {
            id: a.channel.id.clone(),
            expected_version: a.channel.version,
            channel: file,
        }])
        .await
        .unwrap();
    assert_eq!(results[0].channel().channel.name.as_deref(), Some("Kept"));
    let stored = f.store.list_channels().await.unwrap();
    assert_eq!(stored[1].name.as_deref(), Some("Kept"));
}

#[tokio::test]
async fn adding_keeps_existing_channels_and_gives_new_ids() {
    let f = fixture().await;
    let a = existing_channel(&f, vec![rule("A1", "a1"), rule("A2", "a2")]).await;

    let file = import_channel(
        "https://a.example/rss?token=t",
        vec![rule("A1", "a1"), rule("A2", "a2")],
    );
    let results = f
        .store
        .import_channels(vec![ImportAction::Add(file)])
        .await
        .unwrap();

    let stored = f.store.list_channels_with_rules().await.unwrap();
    assert_eq!(stored.len(), 2);
    assert_eq!(
        stored[0], a,
        "the existing channel and its rules are untouched"
    );
    let added = results[0].channel();
    assert_ne!(added.channel.id, a.channel.id);
    assert_eq!(added.channel.position, 1);
    for (new, old) in added.rules.iter().zip(&a.rules) {
        assert_ne!(new.id, old.id);
    }
}

#[tokio::test]
async fn replacing_keeps_ids_of_matching_rules_and_reports_the_removed_ones() {
    let f = fixture().await;
    let other = existing_channel(&f, vec![rule("O", "o")]).await;
    let a = f
        .store
        .create_channel_with_rules(
            ChannelInput::new("https://a.example/rss?token=t"),
            vec![
                rule("Keep1", "old/keep1"),
                rule("Gone1", "gone1"),
                RuleInput {
                    regex: true,
                    ..rule("Keep2", "old/keep2")
                },
                RuleInput {
                    r#match: None,
                    ..rule("", "waiting")
                },
                RuleInput {
                    state: RuleState::Archived,
                    ..rule("Gone2", "gone2")
                },
            ],
        )
        .await
        .unwrap();
    let old_rules = &a.rules;

    // The file has 3 rules; Keep2 comes first now, and a new rule is in between.
    let file = ImportChannel {
        input: ChannelInput {
            excludes: vec!["[Batch]".into()],
            ..ChannelInput::new("https://a.example/rss?token=new")
        },
        rules: vec![
            RuleInput {
                regex: true,
                directory: "new/keep2".into(),
                episode: -12,
                ..rule("Keep2", "")
            },
            rule("Fresh", "fresh"),
            RuleInput {
                episode: -24,
                case_insensitive: false,
                ..rule("Keep1", "new/keep1")
            },
        ],
    };

    let results = f
        .store
        .import_channels(vec![ImportAction::Replace {
            id: a.channel.id.clone(),
            expected_version: a.channel.version,
            channel: file.clone(),
        }])
        .await
        .unwrap();

    let ImportedChannel::Replaced {
        channel,
        kept_rules,
        removed_rules,
    } = &results[0]
    else {
        panic!("expected a replacement");
    };
    assert_eq!(*kept_rules, 2);
    let removed: Vec<_> = removed_rules.iter().map(|r| r.r#match.as_deref()).collect();
    assert_eq!(removed, [Some("Gone1"), None, Some("Gone2")]);

    // The channel is the same row at its old place, with the file's fields.
    assert_eq!(channel.channel.id, a.channel.id);
    assert_eq!(channel.channel.position, a.channel.position);
    assert_eq!(channel.channel.version, a.channel.version + 1);
    assert_eq!(channel.channel.to_input(), file.input);

    // The rules equal the file, in the file's order.
    let inputs: Vec<_> = channel.rules.iter().map(|r| r.to_input()).collect();
    assert_eq!(inputs, file.rules);
    assert_eq!(
        channel.rules.iter().map(|r| r.position).collect::<Vec<_>>(),
        [0, 1, 2]
    );

    // Keep2 and Keep1 kept their IDs (and got a version bump); Fresh is new.
    assert_eq!(channel.rules[0].id, old_rules[2].id);
    assert_eq!(channel.rules[2].id, old_rules[0].id);
    assert_eq!(channel.rules[0].version, old_rules[2].version + 1);
    assert_eq!(channel.rules[2].version, old_rules[0].version + 1);
    let old_ids: Vec<_> = old_rules.iter().map(|r| &r.id).collect();
    assert!(!old_ids.contains(&&channel.rules[1].id));
    assert_eq!(channel.rules[1].version, 1);

    // The result is what is stored, and the other channel did not change.
    let stored = f.store.list_channels_with_rules().await.unwrap();
    assert_eq!(stored[0], other);
    assert_eq!(&stored[1], channel);
    assert_eq!(f.store.get_rule(&old_rules[1].id).await.unwrap(), None);
}

#[tokio::test]
async fn a_failure_in_a_later_action_leaves_the_state_from_before_the_import() {
    let f = fixture().await;
    let a = existing_channel(&f, vec![rule("A1", "a1"), rule("A2", "a2")]).await;
    let before = f.store.list_channels_with_rules().await.unwrap();

    let replace_a = ImportAction::Replace {
        id: a.channel.id.clone(),
        expected_version: a.channel.version,
        channel: import_channel(
            "https://a.example/rss",
            vec![rule("A1", "changed"), rule("New", "new")],
        ),
    };
    let add_b = ImportAction::Add(import_channel(
        "https://b.example/rss",
        vec![rule("B1", "b1"), rule("B2", "boom")],
    ));

    // The database fails while the last rule of the last channel is written,
    // after the earlier writes of the same import have happened.
    f.raw()
        .execute_batch(
            "CREATE TRIGGER inject_failure BEFORE INSERT ON rules
             WHEN NEW.directory = 'boom'
             BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
        )
        .unwrap();
    let err = f
        .store
        .import_channels(vec![replace_a.clone(), add_b.clone()])
        .await
        .unwrap_err();
    assert!(matches!(err, ChannelError::Db(_)), "{err:?}");
    assert_eq!(
        f.store.list_channels_with_rules().await.unwrap(),
        before,
        "neither the replacement nor the added channel remains"
    );

    // The same import applies once the failure is gone.
    f.raw()
        .execute_batch("DROP TRIGGER inject_failure")
        .unwrap();
    f.store
        .import_channels(vec![replace_a, add_b])
        .await
        .unwrap();
    let after = f.store.list_channels_with_rules().await.unwrap();
    assert_eq!(after.len(), 2);
    assert_eq!(after[0].channel.version, a.channel.version + 1);
    assert_eq!(after[1].rules.len(), 2);
}

#[tokio::test]
async fn a_stale_replacement_conflicts_and_nothing_else_of_the_import_is_applied() {
    let f = fixture().await;
    let a = existing_channel(&f, vec![rule("A1", "a1")]).await;
    // Someone saves first.
    f.store
        .update_channel(&a.channel.id, a.channel.version, a.channel.to_input())
        .await
        .unwrap();
    let before = f.store.list_channels_with_rules().await.unwrap();

    let err = f
        .store
        .import_channels(vec![
            ImportAction::Add(import_channel(
                "https://b.example/rss",
                vec![rule("B", "b")],
            )),
            ImportAction::Replace {
                id: a.channel.id.clone(),
                expected_version: a.channel.version,
                channel: import_channel("https://a.example/rss", vec![]),
            },
        ])
        .await
        .unwrap_err();
    assert!(err.is_conflict(), "{err:?}");
    assert_eq!(f.store.list_channels_with_rules().await.unwrap(), before);
}

#[tokio::test]
async fn invalid_input_and_double_replacement_are_rejected_before_anything_is_written() {
    let f = fixture().await;
    let a = existing_channel(&f, vec![rule("A1", "a1")]).await;
    let before = f.store.list_channels_with_rules().await.unwrap();

    let good = import_channel("https://b.example/rss", vec![rule("B", "b")]);

    let absolute = import_channel("https://c.example/rss", vec![rule("C", "/abs")]);
    let err = f
        .store
        .import_channels(vec![
            ImportAction::Add(good.clone()),
            ImportAction::Add(absolute),
        ])
        .await
        .unwrap_err();
    assert!(matches!(err, ChannelError::Invalid(_)), "{err:?}");

    let empty_phrase = import_channel(
        "https://c.example/rss",
        vec![RuleInput {
            r#match: Some(String::new()),
            ..RuleInput::default()
        }],
    );
    let err = f
        .store
        .import_channels(vec![ImportAction::Add(empty_phrase)])
        .await
        .unwrap_err();
    assert!(matches!(err, ChannelError::Invalid(_)), "{err:?}");

    let replace = || ImportAction::Replace {
        id: a.channel.id.clone(),
        expected_version: a.channel.version,
        channel: good.clone(),
    };
    let err = f
        .store
        .import_channels(vec![replace(), replace()])
        .await
        .unwrap_err();
    assert!(matches!(err, ChannelError::Invalid(_)), "{err:?}");

    let err = f
        .store
        .import_channels(vec![ImportAction::Replace {
            id: "missing".into(),
            expected_version: 1,
            channel: good,
        }])
        .await
        .unwrap_err();
    assert!(matches!(err, ChannelError::NotFound { .. }), "{err:?}");

    assert_eq!(f.store.list_channels_with_rules().await.unwrap(), before);
}
