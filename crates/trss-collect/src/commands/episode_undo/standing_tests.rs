//! What a request to undo the automatic value of a rule is for ([`standing`]):
//! the web checks it before it accepts the request and the worker decides it
//! again, so the table here is the one place the rule is tested.

use super::*;
use crate::store::{
    channels::{ChannelInput, RuleInput},
    history::{HistoryResult, HistoryStore, Observation},
};
use tempfile::TempDir;
use trss_core::Db;

struct Env {
    _dir: TempDir,
    path: PathBuf,
    channels: ChannelStore,
    history: HistoryStore,
    channel: String,
}

impl Env {
    async fn new() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.db");
        let db = Db::open(&path).await.unwrap();
        let channels = ChannelStore::new(db.clone());
        let channel = channels
            .create_channel(ChannelInput::new("https://feed.test/rss"))
            .await
            .unwrap()
            .id;
        Env {
            _dir: dir,
            path,
            channels,
            history: HistoryStore::new(db),
            channel,
        }
    }

    /// A rule holding the offset the user typed, `1`.
    async fn typed(&self) -> Rule {
        self.channels
            .create_rule(
                &self.channel,
                RuleInput {
                    r#match: Some("Show".into()),
                    directory: "Show/Season 03".into(),
                    ..RuleInput::default()
                },
            )
            .await
            .unwrap()
    }

    /// A rule the app gave `−48` in place of `1`.
    async fn automatic(&self) -> Rule {
        let rule = self.typed().await;
        self.channels
            .set_auto_episode(
                &rule.id,
                rule.version,
                -48,
                "첫 화가 49화라서 −48로 정했어요.",
            )
            .await
            .unwrap()
            .expect("the rule takes it")
    }

    async fn standing(&self, rule_id: &str, episode: i64) -> Standing {
        let payload = EpisodeUndo {
            rule_id: rule_id.to_owned(),
            episode,
        };
        standing(&self.channels, &payload).await.unwrap()
    }

    /// Begins the undo `command_id` of `−48` of `rule` with one file left to
    /// rename, and returns the item of the file.
    async fn begun(&self, rule: &Rule, command_id: &str) -> i64 {
        self.history
            .record(
                1_000,
                vec![Observation {
                    channel_id: self.channel.clone(),
                    channel_label: "https://feed.test/rss".into(),
                    identity_key: "49".into(),
                    title: "Show - 49".into(),
                    link: "magnet:?xt=urn:btih:abc".into(),
                    result: HistoryResult::Received,
                    rule_id: Some(rule.id.clone()),
                    torrent_hash: None,
                    reason: None,
                }],
            )
            .await
            .unwrap();
        let item = self
            .history
            .list(Default::default())
            .await
            .unwrap()
            .items
            .remove(0)
            .id;
        let file = NewUndoFile {
            item_id: item,
            folder: "/media/Show/Season 03".into(),
            from_name: "Show S03E01.mkv".into(),
            to_name: "Show S03E49.mkv".into(),
            torrent_hash: None,
            identity: None,
            kept: None,
        };
        let begun = self
            .channels
            .begin_episode_undo(command_id, &rule.id, -48, 1, vec![file], 2_000)
            .await
            .unwrap();
        assert!(matches!(begun, UndoBegun::Begun(_)), "{begun:?}");
        item
    }
}

#[tokio::test]
async fn a_request_is_for_the_automatic_value_asked_and_what_it_replaced() {
    let env = Env::new().await;

    // A rule that is gone.
    assert_eq!(env.standing("gone", -48).await, Standing::RuleGone);

    // A value the user typed is nothing to undo, whatever the request says.
    let typed = env.typed().await;
    for episode in [typed.episode, -48] {
        assert_eq!(env.standing(&typed.id, episode).await, Standing::Changed);
    }

    // The automatic value, asked for exactly: a new undo that puts back the
    // value it replaced. Another value is not it.
    let rule = env.automatic().await;
    assert_eq!(
        env.standing(&rule.id, -48).await,
        Standing::New {
            rule: rule.clone(),
            to: 1
        }
    );
    assert_eq!(env.standing(&rule.id, -40).await, Standing::Changed);

    // The user typed another value since: nothing automatic is left.
    let typed_since = env
        .channels
        .set_episode(&rule.id, rule.version, -40)
        .await
        .unwrap();
    assert_eq!(typed_since.episode, -40);
    assert_eq!(env.standing(&rule.id, -48).await, Standing::Changed);
    assert_eq!(env.standing(&rule.id, -40).await, Standing::Changed);
}

#[tokio::test]
async fn an_automatic_value_whose_predecessor_is_unknown_cannot_be_undone() {
    let env = Env::new().await;
    let rule = env.automatic().await;
    rusqlite::Connection::open(&env.path)
        .unwrap()
        .execute(
            "UPDATE rules SET episode_previous = NULL WHERE id = ?1",
            [&rule.id],
        )
        .unwrap();

    assert_eq!(env.standing(&rule.id, -48).await, Standing::PreviousUnknown);
    // The value is not the one asked for before it is asked what it replaced.
    assert_eq!(env.standing(&rule.id, -40).await, Standing::Changed);
}

/// An undo that began and left files to rename is carried on by a request
/// for the same value, though the rule is not automatic any more; for another
/// value, and once nothing is left, there is nothing to carry on.
#[tokio::test]
async fn an_undo_with_files_left_is_carried_on_for_the_same_value_only() {
    let env = Env::new().await;
    let rule = env.automatic().await;
    let item = env.begun(&rule, "undo-first").await;

    let earlier = env.channels.episode_undo("undo-first").await.unwrap();
    assert_eq!(
        env.standing(&rule.id, -48).await,
        Standing::CarryOn(earlier.unwrap())
    );
    assert_eq!(env.standing(&rule.id, -40).await, Standing::Changed);

    env.channels
        .finish_undo_file("undo-first", item, None, 4_000)
        .await
        .unwrap();
    assert_eq!(env.standing(&rule.id, -48).await, Standing::Changed);
}
