//! The episode offset the app sets for a rule (ticket 0024) in the store: who
//! may set it, what a save or an import does to its grounds, and `적용`.

use super::*;
use crate::store::anissia::Anime;

fn anime(no: i64) -> Anime {
    Anime {
        anime_no: no,
        subject: format!("작품 {no}"),
        original_subject: None,
        week: 3,
        air_time: Some("22:00".into()),
        start_date: Some("2026-10-07".into()),
        end_date: None,
        status: "ON".into(),
        fetched_at: 1,
    }
}

struct Env {
    store: ChannelStore,
    channel: String,
    /// The Anissia number of the next subscription: a channel keeps one rule
    /// per subscribed anime.
    next: std::cell::Cell<i64>,
}

impl Env {
    async fn new() -> Env {
        let store = ChannelStore::new(Db::open_blocking(":memory:").unwrap());
        let channel = store
            .create_channel(ChannelInput::new("https://feed.test/rss"))
            .await
            .unwrap()
            .id;
        Env {
            store,
            channel,
            next: std::cell::Cell::new(7),
        }
    }

    async fn subscription(&self, episode: i64) -> Rule {
        let no = self.next.get();
        self.next.set(no + 1);
        self.store
            .create_subscription_rule(
                &self.channel,
                RuleInput {
                    r#match: Some("Show".into()),
                    directory: "Show/Season 03".into(),
                    episode,
                    ..RuleInput::default()
                },
                NewSubscription {
                    anime: anime(no),
                    subtitles: SubtitleMode::Undecided,
                    creator: None,
                    subscribed_at: 100,
                },
            )
            .await
            .unwrap()
    }

    async fn save(&self, rule: &Rule, episode: i64, auto: bool) -> Rule {
        self.store
            .update_rule(
                &rule.id,
                rule.version,
                &self.channel,
                RuleInput {
                    episode,
                    episode_auto: auto,
                    ..rule.to_input()
                },
            )
            .await
            .unwrap()
    }
}

const BASIS: &str = "이전 시즌이 24화까지이고 첫 릴리스가 25화라서 −24로 정했어요.";

#[tokio::test]
async fn a_rule_with_a_neutral_offset_takes_the_apps_value_with_its_grounds() {
    for neutral in [0, 1] {
        let env = Env::new().await;
        let rule = env.subscription(neutral).await;

        let set = env
            .store
            .set_auto_episode(&rule.id, rule.version, -24, BASIS)
            .await
            .unwrap()
            .expect("the rule takes it");

        assert_eq!((set.episode, set.episode_auto), (-24, true));
        assert_eq!(set.version, rule.version + 1);
        let bases = env
            .store
            .episode_bases(vec![rule.id.clone()])
            .await
            .unwrap();
        assert_eq!(bases.get(&rule.id).map(String::as_str), Some(BASIS));
    }
}

#[tokio::test]
async fn the_app_never_replaces_what_the_user_set_or_saw_change() {
    let env = Env::new().await;

    // A value the user typed.
    let typed = env.subscription(-12).await;
    assert!(env
        .store
        .set_auto_episode(&typed.id, typed.version, -24, BASIS)
        .await
        .unwrap()
        .is_none());

    // A rule edited since the app read it.
    let edited = env.subscription(1).await;
    let saved = env.save(&edited, 1, false).await;
    assert!(env
        .store
        .set_auto_episode(&edited.id, edited.version, -24, BASIS)
        .await
        .unwrap()
        .is_none());
    assert_eq!(saved.episode, 1);

    // A value the app set already is not set again.
    let once = env.subscription(1).await;
    let first = env
        .store
        .set_auto_episode(&once.id, once.version, -24, BASIS)
        .await
        .unwrap()
        .unwrap();
    assert!(env
        .store
        .set_auto_episode(&once.id, first.version, -12, BASIS)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        env.store.get_rule(&once.id).await.unwrap().unwrap().episode,
        -24
    );
}

#[tokio::test]
async fn a_save_keeps_the_grounds_of_an_unchanged_value_and_drops_them_with_a_changed_one() {
    let env = Env::new().await;
    let rule = env.subscription(1).await;
    let auto = env
        .store
        .set_auto_episode(&rule.id, rule.version, -24, BASIS)
        .await
        .unwrap()
        .unwrap();

    let kept = env.save(&auto, -24, true).await;
    assert!(kept.episode_auto);
    assert_eq!(
        env.store
            .episode_bases(vec![rule.id.clone()])
            .await
            .unwrap()
            .len(),
        1
    );

    let changed = env.save(&kept, -12, false).await;
    assert!(!changed.episode_auto);
    assert!(env
        .store
        .episode_bases(vec![rule.id.clone()])
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn applying_a_suggestion_is_the_users_value_at_the_version_they_saw() {
    let env = Env::new().await;
    let rule = env.subscription(1).await;
    let auto = env
        .store
        .set_auto_episode(&rule.id, rule.version, -24, BASIS)
        .await
        .unwrap()
        .unwrap();

    // The same value as the app's own is still the user's after `적용`.
    let applied = env
        .store
        .set_episode(&rule.id, auto.version, -24)
        .await
        .unwrap();
    assert_eq!((applied.episode, applied.episode_auto), (-24, false));
    assert!(env
        .store
        .episode_bases(vec![rule.id.clone()])
        .await
        .unwrap()
        .is_empty());

    // The same value again changes nothing, version included.
    let same = env
        .store
        .set_episode(&rule.id, applied.version, -24)
        .await
        .unwrap();
    assert_eq!(same.version, applied.version);

    let stale = env.store.set_episode(&rule.id, rule.version, -12).await;
    assert!(
        matches!(stale, Err(ChannelError::Conflict { .. })),
        "{stale:?}"
    );
    let missing = env.store.set_episode("nope", 1, -12).await;
    assert!(
        matches!(missing, Err(ChannelError::NotFound { .. })),
        "{missing:?}"
    );
}

#[tokio::test]
async fn an_import_keeps_the_grounds_of_an_automatic_value_it_leaves_as_it_is() {
    use super::import::{ImportAction, ImportChannel};
    let env = Env::new().await;
    let rule = env.subscription(1).await;
    let auto = env
        .store
        .set_auto_episode(&rule.id, rule.version, -24, BASIS)
        .await
        .unwrap()
        .unwrap();
    let channel = env.store.get_channel(&env.channel).await.unwrap().unwrap();
    let replace = |episode: i64, episode_auto: bool| ImportAction::Replace {
        id: channel.id.clone(),
        expected_version: channel.version,
        channel: ImportChannel {
            input: ChannelInput::new("https://feed.test/rss"),
            rules: vec![RuleInput {
                episode,
                episode_auto,
                ..auto.to_input()
            }],
        },
    };

    // The same automatic value comes back as it went out: the grounds stay.
    env.store
        .import_channels(vec![replace(-24, true)])
        .await
        .unwrap();
    let bases = env
        .store
        .episode_bases(vec![rule.id.clone()])
        .await
        .unwrap();
    assert_eq!(bases.get(&rule.id).map(String::as_str), Some(BASIS));

    // A file with another value, or one the user typed, carries no grounds.
    let channel = env.store.get_channel(&env.channel).await.unwrap().unwrap();
    let replace_again = |episode: i64, episode_auto: bool| ImportAction::Replace {
        id: channel.id.clone(),
        expected_version: channel.version,
        channel: ImportChannel {
            input: ChannelInput::new("https://feed.test/rss"),
            rules: vec![RuleInput {
                episode,
                episode_auto,
                ..auto.to_input()
            }],
        },
    };
    env.store
        .import_channels(vec![replace_again(-12, true)])
        .await
        .unwrap();
    assert!(env
        .store
        .episode_bases(vec![rule.id.clone()])
        .await
        .unwrap()
        .is_empty());
    let after = env.store.get_rule(&rule.id).await.unwrap().unwrap();
    assert_eq!((after.episode, after.episode_auto), (-12, true));
}
