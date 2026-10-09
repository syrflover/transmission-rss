use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::*;
use trss_collect::store::channels::{RuleInput, RuleState};
use trss_core::Db;

/// Raw secret values used across the tests. No response body may contain them.
const TOKEN: &str = "s3cr3t-tok";
const OTHER: &str = "0th3r-sig";

struct App {
    state: AppState,
    router: Router,
}

impl App {
    fn new() -> App {
        let state = AppState::new(Db::open_blocking(":memory:").unwrap());
        let router = Router::new().nest("/api", crate::api::router().with_state(state.clone()));
        App { state, router }
    }

    /// Sends a request; returns status, the raw body text and its JSON.
    async fn call(
        &self,
        method: Method,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, String, Value) {
        let mut request = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(json) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let response = self
            .router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let json = serde_json::from_str(&text).unwrap_or(Value::Null);
        (status, text, json)
    }

    async fn create(&self, url: &str) -> Value {
        let (status, text, json) = self
            .call(Method::POST, "/api/channels", Some(json!({ "url": url })))
            .await;
        assert_eq!(status, StatusCode::CREATED, "{text}");
        json
    }

    /// The channel as stored, secret values included.
    async fn stored(&self, id: &str) -> Channel {
        self.state.channels.get_channel(id).await.unwrap().unwrap()
    }

    async fn put(&self, channel: &Value, patch: Value) -> (StatusCode, String, Value) {
        let mut body = json!({
            "version": channel["version"],
            "url": channel["edit_url"],
            "excludes": channel["excludes"],
            "secret": secret_flags(channel),
            "past_search": channel["past_search"],
            "name": channel["name"],
        });
        for (key, value) in patch.as_object().unwrap() {
            body[key] = value.clone();
        }
        let uri = format!("/api/channels/{}", channel["id"].as_str().unwrap());
        self.call(Method::PUT, &uri, Some(body)).await
    }
}

/// What a client that only saw a view sends back for the flags.
fn secret_flags(channel: &Value) -> Value {
    let mut flags = serde_json::Map::new();
    for q in channel["query"].as_array().unwrap() {
        flags.insert(q["name"].as_str().unwrap().to_owned(), q["secret"].clone());
    }
    Value::Object(flags)
}

fn assert_no_secret(text: &str) {
    for secret in [TOKEN, OTHER] {
        assert!(
            !text.contains(secret),
            "secret {secret:?} leaked in: {text}"
        );
    }
}

fn names(channel: &Value) -> Vec<(String, bool)> {
    channel["query"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| {
            (
                q["name"].as_str().unwrap().to_owned(),
                q["secret"].as_bool().unwrap(),
            )
        })
        .collect()
}

// --- add and read -----------------------------------------------------------

#[tokio::test]
async fn the_name_is_optional_trimmed_and_returned() {
    let app = App::new();
    let url = format!("https://feed.example/rss?token={TOKEN}");

    // Absent and blank both mean unnamed; the host stays available.
    let unnamed = app.create(&url).await;
    assert_eq!(unnamed["name"], Value::Null);
    assert_eq!(unnamed["host"], "feed.example");
    let (status, _, blank) = app
        .call(
            Method::POST,
            "/api/channels",
            Some(json!({ "url": url, "name": "   " })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(blank["name"], Value::Null);

    let (status, text, named) = app
        .call(
            Method::POST,
            "/api/channels",
            Some(json!({ "url": url, "name": "  주간 애니 " })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_no_secret(&text);
    assert_eq!(named["name"], "주간 애니");
    let id = named["id"].as_str().unwrap();
    assert_eq!(app.stored(id).await.name.as_deref(), Some("주간 애니"));

    // An edit that keeps the name and leaves the secret blank changes neither
    // the name nor the stored secret.
    let (status, text, kept) = app
        .put(&named, json!({ "past_search": "[Sub] {match}" }))
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(kept["name"], "주간 애니");
    assert_eq!(app.stored(id).await.url, url);

    // A blank name clears it; a new value replaces it.
    let (_, _, cleared) = app.put(&kept, json!({ "name": "" })).await;
    assert_eq!(cleared["name"], Value::Null);
    assert_eq!(app.stored(id).await.name, None);
    let (_, _, renamed) = app.put(&cleared, json!({ "name": " Feed B " })).await;
    assert_eq!(renamed["name"], "Feed B");

    // The list carries the name too.
    let (_, _, list) = app.call(Method::GET, "/api/channels", None).await;
    let listed: Vec<_> = list["channels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].clone())
        .collect();
    assert_eq!(listed, [Value::Null, Value::Null, json!("Feed B")]);
}

/// The editor of a channel not yet added has no channel to read the limit
/// from, so the list carries it.
#[tokio::test]
async fn the_list_tells_how_long_a_name_may_be_even_with_no_channel() {
    let app = App::new();
    let (_, _, list) = app.call(Method::GET, "/api/channels", None).await;
    assert_eq!(list["channels"], json!([]));
    assert_eq!(list["name_max_chars"], 100);
    // That many characters are taken and one more is not.
    let url = format!("https://feed.example/rss?token={TOKEN}");
    for (length, status) in [(100, StatusCode::CREATED), (101, StatusCode::BAD_REQUEST)] {
        let (got, text, _) = app
            .call(
                Method::POST,
                "/api/channels",
                Some(json!({ "url": url, "name": "가".repeat(length) })),
            )
            .await;
        assert_eq!(got, status, "{length}: {text}");
    }
}

#[tokio::test]
async fn an_unusable_name_is_refused_without_echoing_the_request() {
    let app = App::new();
    let url = format!("https://feed.example/rss?token={TOKEN}");
    for name in ["two\nlines".to_owned(), "가".repeat(101)] {
        let (status, text, _) = app
            .call(
                Method::POST,
                "/api/channels",
                Some(json!({ "url": url, "name": name })),
            )
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
        assert_no_secret(&text);
        assert!(!text.contains("two"), "{text}");
    }
    let (_, _, list) = app.call(Method::GET, "/api/channels", None).await;
    assert_eq!(list["channels"], json!([]));
}

#[tokio::test]
async fn a_new_channel_masks_every_query_value_in_every_response() {
    let app = App::new();
    let url = format!("https://feed.example/rss?r=1080&token={TOKEN}");
    let (status, text, created) = app
        .call(
            Method::POST,
            "/api/channels",
            Some(json!({ "url": url, "excludes": ["Batch", " "], "past_search": "[Sub] {match} 1080p" })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_no_secret(&text);
    assert!(!text.contains("1080\""), "r's value is secret too: {text}");

    assert_eq!(
        created["masked_url"],
        "https://feed.example/rss?r=***&token=***"
    );
    assert_eq!(created["host"], "feed.example");
    assert_eq!(
        names(&created),
        [("r".into(), true), ("token".into(), true)]
    );
    assert_eq!(created["query"][1]["filled"], true);
    assert_eq!(created["edit_url"], "https://feed.example/rss?r=&token=");
    assert_eq!(
        created["excludes"],
        json!(["Batch"]),
        "blank excludes are dropped"
    );
    assert_eq!(created["past_search"], "[Sub] {match} 1080p");
    assert_eq!(created["rule_count"], 0);

    // The list and the detail carry masked values only, too.
    let id = created["id"].as_str().unwrap();
    let (status, list_text, list) = app.call(Method::GET, "/api/channels", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_no_secret(&list_text);
    assert_eq!(list["channels"][0]["masked_url"], created["masked_url"]);
    let (status, detail_text, detail) = app
        .call(Method::GET, &format!("/api/channels/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_no_secret(&detail_text);
    assert_eq!(detail, created);

    // The original value is what the store keeps.
    assert_eq!(app.stored(id).await.url, url);
}

#[tokio::test]
async fn a_secret_not_entered_yet_is_reported_as_not_filled() {
    let app = App::new();
    let created = app.create("https://feed.example/rss?token=").await;
    assert_eq!(created["masked_url"], "https://feed.example/rss?token=");
    assert_eq!(
        created["query"][0],
        json!({ "name": "token", "secret": true, "filled": false })
    );
}

#[tokio::test]
async fn create_can_start_a_name_as_not_secret() {
    let app = App::new();
    let (status, text, created) = app
        .call(
            Method::POST,
            "/api/channels",
            Some(json!({
                "url": format!("https://feed.example/rss?r=1080&token={TOKEN}"),
                "secret": { "r": false },
            })),
        )
        .await;
    assert_eq!(status, StatusCode::CREATED, "{text}");
    assert_no_secret(&text);
    assert_eq!(
        created["masked_url"],
        "https://feed.example/rss?r=1080&token=***"
    );
    assert_eq!(
        names(&created),
        [("r".into(), false), ("token".into(), true)]
    );
}

// --- ticket rows -------------------------------------------------------------

#[tokio::test]
async fn unsetting_secret_on_one_name_shows_it_and_keeps_the_other_masked() {
    let app = App::new();
    let created = app
        .create(&format!("https://feed.example/rss?r=1080&token={TOKEN}"))
        .await;

    // The client never saw r=1080: it sends the blanks from edit_url and
    // clears r's flag.
    let (status, text, updated) = app
        .put(&created, json!({ "secret": { "r": false, "token": true } }))
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_no_secret(&text);
    assert_eq!(
        updated["masked_url"],
        "https://feed.example/rss?r=1080&token=***"
    );
    assert_eq!(
        names(&updated),
        [("r".into(), false), ("token".into(), true)]
    );
    assert_eq!(updated["version"], created["version"].as_i64().unwrap() + 1);

    let stored = app.stored(created["id"].as_str().unwrap()).await;
    assert_eq!(
        stored.url,
        format!("https://feed.example/rss?r=1080&token={TOKEN}")
    );
    assert_eq!(stored.secret_query, ["token"]);
}

#[tokio::test]
async fn blank_secret_with_another_url_part_changed_keeps_the_stored_secret() {
    let app = App::new();
    let created = app
        .create(&format!(
            "https://feed.example/rss?r=1080&token={TOKEN}&page=1"
        ))
        .await;
    let id = created["id"].as_str().unwrap().to_owned();

    // r is made public; the host, the path and page change; the token stays
    // blank because the client never saw it.
    let (status, text, updated) = app
        .put(
            &created,
            json!({
                "url": "https://mirror.example/feed.xml?r=720&token=&page=2",
                "secret": { "r": false, "token": true, "page": false },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_no_secret(&text);
    assert_eq!(updated["host"], "mirror.example");
    assert_eq!(
        updated["masked_url"],
        "https://mirror.example/feed.xml?r=720&token=***&page=2"
    );
    assert_eq!(
        updated["query"][1],
        json!({ "name": "token", "secret": true, "filled": true })
    );

    // What the worker reads next: the new URL with the original secret.
    let all = app.state.channels.list_channels_with_rules().await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].channel.id, id);
    assert_eq!(
        all[0].channel.url,
        format!("https://mirror.example/feed.xml?r=720&token={TOKEN}&page=2")
    );
    assert_eq!(all[0].channel.secret_query, ["token"]);
}

#[tokio::test]
async fn adding_excludes_is_saved_with_the_original_secret() {
    let app = App::new();
    let created = app
        .create(&format!("https://feed.example/rss?token={TOKEN}"))
        .await;

    let (status, text, updated) = app
        .put(&created, json!({ "excludes": ["[Batch]", "  480p  ", ""] }))
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_no_secret(&text);
    assert_eq!(updated["excludes"], json!(["[Batch]", "480p"]));

    let all = app.state.channels.list_channels_with_rules().await.unwrap();
    assert_eq!(all[0].channel.excludes, ["[Batch]", "480p"]);
    assert_eq!(
        all[0].channel.url,
        format!("https://feed.example/rss?token={TOKEN}")
    );
}

#[tokio::test]
async fn a_non_blank_secret_value_replaces_the_stored_one() {
    let app = App::new();
    let created = app
        .create(&format!("https://feed.example/rss?token={TOKEN}"))
        .await;
    let (status, text, updated) = app
        .put(
            &created,
            json!({ "url": format!("https://feed.example/rss?token={OTHER}") }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_no_secret(&text);
    assert_eq!(updated["masked_url"], "https://feed.example/rss?token=***");
    assert_eq!(
        app.stored(created["id"].as_str().unwrap()).await.url,
        format!("https://feed.example/rss?token={OTHER}")
    );
}

#[tokio::test]
async fn echoing_the_masked_url_back_keeps_the_stored_secret() {
    let app = App::new();
    let created = app
        .create(&format!("https://feed.example/rss?token={TOKEN}"))
        .await;
    let (status, _, _) = app
        .put(&created, json!({ "url": created["masked_url"] }))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        app.stored(created["id"].as_str().unwrap()).await.url,
        format!("https://feed.example/rss?token={TOKEN}"),
        "the mask must never become the stored secret"
    );
}

#[tokio::test]
async fn a_secret_not_entered_yet_can_be_filled_in_later() {
    let app = App::new();
    let created = app.create("https://feed.example/rss?token=").await;
    let (status, text, updated) = app
        .put(
            &created,
            json!({ "url": format!("https://feed.example/rss?token={TOKEN}") }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_no_secret(&text);
    assert_eq!(updated["query"][0]["filled"], true);
}

// --- flags follow the URL's query names --------------------------------------

#[tokio::test]
async fn new_names_start_secret_and_removed_names_drop() {
    let app = App::new();
    let created = app
        .create(&format!("https://feed.example/rss?r=1080&token={TOKEN}"))
        .await;
    let id = created["id"].as_str().unwrap();

    // r is unset first; then token leaves the URL and sig arrives with a
    // value (the client sends no flag for it).
    let (_, _, step1) = app
        .put(&created, json!({ "secret": { "r": false, "token": true } }))
        .await;
    let (status, text, step2) = app
        .put(
            &step1,
            json!({
                "url": format!("https://feed.example/rss?r=&sig={OTHER}"),
                "secret": { "r": false, "token": true },
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_no_secret(&text);
    assert_eq!(names(&step2), [("r".into(), false), ("sig".into(), true)]);

    let stored = app.stored(id).await;
    assert_eq!(
        stored.url,
        format!("https://feed.example/rss?r=&sig={OTHER}")
    );
    assert_eq!(
        stored.secret_query,
        ["sig"],
        "token dropped, sig started secret"
    );
}

#[tokio::test]
async fn a_name_the_client_does_not_mention_is_secret() {
    let app = App::new();
    let created = app.create("https://feed.example/rss?r=1080").await;
    let (status, _, updated) = app
        .put(
            &created,
            json!({ "url": "https://feed.example/rss?r=1080&x=9", "secret": {} }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(names(&updated), [("r".into(), true), ("x".into(), true)]);
}

#[tokio::test]
async fn a_public_value_that_was_shown_can_be_blanked_by_the_client() {
    let app = App::new();
    let created = app.create("https://feed.example/rss?r=1080&t=x").await;
    let (_, _, step1) = app
        .put(&created, json!({ "url": "https://feed.example/rss?r=1080&t=x", "secret": { "r": false, "t": true } }))
        .await;
    // r is public, so its stored value is not restored when blanked.
    let (status, _, step2) = app
        .put(&step1, json!({ "url": "https://feed.example/rss?r=&t=x", "secret": { "r": false, "t": true } }))
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(step2["masked_url"], "https://feed.example/rss?r=&t=***");
}

// --- conflicts ---------------------------------------------------------------

#[tokio::test]
async fn a_stale_save_conflicts_and_shows_the_current_masked_value() {
    let app = App::new();
    let created = app
        .create(&format!("https://feed.example/rss?token={TOKEN}"))
        .await;
    // Another screen saves first.
    let (status, _, _) = app.put(&created, json!({ "name": "Other" })).await;
    assert_eq!(status, StatusCode::OK);

    let (status, text, json) = app
        .put(
            &created,
            json!({ "name": "Mine", "url": format!("https://feed.example/rss?token={OTHER}") }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{text}");
    assert_no_secret(&text);
    assert_eq!(json["error"], "conflict");
    assert_eq!(json["current"]["name"], "Other");
    assert_eq!(
        json["current"]["version"],
        created["version"].as_i64().unwrap() + 1
    );
    assert_eq!(
        json["current"]["masked_url"],
        "https://feed.example/rss?token=***"
    );

    let stored = app.stored(created["id"].as_str().unwrap()).await;
    assert_eq!(
        stored.name.as_deref(),
        Some("Other"),
        "the stale save changed nothing"
    );
    assert_eq!(
        stored.url,
        format!("https://feed.example/rss?token={TOKEN}")
    );

    // Sending the input again with the current version goes through.
    let (status, _, _) = app.put(&json["current"], json!({ "name": "Mine" })).await;
    assert_eq!(status, StatusCode::OK);
}

// --- delete ------------------------------------------------------------------

async fn channel_with_rules(app: &App, n: usize) -> Value {
    let created = app
        .create(&format!("https://feed.example/rss?token={TOKEN}"))
        .await;
    let id = created["id"].as_str().unwrap();
    for i in 0..n {
        app.state
            .channels
            .create_rule(
                id,
                RuleInput {
                    r#match: Some(format!("show {i}")),
                    state: if i == 0 {
                        RuleState::Archived
                    } else {
                        RuleState::Active
                    },
                    ..RuleInput::default()
                },
            )
            .await
            .unwrap();
    }
    let (_, _, json) = app
        .call(Method::GET, &format!("/api/channels/{id}"), None)
        .await;
    assert_eq!(json["rule_count"], n);
    json
}

#[tokio::test]
async fn deleting_a_channel_with_three_rules_removes_both() {
    let app = App::new();
    let channel = channel_with_rules(&app, 3).await;
    let other = app.create("https://other.example/rss").await;
    let id = channel["id"].as_str().unwrap();

    let uri = format!("/api/channels/{id}?version={}&rules=3", channel["version"]);
    let (status, text, json) = app.call(Method::DELETE, &uri, None).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(json, json!({ "removed_rules": 3 }));

    let left = app.state.channels.list_channels_with_rules().await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].channel.id, other["id"].as_str().unwrap());
    let (status, _, _) = app
        .call(Method::GET, &format!("/api/channels/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_refuses_a_stale_version_or_a_changed_rule_count() {
    let app = App::new();
    let channel = channel_with_rules(&app, 3).await;
    let id = channel["id"].as_str().unwrap();
    let version = channel["version"].as_i64().unwrap();

    // The confirmation said 2 rules but there are 3.
    let (status, text, json) = app
        .call(
            Method::DELETE,
            &format!("/api/channels/{id}?version={version}&rules=2"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_no_secret(&text);
    assert_eq!(
        json["current"]["rule_count"], 3,
        "the screen can re-confirm with this"
    );

    // A save happened after the screen read the channel.
    app.put(&channel, json!({ "name": "X" })).await;
    let (status, _, _) = app
        .call(
            Method::DELETE,
            &format!("/api/channels/{id}?version={version}&rules=3"),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT);

    let left = app.state.channels.list_channels_with_rules().await.unwrap();
    assert_eq!((left.len(), left[0].rules.len()), (1, 3));
}

#[tokio::test]
async fn delete_needs_both_numbers_and_a_real_channel() {
    let app = App::new();
    let channel = channel_with_rules(&app, 1).await;
    let id = channel["id"].as_str().unwrap();

    let (status, _, json) = app
        .call(Method::DELETE, &format!("/api/channels/{id}"), None)
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["error"], "invalid");

    let (status, _, json) = app
        .call(Method::DELETE, "/api/channels/nope?version=1&rules=0", None)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["error"], "not_found");
}

// --- bad input never echoes the secret ---------------------------------------

#[tokio::test]
async fn invalid_input_is_a_korean_400_that_does_not_echo_the_request() {
    let app = App::new();
    let cases = [
        json!({ "url": format!("ftp://feed.example/rss?token={TOKEN}") }),
        json!({ "url": format!("not a url {TOKEN}") }),
        // A channel has no save folder of its own any more; the collect folder
        // is the app's. A body that still names one is not accepted.
        json!({ "url": format!("https://feed.example/rss?token={TOKEN}"), "base_dir": "/m" }),
        json!({ "url": format!("https://feed.example/rss?token={TOKEN}"), "nope": TOKEN }),
        json!({ "url": 42, "name": TOKEN }),
        json!({ "name": TOKEN }),
        // serde echoes the offending value for type errors, so these would
        // leak if a rejection's text ever reached the response.
        json!({ "url": format!("https://feed.example/rss?token={TOKEN}"), "excludes": TOKEN }),
        json!({ "url": format!("https://feed.example/rss?token={TOKEN}"), "secret": { "token": TOKEN } }),
        json!({ "url": format!("https://feed.example/rss?token={TOKEN}"), TOKEN: 1 }),
        // `Url::parse` drops a newline that the masking would still see, so
        // a URL like this could name the query differently for the two.
        json!({ "url": format!("https://feed.example/rss?to\nken={TOKEN}") }),
    ];
    for body in cases {
        let (status, text, json) = app
            .call(Method::POST, "/api/channels", Some(body.clone()))
            .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {text}");
        assert_eq!(json["error"], "invalid");
        assert!(json["message"]
            .as_str()
            .unwrap()
            .chars()
            .any(|c| ('가'..='힣').contains(&c)));
        assert_no_secret(&text);
    }
    let (status, text, _) = app.call(Method::GET, "/api/channels", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        text.contains("\"channels\":[]"),
        "nothing was created: {text}"
    );

    // Broken JSON is rejected in the same shape.
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/channels")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!("{{\"url\": \"{TOKEN}")))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let text = String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert_no_secret(&text);
    assert!(serde_json::from_str::<Value>(&text).unwrap()["error"] == "invalid");
}

#[tokio::test]
async fn a_bad_version_on_save_does_not_echo_the_request() {
    let app = App::new();
    let created = app
        .create(&format!("https://feed.example/rss?token={TOKEN}"))
        .await;
    let id = created["id"].as_str().unwrap();
    let (status, text, json) = app
        .call(
            Method::PUT,
            &format!("/api/channels/{id}"),
            Some(json!({
                "url": created["edit_url"],
                "version": TOKEN,
            })),
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert_eq!(json["error"], "invalid");
    assert_no_secret(&text);
}

#[tokio::test]
async fn unknown_channel_is_a_404_for_read_and_save() {
    let app = App::new();
    let (status, _, json) = app.call(Method::GET, "/api/channels/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json["error"], "not_found");
    let (status, _, _) = app
        .call(
            Method::PUT,
            "/api/channels/nope",
            Some(json!({ "version": 1, "url": "https://a.example/" })),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// --- the merge helpers --------------------------------------------------------

fn stored_channel(url: &str, secret: &[&str]) -> Channel {
    Channel {
        id: "c".into(),
        position: 0,
        version: 1,
        url: url.into(),
        excludes: vec![],
        secret_query: secret.iter().map(|s| s.to_string()).collect(),
        past_search: None,
        name: None,
    }
}

#[test]
fn restore_matches_repeated_names_by_position_and_keeps_the_rest_verbatim() {
    let stored = stored_channel("https://a.example/p?t=one&x=1&t=two#frag", &["t"]);
    assert_eq!(
        restore_secrets("https://b.example/p?t=&x=2&t=#frag", &stored),
        "https://b.example/p?t=one&x=2&t=two#frag"
    );
    // A third blank has no stored counterpart and stays blank.
    assert_eq!(
        restore_secrets("https://a.example/p?t=&t=&t=", &stored),
        "https://a.example/p?t=one&t=two&t="
    );
    // A value that is given wins; a name that is not secret is not restored.
    assert_eq!(
        restore_secrets("https://a.example/p?t=new&x=", &stored),
        "https://a.example/p?t=new&x="
    );
}

#[test]
fn restore_matches_names_after_percent_decoding_and_leaves_bare_names_alone() {
    let stored = stored_channel("https://a.example/p?a%20b=secret&flag&k=v", &["a b"]);
    assert_eq!(
        restore_secrets("https://a.example/p?a+b=&flag&k=v", &stored),
        "https://a.example/p?a+b=secret&flag&k=v"
    );
}

#[test]
fn restore_without_a_stored_secret_changes_nothing() {
    let stored = stored_channel("https://a.example/p?r=1", &[]);
    assert_eq!(
        restore_secrets("https://a.example/p?r=", &stored),
        "https://a.example/p?r="
    );
    let stored = stored_channel("https://a.example/p", &["r"]);
    assert_eq!(
        restore_secrets("https://a.example/p?r=***", &stored),
        "https://a.example/p?r="
    );
}

#[test]
fn edit_url_blanks_exactly_the_secret_values() {
    let mut c = stored_channel("https://a.example/p?r=1080&token=abc&r=2#f", &["token"]);
    assert_eq!(
        view(&c, 0).edit_url,
        "https://a.example/p?r=1080&token=&r=2#f"
    );
    c.secret_query = vec!["r".into(), "token".into()];
    assert_eq!(view(&c, 0).edit_url, "https://a.example/p?r=&token=&r=#f");
    assert_eq!(
        view(&c, 0).masked_url,
        "https://a.example/p?r=***&token=***&r=***#f"
    );
}
