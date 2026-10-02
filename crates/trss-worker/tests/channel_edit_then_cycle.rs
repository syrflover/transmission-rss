//! Editing a channel through the real channels HTTP API, then running a worker
//! cycle against the fake Transmission and the fake feed server: the worker must
//! use what the edit stored.
//!
//! Covers the two completion rows of ticket 0007 that needed the worker:
//! a save with the secret left blank keeps the stored secret for the next
//! cycle, and an exclude added through the API is used by the next cycle.
//! Tokens here are made up.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use trss_legacy::store::history::HistoryResult;
use trss_worker::{CycleReport, TickOutcome, Worker};

async fn run(worker: &Worker) -> CycleReport {
    match worker.tick(&CancellationToken::new()).await.unwrap() {
        TickOutcome::Ran(report) => report,
        other => panic!("expected a cycle, got {other:?}"),
    }
}

/// Adds the feed-a channel the way the app's editor does (`POST /api/channels`,
/// every query value secret), then gives it rules. The API has no rule calls
/// yet, so the rules go straight into the store.
async fn add_channel_via_api(h: &Harness, api: &WebApi) -> Value {
    let url = format!("{}?filter=1080p&token={SECRET}", h.feeds.url("feed-a"));
    let (status, text, view) = api
        .call("POST", "/api/channels", Some(json!({ "url": url })))
        .await;
    assert_eq!(status, StatusCode::CREATED, "{text}");
    assert!(
        !text.contains(SECRET),
        "secret in the create response: {text}"
    );

    // A channel has no save folder of its own: rules save under the app's
    // collect folder.
    assert!(view.get("base_dir").is_none(), "{text}");
    let id = view["id"].as_str().unwrap();
    for rule in feed_a_rules() {
        h.channels.create_rule(id, rule).await.unwrap();
    }
    view
}

/// What the editor sends when saving: everything as the view showed it (secret
/// values blank in `edit_url`), except the fields in `patch`.
async fn save(api: &WebApi, view: &Value, patch: Value) -> Value {
    let flags: serde_json::Map<String, Value> = view["query"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| (q["name"].as_str().unwrap().to_owned(), q["secret"].clone()))
        .collect();
    let mut body = json!({
        "version": view["version"],
        "url": view["edit_url"],
        "excludes": view["excludes"],
        "secret": flags,
        "past_search": view["past_search"],
        "name": view["name"],
    });
    for (key, value) in patch.as_object().unwrap() {
        body[key] = value.clone();
    }
    let uri = format!("/api/channels/{}", view["id"].as_str().unwrap());
    let (status, text, saved) = api.call("PUT", &uri, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(
        !text.contains(SECRET),
        "secret in the save response: {text}"
    );
    saved
}

fn add_dirs(h: &Harness) -> Vec<String> {
    let mut dirs: Vec<String> =
        h.tr.calls_of("torrent-add")
            .iter()
            .map(|c| c.args["download-dir"].as_str().unwrap().to_owned())
            .collect();
    dirs.sort();
    dirs
}

/// Ticket 0007, row 1.
#[tokio::test]
async fn a_save_with_the_secret_left_blank_still_fetches_the_feed_with_the_stored_secret() {
    let h = Harness::new().await;
    let api = h.web_api();
    let created = add_channel_via_api(&h, &api).await;
    assert_eq!(
        created["edit_url"],
        format!("{}?filter=&token=", h.feeds.url("feed-a"))
    );

    // A real edit whose URL still has the secret blank: only the name changes.
    let saved = save(&api, &created, json!({ "name": "Feed A" })).await;
    assert_eq!(saved["name"], "Feed A");
    assert_eq!(
        saved["masked_url"],
        format!("{}?filter=***&token=***", h.feeds.url("feed-a"))
    );

    // The next cycle asks the feed with the original values, neither blank
    // nor the `***` mask.
    let report = run(&h.worker()).await;
    assert_eq!(
        h.feeds.requests(),
        [format!("feed-a?filter=1080p&token={SECRET}")]
    );
    assert_eq!(report.channels_read, 1, "{report:?}");
    assert_eq!(report.add_failed, 0, "{report:?}");
    assert!(report.added > 0, "{report:?}");

    // Every torrent went under the collect folder.
    let dirs = add_dirs(&h);
    assert_eq!(dirs.len(), report.added);
    assert!(dirs.iter().all(|d| d.starts_with("/media/")), "{dirs:?}");

    // The secret is kept in the store and stays out of history.
    let stored = h
        .channels
        .get_channel(created["id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert!(stored.url.contains(&format!("token={SECRET}")));
    let dump = format!("{:?}", h.history_items().await);
    assert!(!dump.contains(SECRET), "secret in history:\n{dump}");
}

/// Ticket 0007, row 2 (the worker half; the rules-tab preview waits for 0006).
#[tokio::test]
async fn an_exclude_added_through_the_api_is_used_by_the_next_cycle() {
    // Control: the same channel and rules without the exclude add the 720p
    // release, so the exclude below is what changes the outcome.
    let control = Harness::new().await;
    let control_api = control.web_api();
    add_channel_via_api(&control, &control_api).await;
    run(&control.worker()).await;
    assert_eq!(
        control.item("Sayonara Lara - 03 (720p)").await.result,
        HistoryResult::Received
    );
    assert_eq!(control.tr.torrents().len(), 5);

    let h = Harness::new().await;
    let api = h.web_api();
    let created = add_channel_via_api(&h, &api).await;
    let saved = save(
        &api,
        &created,
        json!({ "excludes": ["(720p)", " ", "[Batch]"] }),
    )
    .await;
    assert_eq!(saved["excludes"], json!(["(720p)", "[Batch]"]));

    let report = run(&h.worker()).await;
    assert_eq!(
        h.feeds.requests(),
        [format!("feed-a?filter=1080p&token={SECRET}")]
    );
    assert_eq!(
        (report.added, report.excluded, report.no_match),
        (3, 2, 2),
        "{report:?}"
    );

    // The excluded items are recorded as such and were not added.
    for part in ["(720p)", "[Batch]"] {
        let item = h.item(part).await;
        assert_eq!(item.result, HistoryResult::Excluded, "{part}");
        assert_eq!(item.torrent_hash, None, "{part}");
    }
    let added_hashes: Vec<String> =
        h.tr.calls_of("torrent-add")
            .iter()
            .map(|c| c.args["filename"].as_str().unwrap().to_owned())
            .collect();
    assert_eq!(added_hashes.len(), 3);
    for hash in [
        "aaaa000000000000000000000000000000000002",
        "aaaa000000000000000000000000000000000005",
    ] {
        assert!(
            h.tr.torrents().iter().all(|t| t.hash != hash),
            "{hash} was added although it is excluded"
        );
        assert!(
            added_hashes.iter().all(|f| !f.contains(hash)),
            "{hash} was sent to Transmission"
        );
    }
}
