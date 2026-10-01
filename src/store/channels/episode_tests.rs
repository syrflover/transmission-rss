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

const BASIS: &str = "이전 시즌이 24화까지이고 첫 화가 25화라서 회차 변환을 −24로 정했어요.";

impl Env {
    async fn mark(&self, rule: &Rule) -> EpisodeMark {
        self.store
            .episode_marks(vec![rule.id.clone()])
            .await
            .unwrap()
            .remove(&rule.id)
            .unwrap()
    }
}

/// The app's value replaces whatever a rule it has not decided holds: the
/// neutral `0` and `1`, a value carried over from the previous season, one
/// the user typed (user decision, 2026-10-02). The value it replaced is kept.
#[tokio::test]
async fn a_rule_the_app_has_not_decided_takes_its_value_whatever_it_held() {
    for held in [0, 1, -24, -12] {
        let env = Env::new().await;
        let rule = env.subscription(held).await;
        assert_eq!(env.mark(&rule).await, EpisodeMark::default());

        let set = env
            .store
            .set_auto_episode(&rule.id, rule.version, -48, BASIS)
            .await
            .unwrap()
            .expect("the rule takes it");

        assert_eq!((set.episode, set.episode_auto), (-48, true));
        assert_eq!(set.version, rule.version + 1);
        assert_eq!(
            env.mark(&rule).await,
            EpisodeMark {
                basis: Some(BASIS.to_owned()),
                previous: Some(held),
                decided: true,
            },
            "{held}"
        );
    }
}

#[tokio::test]
async fn the_app_decides_once_and_never_at_a_version_it_did_not_read() {
    let env = Env::new().await;

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

    // Nor after the user changed it: the app has decided the rule.
    let changed = env.save(&first, -12, false).await;
    assert!(env
        .store
        .set_auto_episode(&once.id, changed.version, -24, BASIS)
        .await
        .unwrap()
        .is_none());
    let mark = env.mark(&once).await;
    assert_eq!(
        (mark.basis, mark.previous, mark.decided),
        (None, None, true)
    );

    // Nor after `적용` turned it into the user's.
    let applied = env.subscription(1).await;
    let auto = env
        .store
        .set_auto_episode(&applied.id, applied.version, -24, BASIS)
        .await
        .unwrap()
        .unwrap();
    let own = env
        .store
        .set_episode(&applied.id, auto.version, 0)
        .await
        .unwrap();
    assert!(env
        .store
        .set_auto_episode(&applied.id, own.version, -24, BASIS)
        .await
        .unwrap()
        .is_none());
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
    let mark = env.mark(&rule).await;
    assert_eq!(
        (mark.basis.as_deref(), mark.previous),
        (Some(BASIS), Some(1))
    );

    let changed = env.save(&kept, -12, false).await;
    assert!(!changed.episode_auto);
    let mark = env.mark(&rule).await;
    assert_eq!(
        (mark.basis, mark.previous, mark.decided),
        (None, None, true)
    );
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
    let mark = env.mark(&rule).await;
    assert_eq!((mark.basis, mark.previous), (None, None));

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
    let mark = env.mark(&rule).await;
    assert_eq!(
        (mark.basis.as_deref(), mark.previous),
        (Some(BASIS), Some(1))
    );

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
    let mark = env.mark(&rule).await;
    assert_eq!((mark.basis, mark.previous), (None, None));
    let after = env.store.get_rule(&rule.id).await.unwrap().unwrap();
    assert_eq!((after.episode, after.episode_auto), (-12, true));
}
