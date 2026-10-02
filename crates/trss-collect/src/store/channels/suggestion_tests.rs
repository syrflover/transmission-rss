//! What archive suggestions keep: when a rule started, and the grounds the user
//! chose to keep collecting on.

use crate::store::channels::*;

async fn store_with_rule() -> (ChannelStore, String, Rule) {
    let store = ChannelStore::new(Db::open_blocking(":memory:").unwrap());
    let channel = store
        .create_channel(ChannelInput::new("https://feed.test/rss"))
        .await
        .unwrap();
    let rule = store
        .create_rule(
            &channel.id,
            RuleInput {
                r#match: Some("Work".into()),
                directory: "Work/Season 01".into(),
                ..RuleInput::default()
            },
        )
        .await
        .unwrap();
    (store, channel.id, rule)
}

#[tokio::test]
async fn a_rule_is_stamped_when_it_is_made_and_the_stamp_goes_with_it() {
    let (store, _, rule) = store_with_rule().await;
    let starts = store.rule_starts().await.unwrap();
    let started = starts[&rule.id];
    // The database's clock: some time after 2026-01-01 and not in the future.
    assert!(started > 1_767_225_600_000, "{started}");
    assert!(
        started
            <= std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64
                + 1_000
    );

    // Editing the rule leaves the stamp alone.
    store
        .update_rule(
            &rule.id,
            rule.version,
            &rule.channel_id,
            RuleInput {
                directory: "Work/Season 02".into(),
                ..rule.to_input()
            },
        )
        .await
        .unwrap();
    assert_eq!(store.rule_starts().await.unwrap()[&rule.id], started);

    let rule = store.get_rule(&rule.id).await.unwrap().unwrap();
    store.delete_rule(&rule.id, rule.version).await.unwrap();
    assert!(store.rule_starts().await.unwrap().is_empty());
}

#[tokio::test]
async fn rules_made_with_a_channel_are_stamped_too() {
    let store = ChannelStore::new(Db::open_blocking(":memory:").unwrap());
    let made = store
        .create_channel_with_rules(
            ChannelInput::new("https://feed.test/rss"),
            vec![
                RuleInput {
                    r#match: Some("A".into()),
                    ..RuleInput::default()
                },
                RuleInput {
                    r#match: Some("B".into()),
                    ..RuleInput::default()
                },
            ],
        )
        .await
        .unwrap();
    let starts = store.rule_starts().await.unwrap();
    assert!(made.rules.iter().all(|rule| starts.contains_key(&rule.id)));
}

#[tokio::test]
async fn a_kept_ground_is_remembered_once_and_goes_with_its_rule() {
    let (store, _, rule) = store_with_rule().await;
    assert!(store.kept_archive_grounds().await.unwrap().is_empty());

    store
        .keep_archive_grounds(
            &rule.id,
            vec!["quiet:100".into(), "unlisted:7".into()],
            1_000,
        )
        .await
        .unwrap();
    // Kept again, the first time stands and nothing doubles.
    store
        .keep_archive_grounds(&rule.id, vec!["quiet:100".into()], 2_000)
        .await
        .unwrap();
    let kept = store.kept_archive_grounds().await.unwrap();
    assert_eq!(kept.len(), 2);
    assert!(kept.contains(&(rule.id.clone(), "quiet:100".to_owned())));
    assert!(kept.contains(&(rule.id.clone(), "unlisted:7".to_owned())));

    let missing = store
        .keep_archive_grounds("no-such-rule", vec!["quiet:1".into()], 1)
        .await;
    assert!(
        matches!(missing, Err(ChannelError::NotFound { .. })),
        "{missing:?}"
    );
    let blank = store
        .keep_archive_grounds(&rule.id, vec![String::new()], 1)
        .await;
    assert!(matches!(blank, Err(ChannelError::Invalid(_))));

    let rule = store.get_rule(&rule.id).await.unwrap().unwrap();
    store.delete_rule(&rule.id, rule.version).await.unwrap();
    assert!(store.kept_archive_grounds().await.unwrap().is_empty());
}
