use std::path::PathBuf;

use tempfile::TempDir;

use crate::store::channels::{
    import::{ImportAction, ImportChannel},
    import_subscriptions::{ImportSubscription, SubscriptionOutcome},
    ChannelError, ChannelInput, ChannelStore, ChannelWithRules, Db, NewSubscription, RuleInput,
    SubtitleMode,
};
use trss_anissia::Anime;

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
    fn raw(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(&self.path).unwrap()
    }

    /// `(subject, week, fetched_at)` of the snapshot of an anime.
    fn snapshot(&self, anime_no: i64) -> Option<(String, i64, i64)> {
        self.raw()
            .query_row(
                "SELECT subject, week, fetched_at FROM anissia_anime WHERE anime_no = ?1",
                [anime_no],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .ok()
    }
}

fn rule(phrase: &str) -> RuleInput {
    RuleInput {
        r#match: Some(phrase.to_owned()),
        directory: format!("{phrase}/Season 01"),
        ..RuleInput::default()
    }
}

fn channel(url: &str, phrases: &[&str]) -> ImportChannel {
    ImportChannel {
        input: ChannelInput::new(url),
        rules: phrases.iter().map(|p| rule(p)).collect(),
    }
}

fn anime(no: i64, subject: &str, week: u8, fetched_at: i64) -> Anime {
    Anime {
        anime_no: no,
        subject: subject.to_owned(),
        original_subject: None,
        week,
        air_time: Some("22:30".into()),
        start_date: None,
        end_date: None,
        status: "ON".into(),
        fetched_at,
    }
}

fn follow(action: usize, rule: usize, anime: Anime, creator: Option<&str>) -> ImportSubscription {
    ImportSubscription {
        action,
        rule,
        subscription: NewSubscription {
            anime,
            subtitles: if creator.is_some() {
                SubtitleMode::Follow
            } else {
                SubtitleMode::Undecided
            },
            creator: creator.map(str::to_owned),
            // The import stamps its own time.
            subscribed_at: 0,
        },
        placeholder: false,
    }
}

fn subscribed(channel: &ChannelWithRules) -> Vec<(Option<i64>, Option<&str>)> {
    channel
        .rules
        .iter()
        .map(|r| {
            (
                r.subscription.as_ref().map(|s| s.anissia_anime_no),
                r.subscription.as_ref().and_then(|s| s.creator.as_deref()),
            )
        })
        .collect()
}

#[tokio::test]
async fn checked_rules_become_subscriptions_with_the_import_time_and_the_others_do_not() {
    let f = fixture().await;
    let actions = vec![
        ImportAction::Add(channel("https://a.example/rss", &["A", "B", "C"])),
        ImportAction::Add(channel("https://b.example/rss", &["D"])),
    ];
    let subs = vec![
        follow(0, 0, anime(10, "에이", 3, 1_000), Some("Team")),
        follow(0, 2, anime(11, "씨", 5, 1_000), None),
        follow(1, 0, anime(10, "에이", 3, 1_000), Some("Other")),
    ];
    let (results, outcomes) = f
        .store
        .import_channels_subscribing(actions, None, subs, || 5_000)
        .await
        .unwrap();
    assert_eq!(outcomes, vec![SubscriptionOutcome::Created; 3]);

    let first = results[0].channel();
    assert_eq!(
        subscribed(first),
        [(Some(10), Some("Team")), (None, None), (Some(11), None)]
    );
    let stored = first.rules[0].subscription.as_ref().unwrap();
    assert_eq!(stored.subtitles, SubtitleMode::Follow);
    assert_eq!(stored.subscribed_at, 5_000);
    assert_eq!(stored.season_id, None);
    assert_eq!(
        first.rules[2].subscription.as_ref().unwrap().subtitles,
        SubtitleMode::Undecided
    );
    // The same anime may be followed in another channel.
    assert_eq!(
        subscribed(results[1].channel()),
        [(Some(10), Some("Other"))]
    );
    // The result is what was stored.
    let stored = f.store.list_channels_with_rules().await.unwrap();
    assert_eq!(subscribed(&stored[0]), subscribed(first));
    assert_eq!(f.snapshot(10), Some(("에이".into(), 3, 1_000)));
}

#[tokio::test]
async fn nothing_of_the_import_stays_when_a_later_part_fails() {
    let f = fixture().await;
    let existing = f
        .store
        .create_channel_with_rules(ChannelInput::new("https://a.example/rss"), vec![rule("A")])
        .await
        .unwrap();
    let actions = vec![
        ImportAction::Add(channel("https://new.example/rss", &["N"])),
        ImportAction::Replace {
            id: existing.channel.id.clone(),
            expected_version: existing.channel.version + 7,
            channel: channel("https://a.example/rss", &["A"]),
        },
    ];
    let subs = vec![follow(0, 0, anime(10, "에이", 3, 1_000), Some("Team"))];
    let error = f
        .store
        .import_channels_subscribing(actions, None, subs, || 5_000)
        .await
        .unwrap_err();
    assert!(error.is_conflict(), "{error}");

    let stored = f.store.list_channels_with_rules().await.unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(f.snapshot(10), None);
    let count: i64 = f
        .raw()
        .query_row("SELECT COUNT(*) FROM rule_subscriptions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn a_replaced_rule_that_is_a_subscription_stays_as_it_is() {
    let f = fixture().await;
    let existing = f
        .store
        .create_channel_with_rules(
            ChannelInput::new("https://a.example/rss"),
            vec![rule("Kept"), rule("Plain")],
        )
        .await
        .unwrap();
    f.store
        .subscribe_rule(
            &existing.rules[0].id,
            existing.rules[0].version,
            follow(0, 0, anime(20, "기존", 1, 900), None).subscription,
        )
        .await
        .unwrap();
    let existing = f.store.list_channels_with_rules().await.unwrap().remove(0);

    let actions = vec![ImportAction::Replace {
        id: existing.channel.id.clone(),
        expected_version: existing.channel.version,
        channel: channel("https://a.example/rss", &["Kept", "Plain"]),
    }];
    let subs = vec![
        follow(0, 0, anime(21, "다른", 2, 1_000), Some("Team")),
        follow(0, 1, anime(22, "새", 2, 1_000), Some("Team")),
    ];
    let (results, outcomes) = f
        .store
        .import_channels_subscribing(actions, None, subs, || 5_000)
        .await
        .unwrap();
    assert_eq!(
        outcomes,
        [
            SubscriptionOutcome::RuleAlreadySubscribed,
            SubscriptionOutcome::Created
        ]
    );
    // The kept rule follows the anime it did; the one it was offered is not stored.
    assert_eq!(
        subscribed(results[0].channel()),
        [(Some(20), None), (Some(22), Some("Team"))]
    );
    assert_eq!(f.snapshot(21), None);
}

#[tokio::test]
async fn a_channel_follows_an_anime_with_one_rule() {
    let f = fixture().await;
    let actions = vec![ImportAction::Add(channel(
        "https://a.example/rss",
        &["A", "B"],
    ))];
    let subs = vec![
        follow(0, 0, anime(10, "에이", 3, 1_000), Some("Team")),
        follow(0, 1, anime(10, "에이", 3, 1_000), Some("Team")),
    ];
    let (results, outcomes) = f
        .store
        .import_channels_subscribing(actions, None, subs, || 5_000)
        .await
        .unwrap();
    let first_rule = results[0].channel().rules[0].id.clone();
    assert_eq!(
        outcomes,
        [
            SubscriptionOutcome::Created,
            SubscriptionOutcome::AnimeTakenInChannel {
                rule_id: first_rule
            }
        ]
    );
    assert_eq!(
        subscribed(results[0].channel()),
        [(Some(10), Some("Team")), (None, None)]
    );
}

#[tokio::test]
async fn a_stand_in_snapshot_never_replaces_one_the_app_has_and_a_real_one_does() {
    let f = fixture().await;
    let stand_in = |no: i64| ImportSubscription {
        placeholder: true,
        ..follow(
            0,
            0,
            ImportSubscription::stand_in(no, "[SubsPlease] A - ", None),
            None,
        )
    };

    // Nothing known: the stand-in is kept, due at once (never received).
    f.store
        .import_channels_subscribing(
            vec![ImportAction::Add(channel("https://a.example/rss", &["A"]))],
            None,
            vec![stand_in(10)],
            || 5_000,
        )
        .await
        .unwrap();
    let (subject, week, fetched_at) = f.snapshot(10).unwrap();
    assert_eq!(
        (subject.as_str(), week, fetched_at),
        ("[SubsPlease] A - ", 7, 0)
    );

    // A real snapshot replaces it.
    f.store
        .import_channels_subscribing(
            vec![ImportAction::Add(channel("https://b.example/rss", &["A"]))],
            None,
            vec![follow(0, 0, anime(10, "에이", 3, 2_000), None)],
            || 5_000,
        )
        .await
        .unwrap();
    assert_eq!(f.snapshot(10), Some(("에이".into(), 3, 2_000)));

    // A stand-in does not replace the real one.
    f.store
        .import_channels_subscribing(
            vec![ImportAction::Add(channel("https://c.example/rss", &["A"]))],
            None,
            vec![stand_in(10)],
            || 5_000,
        )
        .await
        .unwrap();
    assert_eq!(f.snapshot(10), Some(("에이".into(), 3, 2_000)));
}

#[tokio::test]
async fn a_subscription_that_does_not_fit_the_import_is_refused_before_anything_is_written() {
    let f = fixture().await;
    let add = || vec![ImportAction::Add(channel("https://a.example/rss", &["A"]))];
    let refuse = |subs: Vec<ImportSubscription>| {
        let store = f.store.clone();
        async move {
            store
                .import_channels_subscribing(add(), None, subs, || 5_000)
                .await
                .unwrap_err()
        }
    };

    let past_the_actions = follow(1, 0, anime(10, "에이", 3, 1), Some("Team"));
    assert!(matches!(
        refuse(vec![past_the_actions]).await,
        ChannelError::Invalid(_)
    ));
    let past_the_rules = follow(0, 1, anime(10, "에이", 3, 1), Some("Team"));
    assert!(matches!(
        refuse(vec![past_the_rules]).await,
        ChannelError::Invalid(_)
    ));
    // `follow` needs a creator.
    let mut no_creator = follow(0, 0, anime(10, "에이", 3, 1), None);
    no_creator.subscription.subtitles = SubtitleMode::Follow;
    assert!(matches!(
        refuse(vec![no_creator]).await,
        ChannelError::Invalid(_)
    ));
    assert!(f.store.list_channels_with_rules().await.unwrap().is_empty());
}

#[tokio::test]
async fn the_import_time_is_read_while_the_transaction_holds_the_write_lock() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    let f = fixture().await;
    let held = Arc::new(AtomicBool::new(false));
    let probe = {
        let (path, held) = (f.path.clone(), held.clone());
        move || {
            // Nobody else can start a write while the import is writing.
            let other = rusqlite::Connection::open(&path).unwrap();
            other.busy_timeout(std::time::Duration::ZERO).unwrap();
            held.store(
                other.execute_batch("BEGIN IMMEDIATE").is_err(),
                Ordering::SeqCst,
            );
            7_000
        }
    };
    let (results, _) = f
        .store
        .import_channels_subscribing(
            vec![ImportAction::Add(channel("https://a.example/rss", &["A"]))],
            None,
            vec![follow(0, 0, anime(10, "에이", 3, 1_000), None)],
            probe,
        )
        .await
        .unwrap();
    assert!(
        held.load(Ordering::SeqCst),
        "the clock was read outside the transaction"
    );
    let stored = results[0].channel().rules[0].subscription.as_ref().unwrap();
    assert_eq!(stored.subscribed_at, 7_000);
}

#[test]
fn a_stand_in_sits_on_the_comments_weekday_or_in_the_other_tab() {
    use trss_anissia::WEEK_OTHER;

    let on_wednesday = ImportSubscription::stand_in(10, "A", Some((3, "22:30")));
    assert_eq!(
        (on_wednesday.week, on_wednesday.air_time.as_deref()),
        (3, Some("22:30"))
    );
    // Never received, so the daily refresh finds it due and replaces it.
    assert_eq!(
        (on_wednesday.fetched_at, on_wednesday.status.as_str()),
        (0, "OFF")
    );

    let without = ImportSubscription::stand_in(10, "A", None);
    assert_eq!((without.week, without.air_time), (WEEK_OTHER, None));
    // A number that is not one of the seven weekdays stays in `기타`.
    let odd = ImportSubscription::stand_in(10, "A", Some((7, "22:30")));
    assert_eq!((odd.week, odd.air_time), (WEEK_OTHER, None));
}
