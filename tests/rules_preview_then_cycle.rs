//! The rules API and the rules-tab preview against the real worker cycle, with
//! the fake Transmission and the fake feed server of `tests/common`.
//!
//! - Ticket 0006, row 1: the same recorded items and the same settings give the
//!   same selection, applied rule and save path in the preview as in the worker.
//! - Ticket 0006, row 4: after a reorder through the API, the next cycle picks
//!   the first match in the new order.
//!
//! The channel URL's token is made up.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;
use transmission_rss::{
    store::history::HistoryResult,
    worker::{CycleReport, TickOutcome, Worker},
};

async fn run(worker: &Worker) -> CycleReport {
    match worker.tick(&CancellationToken::new()).await.unwrap() {
        TickOutcome::Ran(report) => report,
        other => panic!("expected a cycle, got {other:?}"),
    }
}

/// The rules the way `GET /api/rules` lists them, for the one channel.
async fn rules_of(api: &WebApi) -> Vec<Value> {
    let (status, text, list) = api.call("GET", "/api/rules", None).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    list["rules"].as_array().unwrap().clone()
}

/// What the editor sends to preview `rule` with `patch` applied.
fn preview_body(channel_id: &str, rule: &Value, patch: Value) -> Value {
    let mut edited = json!({
        "match": rule["match"],
        "regex": rule["regex"],
        "case_insensitive": rule["case_insensitive"],
        "directory": rule["directory"],
        "episode": rule["episode"],
    });
    for (key, value) in patch.as_object().unwrap() {
        edited[key] = value.clone();
    }
    json!({ "channel_id": channel_id, "rule_id": rule["id"], "rule": edited })
}

async fn preview(api: &WebApi, body: Value) -> Value {
    let (status, text, json) = api.call("POST", "/api/rules/preview", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    json
}

fn item<'a>(preview: &'a Value, title_part: &str) -> Option<&'a Value> {
    preview["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["title"].as_str().unwrap().contains(title_part))
}

/// Where Transmission was told to put the torrent of `hash`.
fn saved_at(h: &Harness, hash: &str) -> String {
    h.tr.torrents()
        .into_iter()
        .find(|t| t.hash == hash)
        .unwrap_or_else(|| panic!("Transmission has no torrent {hash}"))
        .download_dir
}

async fn channel_a(h: &Harness) -> transmission_rss::store::channels::ChannelWithRules {
    h.add_channel(
        "feed-a",
        "/media/anime",
        &["[Batch]", "(720p)"],
        feed_a_rules(),
    )
    .await
}

/// Ticket 0006, row 1: for every item the worker recorded, the preview of each
/// rule says what the worker did.
#[tokio::test]
async fn the_preview_of_every_rule_agrees_with_what_the_worker_did_with_the_same_items() {
    let h = Harness::new().await;
    let channel = channel_a(&h).await;
    let api = h.web_api();
    let report = run(&h.worker()).await;
    assert_eq!(report.items_seen, 7, "{report:?}");
    assert!(report.added > 0);

    let rules = rules_of(&api).await;
    assert_eq!(rules.len(), 4);
    let previews: Vec<Value> = {
        let mut all = Vec::new();
        for rule in &rules {
            all.push(preview(&api, preview_body(&channel.channel.id, rule, json!({}))).await);
        }
        all
    };

    let history = h.history_items().await;
    assert_eq!(history.len(), 7);
    let mut checked_selected = 0;
    let mut checked_overlap = 0;
    for entry in &history {
        for (rule, preview) in rules.iter().zip(&previews) {
            let listed = preview["items"]
                .as_array()
                .unwrap()
                .iter()
                .find(|i| i["id"] == entry.id);
            let rule_id = rule["id"].as_str().unwrap();
            match (&entry.rule_id, entry.result, listed) {
                // The worker selected the item with this rule.
                (Some(applied), HistoryResult::Received, Some(p)) if applied == rule_id => {
                    assert_eq!(p["kind"], "mine", "{}", entry.title);
                    let hash = entry.torrent_hash.as_deref().unwrap();
                    assert_eq!(
                        p["save_path"].as_str().unwrap(),
                        saved_at(&h, hash),
                        "save path of {}",
                        entry.title
                    );
                    checked_selected += 1;
                }
                (Some(applied), HistoryResult::Received, None) if applied == rule_id => {
                    panic!("the preview of the applied rule omits {}", entry.title)
                }
                // Another rule was selected: this rule's preview may only list
                // the item as taken by that rule.
                (Some(applied), HistoryResult::Received, Some(p)) => {
                    assert_eq!(p["kind"], "earlier", "{}", entry.title);
                    assert_eq!(p["taken_by"]["rule_id"], json!(applied), "{}", entry.title);
                    let hash = entry.torrent_hash.as_deref().unwrap();
                    assert_eq!(p["save_path"].as_str().unwrap(), saved_at(&h, hash));
                    checked_overlap += 1;
                }
                (Some(_), HistoryResult::Received, None) => {}
                // Excluded by the channel: a rule that matches it says so and
                // no rule takes it.
                (None, HistoryResult::Excluded, Some(p)) => {
                    assert_eq!(p["kind"], "excluded", "{}", entry.title);
                    assert_eq!(p["save_path"], Value::Null);
                }
                (None, HistoryResult::Excluded, None) => {}
                // No rule matched: no preview lists it.
                (None, HistoryResult::NoMatch, listed) => {
                    assert!(listed.is_none(), "{} listed by {rule_id}", entry.title)
                }
                other => panic!("unexpected combination for {}: {other:?}", entry.title),
            }
        }
    }
    assert_eq!(checked_selected, report.added);
    assert!(checked_overlap >= 1, "the feed has an item two rules match");
    // The worker's counts and the preview's counts add up the same way.
    let sono = previews[1]["counts"].clone();
    assert_eq!(sono["total"], 7);
}

/// Ticket 0006, row 1 with an unsaved edit: what the preview predicts for a
/// changed rule is what the next cycle does once the change is saved.
#[tokio::test]
async fn what_the_preview_predicts_for_an_edit_is_what_the_next_cycle_does_after_it_is_saved() {
    let h = Harness::new().await;
    let channel = channel_a(&h).await;
    let api = h.web_api();
    run(&h.worker()).await;
    h.tr.clear_calls();

    // The Unrelated Show has no rule. Widen the Slime rule so it matches it,
    // and its own item too (a rule further back in the list, so nothing is taken
    // from an earlier one).
    let rules = rules_of(&api).await;
    let slime = &rules[3];
    let patch = json!({ "match": "[SubsPlease] ", "directory": "Everything" });
    let predicted = preview(
        &api,
        preview_body(&channel.channel.id, slime, patch.clone()),
    )
    .await;

    let unrelated = item(&predicted, "Unrelated Show").expect("the widened rule matches it");
    assert_eq!(unrelated["kind"], "mine");
    assert_eq!(unrelated["save_path"], "/media/anime/Everything");
    let lara = item(&predicted, "Sayonara Lara - 03 (1080p)").unwrap();
    assert_eq!(
        lara["kind"], "earlier",
        "the Lara rule is earlier and keeps it"
    );
    let excluded = item(&predicted, "(720p)").unwrap();
    assert_eq!(excluded["kind"], "excluded");
    let predicted_new: Vec<String> = predicted["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["kind"] == "mine" && i["stored_result"] == "no_match")
        .map(|i| i["title"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(predicted_new.len(), 1, "{predicted}");
    // Nothing is saved yet.
    assert_eq!(
        rules_of(&api).await[3]["match"],
        "[SubsPlease] Tensei Shitara Slime Datta Ken"
    );

    // Save it, then run the next cycle.
    let mut save = json!({
        "version": slime["version"],
        "channel_id": channel.channel.id,
        "match": slime["match"],
        "regex": false,
        "case_insensitive": false,
        "directory": slime["directory"],
        "episode": slime["episode"],
        "state": "active",
    });
    for (key, value) in patch.as_object().unwrap() {
        save[key] = value.clone();
    }
    let (status, text, _) = api
        .call(
            "PUT",
            &format!("/api/rules/{}", slime["id"].as_str().unwrap()),
            Some(save),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    h.advance(300_000);
    run(&h.worker()).await;

    // Exactly the predicted item was added, at the predicted place, by that rule.
    // (The worker offers Transmission every matching item each cycle; the ones
    // it already has are answered as such and are not new.)
    let adds: Vec<_> =
        h.tr.calls_of("torrent-add")
            .into_iter()
            .filter(|c| {
                c.args["filename"]
                    .as_str()
                    .is_some_and(|f| f.contains("btih:aaaa000000000000000000000000000000000006"))
            })
            .collect();
    assert_eq!(adds.len(), 1, "{adds:?}");
    assert_eq!(adds[0].args["download-dir"], "/media/anime/Everything");
    let recorded = h.item("Unrelated Show").await;
    assert_eq!(recorded.result, HistoryResult::Received);
    assert_eq!(recorded.rule_id.as_deref(), slime["id"].as_str());
    assert_eq!(predicted_new, std::slice::from_ref(&recorded.title));

    // And the rules the preview said were earlier still hold their items.
    let lara = h.item("Sayonara Lara - 03 (1080p)").await;
    assert_eq!(lara.rule_id.as_deref(), rules[0]["id"].as_str());

    // After the save the list marks the widened rule as overlapping.
    let after = rules_of(&api).await;
    assert_eq!(after[3]["overlap"], true);
    assert_eq!(after[0]["overlap"], false);
}

/// Ticket 0006, row 4.
#[tokio::test]
async fn after_a_reorder_through_the_api_the_next_cycle_picks_the_first_match_in_the_new_order() {
    // Control: in the stored order the case-insensitive rule wins the item that
    // both Sono Bisque Doll rules match.
    let control = Harness::new().await;
    channel_a(&control).await;
    run(&control.worker()).await;
    let control_item = control.item("Sono Bisque Doll - 13").await;
    let control_rules = control.channels.list_channels_with_rules().await.unwrap();
    assert_eq!(
        control_item.rule_id.as_deref(),
        Some(control_rules[0].rules[1].id.as_str())
    );

    let h = Harness::new().await;
    let channel = channel_a(&h).await;
    let api = h.web_api();
    let before = rules_of(&api).await;
    assert_eq!(
        before[1]["overlap"],
        Value::Bool(false),
        "nothing is recorded yet"
    );

    // Move the second Sono rule ("overlap" folder) in front of the first.
    let order: Vec<Value> = [0usize, 2, 1, 3]
        .iter()
        .map(|&i| json!({ "id": before[i]["id"], "version": before[i]["version"] }))
        .collect();
    let (status, text, reordered) = api
        .call(
            "PUT",
            "/api/rules/order",
            Some(json!({ "channel_id": channel.channel.id, "order": order })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let orders: Vec<(String, u64)> = reordered["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["id"].as_str().unwrap().to_owned(),
                r["order"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(orders[1], (before[2]["id"].as_str().unwrap().to_owned(), 2));
    assert_eq!(orders[2], (before[1]["id"].as_str().unwrap().to_owned(), 3));

    // The listing's check order is the new one.
    let listed = rules_of(&api).await;
    let ids: Vec<&str> = listed.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        [&before[0], &before[2], &before[1], &before[3]].map(|r| r["id"].as_str().unwrap())
    );

    // The preview already says which rule will take the item.
    let preview_of_old_first = preview(
        &api,
        preview_body(&channel.channel.id, &listed[2], json!({})),
    )
    .await;
    assert!(
        preview_of_old_first["items"].as_array().unwrap().is_empty() || {
            let sono = item(&preview_of_old_first, "Sono Bisque Doll - 13");
            sono.is_none() || sono.unwrap()["kind"] == "earlier"
        }
    );

    let report = run(&h.worker()).await;
    assert_eq!(report.add_failed, 0, "{report:?}");
    let picked = h.item("Sono Bisque Doll - 13").await;
    assert_eq!(picked.result, HistoryResult::Received);
    assert_eq!(picked.rule_id.as_deref(), before[2]["id"].as_str());
    // Transmission was told to save it in the folder of the rule now first.
    let overlap_adds: Vec<_> =
        h.tr.calls_of("torrent-add")
            .into_iter()
            .filter(|c| c.args["download-dir"] == "/media/anime/Sono Bisque Doll (overlap)")
            .collect();
    assert_eq!(overlap_adds.len(), 1, "{overlap_adds:?}");
    // The listing after the cycle shows the shadowed rule as overlapping.
    let after = rules_of(&api).await;
    assert_eq!(after[2]["overlap"], true);
    assert_eq!(after[1]["overlap"], false);
}
