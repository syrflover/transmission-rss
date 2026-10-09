//! `receive_past` as the web accepts it (`check_receive_past`): the title and
//! the link are the search's own, and a result is refused unless a finished
//! search of the rule has it and the rule would receive it. Whether the worker
//! then adds it is trss-collect's (`receive_past`); what the preview offers
//! is its `past_search::world`. The cases run the real API and search service
//! against the fake tracker.

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use crate::testing::past_search::*;
use trss_collect::{
    commands::{receive_once::NotRetryable, receive_past::ReceivePast},
    fake::FakeNyaa,
    store::{
        channels::{RuleInput, RuleState},
        history::{HistoryResult, Observation},
        search_pace::SearchPace,
    },
};

const ID: &str = "0b7d5a44-6c1e-4c62-9a6a-3f0c1d2e4b55";
const HASH: &str = "bbbb000000000000000000000000000000000005";

/// A search of episodes 5 to 8 and its poll.
async fn five_to_eight(t: &Tracker) -> Value {
    let titles: Vec<String> = (5..=8)
        .rev()
        .map(|n| episode("SubsPlease", "Show", n, ""))
        .collect();
    t.nyaa.set_releases(&titles);
    let poll = t.search("[SubsPlease] Show 1080p", 5, 8).await;
    assert_eq!(poll["state"], "done", "{poll}");
    poll
}

fn key_of(poll: &Value, title_part: &str) -> String {
    item(poll, title_part)["key"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn a_result_is_accepted_with_the_title_and_the_link_the_search_holds() {
    let t = Tracker::new().await;
    t.nyaa.set_releases(&[episode("SubsPlease", "Show", 1, "")]);
    let poll = t.search("[SubsPlease] Show 1080p", 1, 2).await;
    let key = key_of(&poll, "- 01 ");

    let (status, accepted) = t.receive(ID, &poll, &key).await;

    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let stored = t.state.commands.get(ID).await.unwrap().unwrap();
    assert_eq!(stored.kind, "receive_past");
    let payload: ReceivePast = serde_json::from_str(&stored.payload).unwrap();
    assert_eq!(payload.rule_id, t.rule_id);
    assert_eq!(payload.key, key);
    assert_eq!(
        payload.title,
        item(&poll, "- 01 ")["title"].as_str().unwrap()
    );
    // The link of the result the tracker listed, which is the torrent's.
    let hash = FakeNyaa::hash_for(&payload.title);
    assert!(payload.link.contains(&hash), "{}", payload.link);
    assert_eq!(stored.subject.as_deref(), Some(payload.subject().as_str()));
    // A search is not a receive: history holds nothing until the worker adds.
    assert!(t
        .state
        .history
        .item_by_key(t.channel_id.clone(), key.clone())
        .await
        .unwrap()
        .is_none());

    // The title and the link are the search's, never the browser's.
    let (status, _) = t
        .call(
            Method::POST,
            "/api/commands",
            Some(json!({
                "id": "0b7d5a44-0000-4000-8000-000000000002",
                "kind": "receive_past",
                "payload": {
                    "rule_id": t.rule_id,
                    "search_id": poll["search_id"],
                    "key": key,
                    "link": "magnet:?xt=urn:btih:ffff",
                },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The accepted request still answers as itself once its search is gone.
    let search = poll["search_id"].as_str().unwrap();
    let (status, _) = t
        .call(
            Method::DELETE,
            &format!("/api/past-searches/{search}"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, again) = t.receive(ID, &poll, &key).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    // A request with no search behind it stores nothing.
    let (status, gone) = t
        .receive("0b7d5a44-0000-4000-8000-000000000003", &poll, &key)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{gone}");
    assert_eq!(gone["error"], "not_found");
    let missing = t
        .state
        .commands
        .get("0b7d5a44-0000-4000-8000-000000000003")
        .await
        .unwrap();
    assert!(missing.is_none());
}

#[tokio::test]
async fn a_result_the_search_does_not_hold_or_of_another_rule_or_not_yet_finished_is_refused() {
    let t = Tracker::new().await;
    let poll = five_to_eight(&t).await;
    let key = key_of(&poll, "- 05 ");
    let other = t
        .state
        .channels
        .create_rule(
            &t.channel_id,
            RuleInput {
                r#match: Some("Show".into()),
                directory: "Show/Season 01".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    // (the rule, the search, the key) -> the sentence
    let cases = [
        (
            &t.rule_id,
            poll["search_id"].clone(),
            "guid:nope",
            "이 검색 결과에 없는 항목이에요.",
        ),
        (
            &other.id,
            poll["search_id"].clone(),
            key.as_str(),
            "이 검색은 다른 규칙의 검색이에요.",
        ),
    ];
    for (n, (rule, search, key, sentence)) in cases.into_iter().enumerate() {
        let id = format!("0b7d5a44-0000-4000-8000-00000000010{n}");
        let (status, refused) = t.receive_of(&id, rule, &search, key).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{sentence}: {refused}");
        assert_eq!(refused["error"], "invalid");
        assert_eq!(refused["message"], sentence);
        assert!(t.state.commands.get(&id).await.unwrap().is_none());
    }

    // A search that has not finished: the host asked for no request for a
    // while, so the search waits for its turn.
    let blocked_until = super::now_millis() + 30_000;
    SearchPace::new(t.db.clone())
        .block("127.0.0.1", blocked_until)
        .await
        .unwrap();
    let (status, started) = t.start("[SubsPlease] Show 1080p", 5, 8).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{started}");
    let running = started["search_id"].clone();
    let (_, poll) = t
        .call(
            Method::GET,
            &format!("/api/past-searches/{}", running.as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(poll["state"], "running", "{poll}");
    let (status, refused) = t.receive_of(ID, &t.rule_id, &running, &key).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(refused["message"], "검색이 아직 끝나지 않았어요.");
    assert!(t.state.commands.get(ID).await.unwrap().is_none());
    let (status, _) = t
        .call(
            Method::DELETE,
            &format!("/api/past-searches/{}", running.as_str().unwrap()),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// What changed after the worker received episode 5 and before the person
/// chose it again.
#[derive(Debug, Clone, Copy)]
enum Since {
    /// Its torrent is in Transmission still.
    TheTorrentIsThere,
    /// Its torrent was removed and its video is in the folder.
    TheVideoIsInTheFolder,
    /// Its torrent was removed and the work folder is not there (a volume
    /// that is not mounted).
    TheFolderIsNotThere,
    /// Its torrent was removed and its video never reached the folder.
    NothingIsLeft,
    /// [`Since::NothingIsLeft`], and the rule was paused after the search.
    NothingIsLeftAndTheRuleIsPaused,
}

#[tokio::test]
async fn a_received_episode_is_chosen_again_only_when_it_is_gone_and_its_rule_would_receive_it() {
    let cases = [
        (Since::TheTorrentIsThere, Some(NotRetryable::Held)),
        (Since::TheVideoIsInTheFolder, Some(NotRetryable::Held)),
        (Since::TheFolderIsNotThere, Some(NotRetryable::Held)),
        (Since::NothingIsLeft, None),
        (
            Since::NothingIsLeftAndTheRuleIsPaused,
            Some(NotRetryable::RulePaused),
        ),
    ];
    for (since, refused) in cases {
        let t = Tracker::new().await;
        let poll = five_to_eight(&t).await;
        let key = key_of(&poll, "- 05 ");
        // The worker received episode 5: history says so.
        let title = item(&poll, "- 05 ")["title"].as_str().unwrap().to_owned();
        t.state
            .history
            .record(
                1,
                vec![Observation {
                    channel_id: t.channel_id.clone(),
                    channel_label: "nyaa".into(),
                    identity_key: key.clone(),
                    title,
                    link: format!("magnet:?xt=urn:btih:{HASH}"),
                    result: HistoryResult::Received,
                    rule_id: Some(t.rule_id.clone()),
                    torrent_hash: Some(HASH.into()),
                    reason: None,
                }],
            )
            .await
            .unwrap();
        // The worker's next look at Transmission.
        let listed = match since {
            Since::TheTorrentIsThere => vec![HASH.to_owned()],
            _ => Vec::new(),
        };
        t.state.status.record_listing(1_000, listed).await.unwrap();
        match since {
            Since::TheVideoIsInTheFolder => t.write("Show S01E05.mkv", b"x"),
            Since::TheFolderIsNotThere => std::fs::remove_dir_all(&t.folder).unwrap(),
            _ => {}
        }

        // What the screen offers.
        let poll = t.search("[SubsPlease] Show 1080p", 5, 8).await;
        let offered = item(&poll, "- 05 ");
        // The preview refuses what is held; a rule paused after the search
        // is the web's check at the time of the request.
        let held = matches!(refused, Some(NotRetryable::Held));
        assert_eq!(offered["selectable"], !held, "{since:?}: {offered}");
        if matches!(since, Since::NothingIsLeftAndTheRuleIsPaused) {
            t.state
                .channels
                .set_rule_state(&t.rule_id, RuleState::Paused, 2)
                .await
                .unwrap();
        }

        // What the web takes.
        let (status, answer) = t.receive(ID, &poll, &key).await;
        match refused {
            Some(reason) => {
                assert_eq!(status, StatusCode::BAD_REQUEST, "{since:?}: {answer}");
                assert_eq!(answer["message"], reason.message(), "{since:?}");
                assert!(
                    t.state.commands.get(ID).await.unwrap().is_none(),
                    "{since:?}"
                );
            }
            None => {
                assert_eq!(status, StatusCode::ACCEPTED, "{since:?}: {answer}");
                assert!(
                    t.state.commands.get(ID).await.unwrap().is_some(),
                    "{since:?}"
                );
            }
        }
    }
}
