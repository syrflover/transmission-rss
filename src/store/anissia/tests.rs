use super::*;
use crate::store::channels::{
    ChannelError, ChannelInput, ChannelStore, NewSubscription, RuleInput, RuleState, SubtitleMode,
};

const DAY: i64 = REFRESH_AFTER_MS;

fn anime(no: i64, fetched_at: Millis) -> Anime {
    Anime {
        anime_no: no,
        subject: format!("작품 {no}"),
        original_subject: Some(format!("原題 {no}")),
        week: 3,
        air_time: Some("22:00".into()),
        start_date: Some("2026-10-07".into()),
        end_date: None,
        status: "ON".into(),
        fetched_at,
    }
}

fn rule(phrase: &str) -> RuleInput {
    RuleInput {
        r#match: Some(phrase.into()),
        directory: format!("{phrase}/Season 01"),
        ..RuleInput::default()
    }
}

fn subscription(no: i64, at: Millis) -> NewSubscription {
    NewSubscription {
        anime: anime(no, at),
        subtitles: SubtitleMode::Follow,
        creator: Some("에텔레로사".into()),
        subscribed_at: at,
    }
}

struct Env {
    db: Db,
    anissia: AnissiaStore,
    channels: ChannelStore,
    channel: String,
}

impl Env {
    async fn new() -> Env {
        let db = Db::open_blocking(":memory:").unwrap();
        let channels = ChannelStore::new(db.clone());
        let channel = channels
            .create_channel(ChannelInput::new("https://feed.test/rss"))
            .await
            .unwrap()
            .id;
        Env {
            anissia: AnissiaStore::new(db.clone()),
            db,
            channels,
            channel,
        }
    }

    async fn subscribe(&self, phrase: &str, no: i64, at: Millis) -> crate::store::channels::Rule {
        self.channels
            .create_subscription_rule(&self.channel, rule(phrase), subscription(no, at))
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn a_snapshot_is_replaced_by_the_newer_one_and_forgets_a_held_back_refresh() {
    let env = Env::new().await;
    env.anissia.put_anime(anime(7, 100)).await.unwrap();
    env.anissia.refresh_later(vec![7], 5_000).await.unwrap();
    let held: Option<i64> = env
        .db
        .run::<_, AnissiaStoreError, _>(|c| {
            Ok(c.query_row(
                "SELECT refresh_not_before FROM anissia_anime WHERE anime_no = 7",
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(held, Some(5_000));

    let mut newer = anime(7, 900);
    newer.subject = "바뀐 제목".into();
    newer.week = 8;
    newer.air_time = None;
    env.anissia.put_anime(newer.clone()).await.unwrap();

    assert_eq!(env.anissia.anime(7).await.unwrap(), Some(newer));
    assert_eq!(env.anissia.anime(8).await.unwrap(), None);
    let held: Option<i64> = env
        .db
        .run::<_, AnissiaStoreError, _>(|c| {
            Ok(c.query_row(
                "SELECT refresh_not_before FROM anissia_anime WHERE anime_no = 7",
                [],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(held, None);
}

#[tokio::test]
async fn a_subscription_rule_is_stored_with_its_snapshot_and_a_plain_rule_has_none() {
    let env = Env::new().await;
    let plain = env
        .channels
        .create_rule(&env.channel, rule("Plain"))
        .await
        .unwrap();
    let subscribed = env.subscribe("Work", 3320, 1_000).await;

    assert_eq!(plain.subscription, None);
    let sub = subscribed.subscription.clone().unwrap();
    assert_eq!(sub.anissia_anime_no, 3320);
    assert_eq!(sub.subtitles, SubtitleMode::Follow);
    assert_eq!(sub.creator.as_deref(), Some("에텔레로사"));
    assert_eq!(sub.season_id, None);
    assert_eq!(sub.subscribed_at, 1_000);
    // It goes last in the channel and reads back the same.
    assert_eq!(subscribed.position, 1);
    assert_eq!(
        env.channels.get_rule(&subscribed.id).await.unwrap(),
        Some(subscribed.clone())
    );
    let listed = env.channels.list_rules(&env.channel).await.unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].subscription, None);
    assert_eq!(listed[1], subscribed);
    // The snapshot is stored with it.
    assert_eq!(
        env.anissia.anime(3320).await.unwrap(),
        Some(anime(3320, 1_000))
    );
}

#[tokio::test]
async fn subtitles_are_followed_undecided_or_off_and_only_following_names_a_creator() {
    let env = Env::new().await;
    for (n, mode) in [(1, SubtitleMode::Undecided), (2, SubtitleMode::None)] {
        let rule = env
            .channels
            .create_subscription_rule(
                &env.channel,
                rule(&format!("Work {n}")),
                NewSubscription {
                    subtitles: mode,
                    creator: None,
                    ..subscription(n, 10)
                },
            )
            .await
            .unwrap();
        let stored = rule.subscription.unwrap();
        assert_eq!((stored.subtitles, stored.creator), (mode, None));
    }

    // A creator needs `follow`, and `follow` needs a non-blank creator.
    for (n, mode, creator) in [
        (3, SubtitleMode::Follow, None),
        (4, SubtitleMode::Follow, Some(String::new())),
        (5, SubtitleMode::Undecided, Some("에텔레로사".to_owned())),
        (6, SubtitleMode::None, Some("에텔레로사".to_owned())),
    ] {
        let refused = env
            .channels
            .create_subscription_rule(
                &env.channel,
                rule(&format!("Work {n}")),
                NewSubscription {
                    subtitles: mode,
                    creator,
                    ..subscription(n, 10)
                },
            )
            .await;
        assert!(matches!(refused, Err(ChannelError::Invalid(_))), "{n}");
        assert_eq!(env.anissia.anime(n).await.unwrap(), None);
    }
    assert_eq!(
        env.channels.list_rules(&env.channel).await.unwrap().len(),
        2
    );

    // The table refuses the same shapes on its own.
    let result = env
        .db
        .run::<_, AnissiaStoreError, _>(|c| {
            Ok(c.execute(
                "INSERT INTO rule_subscriptions (rule_id, anissia_anime_no, subtitles, creator, subscribed_at)
                 SELECT id, 1, 'none', '제작자', 1 FROM rules LIMIT 1",
                [],
            )
            .map(|_| ())?)
        })
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn a_channel_has_one_rule_for_an_anime_and_a_refusal_changes_nothing() {
    let env = Env::new().await;
    let first = env.subscribe("Work", 3320, 1_000).await;

    let again = env
        .channels
        .create_subscription_rule(&env.channel, rule("Work v2"), subscription(3320, 2_000))
        .await;
    match again {
        Err(ChannelError::AlreadySubscribed { rule_id }) => assert_eq!(rule_id, first.id),
        other => panic!("expected AlreadySubscribed, got {other:?}"),
    }
    assert_eq!(
        env.channels.list_rules(&env.channel).await.unwrap().len(),
        1
    );
    // The refused request did not touch the stored snapshot.
    assert_eq!(
        env.anissia.anime(3320).await.unwrap().unwrap().fetched_at,
        1_000
    );

    // Another channel may follow the same anime.
    let other = env
        .channels
        .create_channel(ChannelInput::new("https://other.test/rss"))
        .await
        .unwrap();
    env.channels
        .create_subscription_rule(&other.id, rule("Work"), subscription(3320, 3_000))
        .await
        .unwrap();
}

#[tokio::test]
async fn editing_a_rule_keeps_its_subscription_and_deleting_it_takes_the_subscription_along() {
    let env = Env::new().await;
    let rule = env.subscribe("Work", 3320, 1_000).await;

    let edited = env
        .channels
        .update_rule(
            &rule.id,
            rule.version,
            &env.channel,
            RuleInput {
                directory: "Work/Season 02".into(),
                ..rule.to_input()
            },
        )
        .await
        .unwrap();
    assert_eq!(edited.version, rule.version + 1);
    assert_eq!(edited.subscription, rule.subscription);

    env.channels
        .delete_rule(&rule.id, edited.version)
        .await
        .unwrap();
    let rows: i64 = env
        .db
        .run::<_, AnissiaStoreError, _>(|c| {
            Ok(c.query_row("SELECT count(*) FROM rule_subscriptions", [], |r| r.get(0))?)
        })
        .await
        .unwrap();
    assert_eq!(rows, 0);
    // The snapshot stays: the anime is still known.
    assert!(env.anissia.anime(3320).await.unwrap().is_some());
}

#[tokio::test]
async fn deleting_a_channel_takes_its_rules_subscriptions_along() {
    let env = Env::new().await;
    env.subscribe("Work", 3320, 1_000).await;
    let channel = env
        .channels
        .get_channel(&env.channel)
        .await
        .unwrap()
        .unwrap();
    env.channels
        .delete_channel(&env.channel, channel.version, 1)
        .await
        .unwrap();
    let rows: i64 = env
        .db
        .run::<_, AnissiaStoreError, _>(|c| {
            Ok(c.query_row("SELECT count(*) FROM rule_subscriptions", [], |r| r.get(0))?)
        })
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

#[tokio::test]
async fn a_subscription_needs_a_snapshot_of_its_anime() {
    let env = Env::new().await;
    let rule = env
        .channels
        .create_rule(&env.channel, rule("Work"))
        .await
        .unwrap();
    let result = env
        .db
        .run::<_, AnissiaStoreError, _>(move |c| {
            Ok(c.execute(
                "INSERT INTO rule_subscriptions (rule_id, anissia_anime_no, subscribed_at)
                 VALUES (?1, 99, 1)",
                [&rule.id],
            )
            .map(|_| ())?)
        })
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn only_the_snapshots_a_day_old_of_active_rules_are_due_and_held_ones_wait() {
    let env = Env::new().await;
    let now = 10 * DAY;
    // Fetched a day ago exactly, a day and a half ago, and an hour ago.
    let old = env.subscribe("Old", 1, now - DAY).await;
    env.subscribe("Older", 2, now - DAY - DAY / 2).await;
    env.subscribe("Fresh", 3, now - 60 * 60 * 1000).await;
    // An archived rule's anime is not refreshed.
    let archived = env.subscribe("Archived", 4, now - 3 * DAY).await;
    env.channels
        .set_rule_state(&archived.id, RuleState::Archived)
        .await
        .unwrap();

    let due = env.anissia.due(now).await.unwrap();
    assert_eq!(
        due,
        [
            Due {
                anime_no: 2,
                week: Some(3)
            },
            Due {
                anime_no: 1,
                week: Some(3)
            },
        ]
    );

    // A refresh held back waits until its time; two rules for one anime are one.
    env.anissia
        .refresh_later(vec![1], now + 1000)
        .await
        .unwrap();
    assert_eq!(
        env.anissia
            .due(now)
            .await
            .unwrap()
            .iter()
            .map(|d| d.anime_no)
            .collect::<Vec<_>>(),
        [2]
    );
    assert_eq!(
        env.anissia
            .due(now + 1000)
            .await
            .unwrap()
            .iter()
            .map(|d| d.anime_no)
            .collect::<Vec<_>>(),
        [2, 1]
    );
    let other = env
        .channels
        .create_channel(ChannelInput::new("https://other.test/rss"))
        .await
        .unwrap();
    env.channels
        .create_subscription_rule(&other.id, rule("Old"), subscription(1, now - DAY))
        .await
        .unwrap();
    assert_eq!(env.anissia.due(now + 1000).await.unwrap().len(), 2);
    let _ = old;
}

#[tokio::test]
async fn request_slots_keep_their_spacing_and_a_block_holds_every_request() {
    let env = Env::new().await;
    let store = &env.anissia;
    assert_eq!(
        store.take_request_slot(1000, 2000, None).await.unwrap(),
        Ok(1000)
    );
    assert_eq!(
        store.take_request_slot(1000, 2000, None).await.unwrap(),
        Ok(3000)
    );
    // A caller that may wait 1 s is told the wait instead of taking the slot.
    assert_eq!(
        store
            .take_request_slot(1000, 2000, Some(1000))
            .await
            .unwrap(),
        Err(4000)
    );
    assert_eq!(
        store.take_request_slot(1000, 2000, None).await.unwrap(),
        Ok(5000)
    );

    store.block_requests(60_000).await.unwrap();
    assert_eq!(
        store.take_request_slot(7000, 2000, None).await.unwrap(),
        Ok(60_000)
    );
    // A shorter block does not shorten a longer one.
    store.block_requests(10_000).await.unwrap();
    assert_eq!(
        store.take_request_slot(8000, 2000, None).await.unwrap(),
        Ok(62_000)
    );
}
