//! Title-waiting subscriptions in the store: giving one its title, the time it
//! was given, and the titles the user rejected.

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
}

impl Env {
    async fn new() -> Env {
        let db = Db::open_blocking(":memory:").unwrap();
        let store = ChannelStore::new(db);
        let channel = store
            .create_channel(ChannelInput::new("https://feed.test/rss"))
            .await
            .unwrap()
            .id;
        Env { store, channel }
    }

    /// A subscription with no match phrase yet, subscribed at `at`.
    async fn waiting(&self, no: i64, at: i64) -> Rule {
        self.store
            .create_subscription_rule(
                &self.channel,
                RuleInput {
                    r#match: None,
                    directory: format!("작품 {no}"),
                    ..RuleInput::default()
                },
                NewSubscription {
                    anime: anime(no),
                    subtitles: SubtitleMode::Undecided,
                    creator: None,
                    subscribed_at: at,
                },
            )
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn a_waiting_subscription_has_no_phrase_and_no_title_time() {
    let env = Env::new().await;
    let rule = env.waiting(7, 100).await;
    assert_eq!(rule.r#match, None);
    assert_eq!(rule.state, RuleState::Active);
    let subscription = rule.subscription.unwrap();
    assert_eq!(
        (subscription.subscribed_at, subscription.titled_at),
        (100, None)
    );
}

#[tokio::test]
async fn giving_a_title_sets_the_phrase_and_notes_when_and_keeps_the_rest() {
    let env = Env::new().await;
    let rule = env.waiting(7, 100).await;

    let titled = env
        .store
        .give_title(&rule.id, rule.version, "New Work", None, 500)
        .await
        .unwrap();

    assert_eq!(titled.r#match.as_deref(), Some("New Work"));
    assert_eq!(titled.directory, "작품 7");
    assert_eq!(titled.version, rule.version + 1);
    assert_eq!(titled.state, RuleState::Active);
    assert_eq!(titled.resumed_at, None);
    let subscription = titled.subscription.unwrap();
    assert_eq!(
        (subscription.subscribed_at, subscription.titled_at),
        (100, Some(500))
    );
    assert_eq!(subscription.anissia_anime_no, 7);
}

#[tokio::test]
async fn giving_a_title_can_replace_the_save_folder() {
    let env = Env::new().await;
    let rule = env.waiting(7, 100).await;
    let titled = env
        .store
        .give_title(
            &rule.id,
            rule.version,
            "New Work",
            Some("New Work/Season 01"),
            500,
        )
        .await
        .unwrap();
    assert_eq!(titled.directory, "New Work/Season 01");

    // An absolute folder is refused and nothing is written.
    let rule = env.waiting(8, 100).await;
    let refused = env
        .store
        .give_title(&rule.id, rule.version, "Other", Some("/etc"), 500)
        .await;
    assert!(
        matches!(refused, Err(ChannelError::Invalid(_))),
        "{refused:?}"
    );
    let kept = env.store.get_rule(&rule.id).await.unwrap().unwrap();
    assert_eq!((kept.r#match, kept.version), (None, rule.version));
}

#[tokio::test]
async fn a_title_is_given_to_a_collecting_waiting_subscription_only() {
    let env = Env::new().await;

    // A stale version conflicts.
    let rule = env.waiting(7, 100).await;
    let stale = env
        .store
        .give_title(&rule.id, rule.version + 5, "New Work", None, 500)
        .await;
    assert!(matches!(stale, Err(ref e) if e.is_conflict()), "{stale:?}");

    // An empty title is refused.
    let empty = env
        .store
        .give_title(&rule.id, rule.version, "", None, 500)
        .await;
    assert!(matches!(empty, Err(ChannelError::Invalid(_))));

    // A paused one is refused until it is turned on.
    let paused = env
        .store
        .set_video_receiving(&rule.id, rule.version, false, 200)
        .await
        .unwrap();
    let refused = env
        .store
        .give_title(&rule.id, paused.version, "New Work", None, 500)
        .await;
    assert!(
        matches!(refused, Err(ChannelError::Invalid(_))),
        "{refused:?}"
    );

    // One that has a title already, and a plain rule, are refused too.
    let on = env
        .store
        .set_video_receiving(&rule.id, paused.version, true, 300)
        .await
        .unwrap();
    let titled = env
        .store
        .give_title(&rule.id, on.version, "New Work", None, 500)
        .await
        .unwrap();
    let again = env
        .store
        .give_title(&rule.id, titled.version, "Another", None, 600)
        .await;
    assert!(matches!(again, Err(ChannelError::Invalid(_))), "{again:?}");
    let plain = env
        .store
        .create_rule(
            &env.channel,
            RuleInput {
                r#match: None,
                ..RuleInput::default()
            },
        )
        .await
        .unwrap();
    let plain_refused = env
        .store
        .give_title(&plain.id, plain.version, "New Work", None, 500)
        .await;
    assert!(matches!(plain_refused, Err(ChannelError::Invalid(_))));
}

#[tokio::test]
async fn saving_a_phrase_into_a_waiting_subscription_notes_the_time_like_giving_a_title() {
    let env = Env::new().await;
    let rule = env.waiting(7, 100).await;

    let saved = env
        .store
        .update_rule_at(
            &rule.id,
            rule.version,
            &env.channel,
            RuleInput {
                r#match: Some("New Work".into()),
                ..rule.to_input()
            },
            700,
        )
        .await
        .unwrap();
    assert_eq!(saved.subscription.as_ref().unwrap().titled_at, Some(700));

    // Saving the other fields of a subscription that has its phrase notes nothing.
    let again = env
        .store
        .update_rule_at(
            &rule.id,
            saved.version,
            &env.channel,
            RuleInput {
                directory: "Elsewhere".into(),
                ..saved.to_input()
            },
            900,
        )
        .await
        .unwrap();
    assert_eq!(again.subscription.unwrap().titled_at, Some(700));

    // A plain rule given a phrase is not a subscription: nothing is noted, and
    // it keeps taking what was recorded before, as it always did.
    let plain = env
        .store
        .create_rule(
            &env.channel,
            RuleInput {
                r#match: None,
                ..RuleInput::default()
            },
        )
        .await
        .unwrap();
    let plain = env
        .store
        .update_rule_at(
            &plain.id,
            plain.version,
            &env.channel,
            RuleInput {
                r#match: Some("Plain".into()),
                ..plain.to_input()
            },
            800,
        )
        .await
        .unwrap();
    assert!(plain.subscription.is_none());
}

#[tokio::test]
async fn a_rejected_title_is_remembered_once_and_goes_with_its_channel() {
    let env = Env::new().await;
    assert!(env.store.rejected_titles().await.unwrap().is_empty());

    env.store
        .reject_title(&env.channel, "new work", "New Work", 100)
        .await
        .unwrap();
    // Asked again, with another spelling and time, the first stands.
    env.store
        .reject_title(&env.channel, "new work", "NEW WORK", 200)
        .await
        .unwrap();
    let rejected = env.store.rejected_titles().await.unwrap();
    assert_eq!(rejected.len(), 1);
    assert!(rejected.contains(&(env.channel.clone(), "new work".to_owned())));

    let missing = env.store.reject_title("no-such-channel", "x", "X", 1).await;
    assert!(
        matches!(missing, Err(ChannelError::NotFound { .. })),
        "{missing:?}"
    );
    let blank = env.store.reject_title(&env.channel, "", "X", 1).await;
    assert!(matches!(blank, Err(ChannelError::Invalid(_))));

    let channel = env.store.get_channel(&env.channel).await.unwrap().unwrap();
    env.store
        .delete_channel(&channel.id, channel.version, 0)
        .await
        .unwrap();
    assert!(env.store.rejected_titles().await.unwrap().is_empty());
}
