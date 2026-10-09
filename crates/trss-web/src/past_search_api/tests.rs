//! The web's own part of the past episode search: the request checks, the
//! shapes of the context, the poll and the range label, and the end of a
//! search (ticket 0026). What a search judges, pages and refuses is
//! trss-collect's (`past_search`); the receive of a result is `commands_api`'s
//! (`past_tests`). The cases run the real API and search service against the
//! fake tracker.

use axum::http::{Method, StatusCode};
use serde_json::{json, Value};

use crate::testing::past_search::*;
use trss_collect::store::{channels::RuleState, history::HistoryQuery};

#[tokio::test]
async fn the_context_starts_from_the_channels_format_or_else_the_match_phrase() {
    let t = Tracker::with(
        Some("[SubsPlease] {match} 1080p"),
        "Sayonara Lara",
        "Sayonara Lara/Season 01",
        0,
    )
    .await;
    let (status, context) = t.call(Method::GET, &t.search_uri(), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(context["rule_id"], json!(t.rule_id));
    assert_eq!(context["channel_id"], json!(t.channel_id));
    assert_eq!(context["format"], "[SubsPlease] {match} 1080p");
    assert_eq!(context["query"], "[SubsPlease] Sayonara Lara 1080p");
    assert_eq!(context["from_format"], true);
    assert_eq!(
        (&context["offset"], &context["season"]),
        (&json!(0), &json!(1))
    );
    assert_eq!(
        (&context["running"], &context["blocked"]),
        (&Value::Null, &Value::Null)
    );

    // A channel with no format: the match phrase alone.
    let t = Tracker::with(None, "Sayonara Lara", "Sayonara Lara/Season 01", 0).await;
    let (_, context) = t.call(Method::GET, &t.search_uri(), None).await;
    assert_eq!(context["query"], "Sayonara Lara");
    assert_eq!(context["from_format"], false);
    assert_eq!(context["format"], Value::Null);
}

#[tokio::test]
async fn the_words_the_person_edited_in_the_box_are_tidied_and_searched_once() {
    let t = Tracker::with(None, "Sayonara Lara", "Sayonara Lara/Season 01", 0).await;
    t.nyaa.set_releases(&[
        episode("SubsPlease", "Sayonara Lara", 2, ""),
        episode("Erai-raws", "Sayonara Lara", 1, ""),
        episode("SubsPlease", "Sayonara Lara", 1, ""),
    ]);

    // Edited in the box before the one search: the group is added.
    let poll = t.search("[SubsPlease]   Sayonara Lara 1080p", 1, 2).await;

    assert_eq!(poll["state"], "done", "{poll}");
    assert_eq!(t.nyaa.queries(), vec!["[SubsPlease] Sayonara Lara 1080p"]);
    assert_eq!(poll["result"]["query"], "[SubsPlease] Sayonara Lara 1080p");
    assert_eq!(poll["result"]["items"].as_array().unwrap().len(), 2);
    assert_eq!(selected(&poll), vec![1, 2]);
}

#[tokio::test]
async fn a_finished_search_tells_its_results_and_its_runs_as_the_screen_reads_them() {
    let t = Tracker::new().await;
    let titles: Vec<String> = [9, 8, 6, 5]
        .into_iter()
        .map(|n| episode("SubsPlease", "Show", n, ""))
        .collect();
    t.nyaa.set_releases(&titles);

    let poll = t.search("[SubsPlease] Show 1080p", 5, 10).await;

    assert_eq!(poll["state"], "done", "{poll}");
    assert_eq!(poll["rule_id"], json!(t.rule_id));
    assert_eq!(
        (&poll["error"], &poll["sent"], &poll["needed"]),
        (&Value::Null, &json!(0), &json!(0))
    );
    let result = &poll["result"];
    assert_eq!((&result["from"], &result["to"]), (&json!(5), &json!(10)));
    assert_eq!(result["range_label"], "S01E05–10");
    assert_eq!(result["query"], "[SubsPlease] Show 1080p");
    assert_eq!(result["missing"], json!([5, 6, 7, 8, 9, 10]));
    assert_eq!(result["missing_ranges"], json!(["5–10"]));
    assert_eq!(result["not_found"], json!([7, 10]));
    assert_eq!(result["not_found_ranges"], json!(["7", "10"]));
    assert_eq!(result["out_of_range"], 0);
    assert_eq!(result["not_picked"], 0);
    assert_eq!(result["notes"], json!([]));
    assert_eq!(result["first_full"], false);
    assert_eq!(
        (&result["extra_sent"], &result["extra_needed"]),
        (&json!(0), &json!(0))
    );
    // The fields of an item.
    let five = item(&poll, "- 05 ");
    assert_eq!(
        *five,
        json!({
            "key": five["key"],
            "title": episode("SubsPlease", "Show", 5, ""),
            "state": "missing",
            "note": null,
            "release": 5,
            "half": false,
            "version": 1,
            "folder": "S01E05",
            "selected": true,
            "selectable": true,
        })
    );
    assert!(five["key"].as_str().is_some_and(|key| !key.is_empty()));
    // A search leaves no channel and no history behind.
    assert_eq!(
        t.state
            .channels
            .list_channels_with_rules()
            .await
            .unwrap()
            .len(),
        1
    );
    let history = t
        .state
        .history
        .list(HistoryQuery {
            limit: 10,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(history.items.is_empty());
}

#[tokio::test]
async fn the_range_is_labelled_by_the_folder_episodes_of_the_rules_conversion() {
    let t = Tracker::with(
        Some("[SubsPlease] {match} 1080p"),
        "Show",
        "Show/Season 02",
        -12,
    )
    .await;
    let uri = format!("{}/range", t.search_uri());
    let (status, label) = t
        .call(Method::POST, &uri, Some(json!({ "from": 13, "to": 24 })))
        .await;
    assert_eq!(status, StatusCode::OK, "{label}");
    assert_eq!(label, json!({ "from": 13, "to": 24, "label": "S02E01–12" }));
    let (_, one) = t
        .call(Method::POST, &uri, Some(json!({ "from": 14, "to": 14 })))
        .await;
    assert_eq!(one["label"], "S02E02");

    // The finished search tells the same label for its own range.
    t.nyaa
        .set_releases(&[episode("SubsPlease", "Show", 13, "")]);
    let poll = t.search("[SubsPlease] Show 1080p", 13, 24).await;
    assert_eq!(poll["state"], "done", "{poll}");
    assert_eq!(poll["result"]["range_label"], "S02E01–12");

    // A rule whose folder has no season is labelled by its episodes alone.
    let t = Tracker::with(None, "Show", "Show", 0).await;
    let (status, label) = t
        .call(
            Method::POST,
            &format!("{}/range", t.search_uri()),
            Some(json!({ "from": 1, "to": 12 })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{label}");
    assert_eq!(label["label"], "1–12화");
}

#[tokio::test]
async fn the_range_route_refuses_a_range_the_search_would_and_an_unknown_rule_is_not_found() {
    let t = Tracker::new().await;
    let uri = format!("{}/range", t.search_uri());
    for body in [
        json!({ "from": 0, "to": 5 }),
        json!({ "from": 5, "to": 0 }),
        json!({ "from": 6, "to": 5 }),
    ] {
        let (status, answer) = t.call(Method::POST, &uri, Some(body.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {answer}");
        assert_eq!(answer["error"], "invalid");
    }
    // A body that lacks a field, holds a negative number or an unknown field
    // is no request.
    for body in [
        json!({ "from": 1 }),
        json!({ "from": -1, "to": 5 }),
        json!({ "from": 1, "to": 5, "offset": 3 }),
    ] {
        let (status, answer) = t.call(Method::POST, &uri, Some(body.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {answer}");
    }
    let (status, _) = t
        .call(
            Method::POST,
            "/api/rules/nope/past-search/range",
            Some(json!({ "from": 1, "to": 5 })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_request_that_cannot_be_searched_is_refused_before_any_request_is_sent() {
    let t = Tracker::new().await;
    let uri = t.search_uri();
    for body in [
        json!({ "query": "  ", "from": 1, "to": 2 }),
        json!({ "query": "Show", "from": 0, "to": 2 }),
        json!({ "query": "Show", "from": 5, "to": 2 }),
        json!({ "query": "Show", "from": 1, "to": 5000 }),
        json!({ "query": "Show", "from": 1, "to": 2, "extra": 1 }),
    ] {
        let (status, answer) = t.call(Method::POST, &uri, Some(body.clone())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {answer}");
    }
    let (status, answer) = t
        .call(
            Method::POST,
            "/api/rules/nope/past-search",
            Some(json!({ "query": "Show", "from": 1, "to": 2 })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{answer}");

    // A rule that is paused searches nothing, and the context says why.
    t.state
        .channels
        .set_rule_state(&t.rule_id, RuleState::Paused, 1)
        .await
        .unwrap();
    let (status, _) = t.start("Show", 1, 2).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, context) = t.call(Method::GET, &uri, None).await;
    assert!(context["blocked"].as_str().unwrap().contains("멈춰"));
    assert!(t.nyaa.queries().is_empty());
}

#[tokio::test]
async fn a_search_the_tracker_refuses_is_polled_as_failed_with_a_sentence() {
    let t = Tracker::new().await;
    t.nyaa.refuse(Some((429, Some(30))));

    let poll = t.search("[SubsPlease] Show 1080p", 1, 2).await;

    assert_eq!(poll["state"], "failed", "{poll}");
    assert!(
        poll["error"].as_str().is_some_and(|s| !s.is_empty()),
        "{poll}"
    );
    assert_eq!(poll["result"], Value::Null);
    assert_eq!(t.nyaa.queries().len(), 1);
}

#[tokio::test]
async fn a_search_is_forgotten_when_it_is_cancelled_or_a_new_one_of_the_rule_starts() {
    let t = Tracker::new().await;
    t.nyaa.set_releases(&[episode("SubsPlease", "Show", 1, "")]);

    // Leaving the search (the screen's cancel) forgets its results.
    let poll = t.search("[SubsPlease] Show 1080p", 1, 2).await;
    let id = poll["search_id"].as_str().unwrap();
    let uri = format!("/api/past-searches/{id}");
    let (status, _) = t.call(Method::DELETE, &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, gone) = t.call(Method::GET, &uri, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{gone}");
    // Cancelling again, or a search that was never there, is no error.
    let (status, _) = t.call(Method::DELETE, &uri, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // A new search of the rule ends the one before it.
    let first = t.search("[SubsPlease] Show 1080p", 1, 2).await;
    let second = t.search("[SubsPlease] Show 1080p", 1, 2).await;
    assert_ne!(first["search_id"], second["search_id"]);
    let (status, _) = t
        .call(
            Method::GET,
            &format!(
                "/api/past-searches/{}",
                first["search_id"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
