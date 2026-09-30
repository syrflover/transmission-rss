use tempfile::TempDir;

use super::super::*;

struct Fixture {
    _dir: TempDir,
    path: std::path::PathBuf,
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

fn rule(phrase: &str) -> RuleInput {
    RuleInput {
        r#match: Some(phrase.to_owned()),
        ..RuleInput::default()
    }
}

async fn channel_with_rules(f: &Fixture, url: &str, n: usize) -> ChannelWithRules {
    let rules = (0..n).map(|i| rule(&format!("r{i}"))).collect();
    f.store
        .create_channel_with_rules(ChannelInput::new(url), rules)
        .await
        .unwrap()
}

fn count(f: &Fixture, table: &str) -> i64 {
    rusqlite::Connection::open(&f.path)
        .unwrap()
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .unwrap()
}

#[tokio::test]
async fn deleting_removes_the_channel_and_all_its_rules_only() {
    let f = fixture().await;
    let doomed = channel_with_rules(&f, "https://example.com/a?t=1", 3).await;
    let kept = channel_with_rules(&f, "https://example.com/b?t=2", 2).await;

    let removed = f
        .store
        .delete_channel(&doomed.channel.id, doomed.channel.version, 3)
        .await
        .unwrap();
    assert_eq!(removed, 3);

    let left = f.store.list_channels_with_rules().await.unwrap();
    assert_eq!(left, vec![kept]);
    assert_eq!(count(&f, "rules"), 2);
    assert!(f
        .store
        .get_channel(&doomed.channel.id)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn archived_rules_go_with_the_channel() {
    let f = fixture().await;
    let created = channel_with_rules(&f, "https://example.com/a", 2).await;
    let archived = RuleInput {
        state: RuleState::Archived,
        ..created.rules[0].to_input()
    };
    let rule0 = &created.rules[0];
    f.store
        .update_rule(&rule0.id, rule0.version, &created.channel.id, archived)
        .await
        .unwrap();

    let removed = f
        .store
        .delete_channel(&created.channel.id, created.channel.version, 2)
        .await
        .unwrap();
    assert_eq!(removed, 2);
    assert_eq!(count(&f, "rules"), 0);
}

#[tokio::test]
async fn a_channel_without_rules_can_be_deleted() {
    let f = fixture().await;
    let created = channel_with_rules(&f, "https://example.com/a", 0).await;
    let removed = f
        .store
        .delete_channel(&created.channel.id, created.channel.version, 0)
        .await
        .unwrap();
    assert_eq!(removed, 0);
    assert_eq!(count(&f, "channels"), 0);
}

#[tokio::test]
async fn stale_version_conflicts_and_deletes_nothing() {
    let f = fixture().await;
    let created = channel_with_rules(&f, "https://example.com/a", 2).await;
    let seen = created.channel.version;
    // Someone saves the channel first.
    f.store
        .update_channel(&created.channel.id, seen, created.channel.to_input())
        .await
        .unwrap();

    let err = f
        .store
        .delete_channel(&created.channel.id, seen, 2)
        .await
        .unwrap_err();
    assert!(matches!(err, ChannelError::Conflict { .. }), "{err:?}");
    assert!(err.is_conflict());
    assert_eq!(count(&f, "channels"), 1);
    assert_eq!(count(&f, "rules"), 2);
}

#[tokio::test]
async fn a_rule_added_after_the_confirmation_refuses_the_delete() {
    let f = fixture().await;
    let created = channel_with_rules(&f, "https://example.com/a", 3).await;
    // The user was told "3 rules will go"; then a fourth appears.
    f.store
        .create_rule(&created.channel.id, rule("late"))
        .await
        .unwrap();

    let err = f
        .store
        .delete_channel(&created.channel.id, created.channel.version, 3)
        .await
        .unwrap_err();
    assert!(err.is_conflict(), "{err:?}");
    assert_eq!(count(&f, "channels"), 1);
    assert_eq!(count(&f, "rules"), 4);
}

#[tokio::test]
async fn deleting_a_missing_channel_is_not_found() {
    let f = fixture().await;
    let err = f.store.delete_channel("nope", 1, 0).await.unwrap_err();
    assert!(matches!(err, ChannelError::NotFound { .. }), "{err:?}");
}

#[tokio::test]
async fn a_failure_while_deleting_keeps_both_channel_and_rules() {
    let f = fixture().await;
    let created = channel_with_rules(&f, "https://example.com/a", 3).await;
    // Fail the channel-row delete after the rules are already gone.
    rusqlite::Connection::open(&f.path)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER no_channel_delete BEFORE DELETE ON channels
             BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
        )
        .unwrap();

    let err = f
        .store
        .delete_channel(&created.channel.id, created.channel.version, 3)
        .await
        .unwrap_err();
    assert!(matches!(err, ChannelError::Db(_)), "{err:?}");
    assert_eq!(
        f.store.list_channels_with_rules().await.unwrap(),
        vec![created]
    );
}

#[tokio::test]
async fn history_rows_that_mention_the_channel_survive_the_delete() {
    let f = fixture().await;
    let created = channel_with_rules(&f, "https://example.com/a", 1).await;
    // Ticket 0004 adds the real history table; it deliberately has no foreign
    // key to `channels`. A stand-in of the same shape shows the delete leaves
    // such rows alone.
    let raw = rusqlite::Connection::open(&f.path).unwrap();
    raw.execute_batch(
        "CREATE TABLE history_standin (channel_id TEXT NOT NULL, title TEXT NOT NULL)",
    )
    .unwrap();
    raw.execute(
        "INSERT INTO history_standin VALUES (?1, 'Show - 01')",
        [&created.channel.id],
    )
    .unwrap();

    f.store
        .delete_channel(&created.channel.id, created.channel.version, 1)
        .await
        .unwrap();

    assert_eq!(count(&f, "history_standin"), 1);
}
