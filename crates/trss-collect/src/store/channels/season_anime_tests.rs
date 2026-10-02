//! A season's Anissia anime and the subscriptions that hold seasons: the two
//! stay one thing (`super::season_anime`).

use std::collections::BTreeSet;

use trss_anissia::Anime;
use trss_core::Db;
use trss_library::{
    discovery::{Scan, ScannedWork, WorkRead},
    store::{library::LibraryStore, seasons::SeasonStore},
};

use crate::store::{
    anissia::AnissiaStore,
    channels::{
        ChannelInput, ChannelStore, NewSubscription, Rule, RuleInput, SeasonAnimeError,
        SeasonLinked, SubtitleMode,
    },
};

fn anime(no: i64, status: &str) -> Anime {
    Anime {
        anime_no: no,
        subject: format!("작품 {no}"),
        original_subject: None,
        week: 3,
        air_time: Some("22:00".into()),
        start_date: Some("2021-04-04".into()),
        end_date: None,
        status: status.into(),
        fetched_at: 1_000,
    }
}

struct Env {
    db: Db,
    channels: ChannelStore,
    anissia: AnissiaStore,
    seasons: SeasonStore,
    channel: String,
    /// The work `Show` with seasons 1 and 2.
    work: String,
}

impl Env {
    async fn new() -> Env {
        let db = Db::open_blocking(":memory:").unwrap();
        let library = LibraryStore::new(db.clone());
        let scan = Scan {
            works: vec![WorkRead::Read(ScannedWork {
                dir_name: "Show".into(),
                seasons: BTreeSet::from([1, 2]),
                files: Vec::new(),
                unrecognized: Vec::new(),
            })],
        };
        let (folder, _) = library
            .add_folder("/shows".into(), scan, 100, &[])
            .await
            .unwrap();
        let work = library.works(&folder.id).await.unwrap().remove(0).id;
        let channels = ChannelStore::new(db.clone());
        let channel = channels
            .create_channel(ChannelInput::new("https://feed.test/rss"))
            .await
            .unwrap()
            .id;
        Env {
            anissia: AnissiaStore::new(db.clone()),
            seasons: SeasonStore::new(db.clone()),
            db,
            channels,
            channel,
            work,
        }
    }

    fn season(&self, number: u32) -> String {
        format!("{}:{number}", self.work)
    }

    async fn subscribe(&self, phrase: &str, no: i64) -> Rule {
        self.channels
            .create_subscription_rule(
                &self.channel,
                RuleInput {
                    r#match: Some(phrase.into()),
                    directory: format!("{phrase}/Season 01"),
                    ..RuleInput::default()
                },
                NewSubscription {
                    anime: anime(no, "ON"),
                    subtitles: SubtitleMode::Undecided,
                    creator: None,
                    subscribed_at: 1_000,
                },
            )
            .await
            .unwrap()
    }

    async fn link(&self, season: u32) -> (i64, Option<i64>) {
        let link = self.seasons.anissia_link(&self.work, season).await.unwrap();
        (link.version, link.anime_no)
    }
}

#[tokio::test]
async fn an_unsubscribed_season_is_linked_to_a_finished_anime_changed_and_cut() {
    let env = Env::new().await;

    // The finished anime has no schedule entry: the link stores its snapshot.
    let linked = env
        .channels
        .set_season_anime(&env.work, 1, 0, Some(anime(29, "END")))
        .await
        .unwrap();
    assert_eq!((linked.version, linked.anime_no), (1, Some(29)));
    let snapshot = env.anissia.anime(29).await.unwrap().unwrap();
    assert_eq!(
        (snapshot.subject.as_str(), snapshot.status.as_str()),
        ("작품 29", "END")
    );

    let changed = env
        .channels
        .set_season_anime(&env.work, 1, 1, Some(anime(30, "ON")))
        .await
        .unwrap();
    assert_eq!((changed.version, changed.anime_no), (2, Some(30)));
    let cut = env
        .channels
        .set_season_anime(&env.work, 1, 2, None)
        .await
        .unwrap();
    assert_eq!((cut.version, cut.anime_no), (3, None));
    // The other season was never touched.
    assert_eq!(env.link(2).await, (0, None));
    // A work the library does not have has no season to link.
    assert!(matches!(
        env.channels
            .set_season_anime("nobody", 1, 0, Some(anime(29, "END")))
            .await,
        Err(SeasonAnimeError::NoWork)
    ));
}

#[tokio::test]
async fn a_change_from_an_older_version_is_refused_with_the_current_link_and_changes_nothing() {
    let env = Env::new().await;
    env.channels
        .set_season_anime(&env.work, 1, 0, Some(anime(29, "END")))
        .await
        .unwrap();

    // A second screen still shows the season as never linked.
    let late = env
        .channels
        .set_season_anime(&env.work, 1, 0, Some(anime(31, "END")))
        .await;
    match late {
        Err(SeasonAnimeError::Conflict(current)) => {
            assert_eq!((current.version, current.anime_no), (1, Some(29)))
        }
        other => panic!("expected a conflict, got {other:?}"),
    }
    assert_eq!(env.link(1).await, (1, Some(29)));
    // The refused change did not store the anime it named.
    assert_eq!(env.anissia.anime(31).await.unwrap(), None);
    // Cutting from the old version is refused the same way.
    assert!(matches!(
        env.channels.set_season_anime(&env.work, 1, 0, None).await,
        Err(SeasonAnimeError::Conflict(_))
    ));
    assert_eq!(env.link(1).await, (1, Some(29)));
}

#[tokio::test]
async fn connecting_a_subscription_links_its_season_to_its_anime_once() {
    let env = Env::new().await;
    let rule = env.subscribe("Show", 7).await;
    assert_eq!(env.link(2).await, (0, None));

    assert_eq!(
        env.channels
            .link_season(&rule.id, &env.season(2))
            .await
            .unwrap(),
        SeasonLinked::Linked
    );
    assert_eq!(env.link(2).await, (1, Some(7)));
    assert_eq!(env.link(1).await, (0, None));

    // Connected already: nothing moves, and a rule of the same anime in another
    // channel shares the season without moving the link either.
    assert_eq!(
        env.channels
            .link_season(&rule.id, &env.season(1))
            .await
            .unwrap(),
        SeasonLinked::Kept
    );
    assert_eq!(env.link(1).await, (0, None));
    let channel = env
        .channels
        .create_channel(ChannelInput::new("https://feed2.test/rss"))
        .await
        .unwrap();
    let same = env
        .channels
        .create_subscription_rule(
            &channel.id,
            RuleInput {
                r#match: Some("Same".into()),
                directory: "Same".into(),
                ..RuleInput::default()
            },
            NewSubscription {
                anime: anime(7, "ON"),
                subtitles: SubtitleMode::None,
                creator: None,
                subscribed_at: 1_000,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        env.channels
            .link_season(&same.id, &env.season(2))
            .await
            .unwrap(),
        SeasonLinked::Linked
    );
    assert_eq!(env.link(2).await, (1, Some(7)));
}

#[tokio::test]
async fn a_subscribed_season_cannot_be_changed_or_cut_from_the_work_detail() {
    let env = Env::new().await;
    let rule = env.subscribe("Show", 7).await;
    env.channels
        .link_season(&rule.id, &env.season(2))
        .await
        .unwrap();
    let snapshots_before = env.anissia.anime(8).await.unwrap();

    for anime in [Some(anime(8, "END")), Some(anime(7, "ON")), None] {
        match env.channels.set_season_anime(&env.work, 2, 1, anime).await {
            Err(SeasonAnimeError::Subscribed { rule_id, anime_no }) => {
                assert_eq!((rule_id, anime_no), (rule.id.clone(), 7))
            }
            other => panic!("expected the subscription to hold the season, got {other:?}"),
        }
    }
    assert_eq!(env.link(2).await, (1, Some(7)));
    // The refusal came before anything was stored, even a stale version's.
    assert_eq!(env.anissia.anime(8).await.unwrap(), snapshots_before);
    assert!(matches!(
        env.channels
            .set_season_anime(&env.work, 2, 0, Some(anime(8, "END")))
            .await,
        Err(SeasonAnimeError::Subscribed { .. })
    ));
    // The other season of the work is free.
    env.channels
        .set_season_anime(&env.work, 1, 0, Some(anime(8, "END")))
        .await
        .unwrap();

    // A paused or archived rule still holds its season.
    let connected = env.channels.get_rule(&rule.id).await.unwrap().unwrap();
    env.channels
        .set_video_receiving(&rule.id, connected.version, false, 0)
        .await
        .unwrap();
    assert!(matches!(
        env.channels.set_season_anime(&env.work, 2, 1, None).await,
        Err(SeasonAnimeError::Subscribed { .. })
    ));
}

#[tokio::test]
async fn deleting_the_subscription_leaves_the_link_and_frees_the_season() {
    let env = Env::new().await;
    let rule = env.subscribe("Show", 7).await;
    env.channels
        .link_season(&rule.id, &env.season(2))
        .await
        .unwrap();

    let connected = env.channels.get_rule(&rule.id).await.unwrap().unwrap();
    env.channels
        .delete_rule(&rule.id, connected.version)
        .await
        .unwrap();

    assert_eq!(env.link(2).await, (1, Some(7)));
    let cut = env
        .channels
        .set_season_anime(&env.work, 2, 1, None)
        .await
        .unwrap();
    assert_eq!((cut.version, cut.anime_no), (2, None));
}

#[tokio::test]
async fn a_season_linked_to_another_anime_is_not_taken_by_a_subscription_and_says_so() {
    let env = Env::new().await;
    env.channels
        .set_season_anime(&env.work, 2, 0, Some(anime(29, "END")))
        .await
        .unwrap();
    let rule = env.subscribe("Show", 7).await;
    let season = env.season(2);

    assert_eq!(
        env.channels.link_season(&rule.id, &season).await.unwrap(),
        SeasonLinked::Taken
    );
    let noted = env.channels.get_rule(&rule.id).await.unwrap().unwrap();
    let s = noted.subscription.unwrap();
    assert_eq!(
        (s.season_id.as_deref(), s.season_blocked.as_deref()),
        (None, Some(season.as_str()))
    );
    assert_eq!(env.channels.season_holder(&season).await.unwrap(), Some(29));
    // The link is as the user left it.
    assert_eq!(env.link(2).await, (1, Some(29)));
    // The note stays while the season is held, and the worker's release finds nothing.
    assert!(env
        .channels
        .release_unheld_seasons()
        .await
        .unwrap()
        .is_empty());

    // The user cuts the link: the note goes, and the rule connects next time.
    env.channels
        .set_season_anime(&env.work, 2, 1, None)
        .await
        .unwrap();
    assert_eq!(
        env.channels.release_unheld_seasons().await.unwrap(),
        vec![rule.id.clone()]
    );
    assert_eq!(env.channels.season_holder(&season).await.unwrap(), None);
    assert_eq!(
        env.channels.link_season(&rule.id, &season).await.unwrap(),
        SeasonLinked::Linked
    );
    assert_eq!(env.link(2).await, (3, Some(7)));
}

#[tokio::test]
async fn a_season_linked_to_the_subscriptions_own_anime_is_connected_without_a_new_version() {
    let env = Env::new().await;
    // The user linked the season to the anime before subscribing to it.
    env.channels
        .set_season_anime(&env.work, 2, 0, Some(anime(7, "ON")))
        .await
        .unwrap();
    let rule = env.subscribe("Show", 7).await;

    assert_eq!(
        env.channels
            .link_season(&rule.id, &env.season(2))
            .await
            .unwrap(),
        SeasonLinked::Linked
    );
    assert_eq!(env.link(2).await, (1, Some(7)));
    // From now on the season is the subscription's.
    assert!(matches!(
        env.channels.set_season_anime(&env.work, 2, 1, None).await,
        Err(SeasonAnimeError::Subscribed { .. })
    ));
}

#[tokio::test]
async fn a_subscribed_anime_keeps_the_schedule_snapshot_when_a_search_names_it() {
    let env = Env::new().await;
    env.subscribe("Show", 7).await;
    let mut listed = anime(7, "END");
    listed.subject = "목록의 이름".into();
    listed.fetched_at = 9_999;

    env.channels
        .set_season_anime(&env.work, 1, 0, Some(listed))
        .await
        .unwrap();

    let kept = env.anissia.anime(7).await.unwrap().unwrap();
    assert_eq!(
        (kept.subject.as_str(), kept.status.as_str(), kept.fetched_at),
        ("작품 7", "ON", 1_000)
    );
    assert_eq!(env.link(1).await, (1, Some(7)));
    // The links of the db are all there is to it.
    let rows: i64 = env
        .db
        .run::<_, trss_core::DbError, _>(|c| {
            Ok(c.query_row("SELECT count(*) FROM season_anissia", [], |r| r.get(0))?)
        })
        .await
        .unwrap();
    assert_eq!(rows, 1);
}
