use super::super::*;

async fn store() -> ChannelStore {
    ChannelStore::new(Db::open(":memory:").await.unwrap())
}

fn rule(phrase: &str) -> RuleInput {
    RuleInput {
        r#match: Some(phrase.to_owned()),
        ..RuleInput::default()
    }
}

async fn channel_with_rules(store: &ChannelStore, n: usize) -> ChannelWithRules {
    let rules = (0..n).map(|i| rule(&format!("r{i}"))).collect();
    store
        .create_channel_with_rules(
            ChannelInput::new("https://example.com/a?t=1", "/media"),
            rules,
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn deleting_removes_only_that_rule_and_keeps_the_order() {
    let store = store().await;
    let created = channel_with_rules(&store, 3).await;
    let doomed = &created.rules[1];

    store.delete_rule(&doomed.id, doomed.version).await.unwrap();

    let left = store.list_rules(&created.channel.id).await.unwrap();
    assert_eq!(
        left.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
        vec![created.rules[0].id.clone(), created.rules[2].id.clone()]
    );
    // Untouched rules keep their version, and the channel is not changed.
    assert_eq!(left[0], created.rules[0]);
    assert_eq!(left[1], created.rules[2]);
    let channel = store
        .get_channel(&created.channel.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(channel, created.channel);
}

#[tokio::test]
async fn a_rule_added_after_a_delete_goes_last() {
    let store = store().await;
    let created = channel_with_rules(&store, 3).await;
    let last = &created.rules[2];
    store.delete_rule(&last.id, last.version).await.unwrap();

    let added = store
        .create_rule(&created.channel.id, rule("new"))
        .await
        .unwrap();
    let ids: Vec<String> = store
        .list_rules(&created.channel.id)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids.last(), Some(&added.id));
    assert_eq!(ids.len(), 3);
}

#[tokio::test]
async fn an_old_version_is_a_conflict_and_deletes_nothing() {
    let store = store().await;
    let created = channel_with_rules(&store, 2).await;
    let target = &created.rules[0];
    // Someone else saved first.
    let saved = store
        .update_rule(
            &target.id,
            target.version,
            &created.channel.id,
            RuleInput {
                directory: "elsewhere".into(),
                ..target.to_input()
            },
        )
        .await
        .unwrap();

    let err = store
        .delete_rule(&target.id, target.version)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ChannelError::Conflict { kind: "rule", expected, actual, .. }
            if expected == target.version && actual == saved.version
    ));
    assert!(err.is_conflict());
    assert_eq!(store.get_rule(&target.id).await.unwrap(), Some(saved));
}

#[tokio::test]
async fn a_missing_rule_is_not_found() {
    let store = store().await;
    let err = store.delete_rule("nope", 1).await.unwrap_err();
    assert!(matches!(err, ChannelError::NotFound { kind: "rule", .. }));

    // Deleting twice: the second one finds nothing.
    let created = channel_with_rules(&store, 1).await;
    let r = &created.rules[0];
    store.delete_rule(&r.id, r.version).await.unwrap();
    assert!(matches!(
        store.delete_rule(&r.id, r.version).await.unwrap_err(),
        ChannelError::NotFound { kind: "rule", .. }
    ));
}
