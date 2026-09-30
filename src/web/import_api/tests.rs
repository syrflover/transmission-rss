use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

use crate::store::channels::{ChannelInput, ChannelStore, RuleInput};
use crate::store::Db;

use super::*;

const TOKEN_A: &str = "sekret-token-A-123";
const TOKEN_B: &str = "sekret-token-B-456";

struct App {
    _dir: TempDir,
    path: std::path::PathBuf,
    store: ChannelStore,
    app: Router,
}

async fn app() -> App {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    let db = Db::open(&path).await.unwrap();
    let state = AppState::new(db);
    App {
        _dir: dir,
        path,
        store: state.channels.clone(),
        app: Router::new().nest("/api", crate::web::api::router().with_state(state)),
    }
}

impl App {
    async fn post(&self, uri: &str, body: Value) -> (StatusCode, String, Value) {
        self.post_raw(uri, body.to_string()).await
    }

    async fn post_raw(&self, uri: &str, body: String) -> (StatusCode, String, Value) {
        let request = Request::builder()
            .method(Method::POST)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        let json = serde_json::from_str(&text).unwrap_or(Value::Null);
        (status, text, json)
    }

    async fn preview(&self, content: &str) -> (StatusCode, String, Value) {
        self.post("/api/import/legacy/preview", json!({ "content": content }))
            .await
    }

    async fn apply(&self, content: &str, choices: Value) -> (StatusCode, String, Value) {
        self.post(
            "/api/import/legacy/apply",
            json!({ "content": content, "choices": choices }),
        )
        .await
    }

    async fn all(&self) -> Vec<ChannelWithRules> {
        self.store.list_channels_with_rules().await.unwrap()
    }
}

/// Two channels, seven rules; values chosen to expose lossy handling.
fn file() -> String {
    format!(
        r#"# comment
- url: https://feeds.example.test/a?filter=1080p&token={TOKEN_A}
  directory: /media/a
  excludes: ["[Batch]", "(720p)"]
  rules:
    - match: Keep1
      directory: A/keep1
    - match: '^Regex \d+$'
      regex: true
      case_insensitive: true
      episode: -24
      directory: A/regex
    - match: Fresh
      directory: A/fresh
      episode: 13
- url: https://feeds.example.test/b?token={TOKEN_B}
  directory: /media/b
  rules:
    - match: B1
      directory: B/one
    - match: ''
      directory: B/waiting
    - match: B3
      directory: B/three
      episode: 0
    - match: B4
      directory: B/four
"#
    )
}

fn existing_a() -> (ChannelInput, Vec<RuleInput>) {
    let rule = |m: &str, d: &str| RuleInput {
        r#match: Some(m.into()),
        directory: d.into(),
        ..RuleInput::default()
    };
    (
        ChannelInput::new(
            "https://feeds.example.test/a?filter=1080p&token=old",
            "/old",
        ),
        vec![
            rule("Keep1", "old/keep1"),
            rule("Gone1", "gone1"),
            RuleInput {
                regex: true,
                case_insensitive: true,
                ..rule(r"^Regex \d+$", "old/regex")
            },
            rule("Gone2", "gone2"),
            rule("Gone3", "gone3"),
        ],
    )
}

fn choice(index: usize, existing: &ChannelWithRules, decision: &str) -> Value {
    json!({
        "index": index,
        "existing_id": existing.channel.id,
        "existing_version": existing.channel.version,
        "decision": decision,
    })
}

fn assert_no_secret(text: &str) {
    for secret in [TOKEN_A, TOKEN_B, "1080p"] {
        assert!(!text.contains(secret), "response leaks {secret}: {text}");
    }
}

#[tokio::test]
async fn an_empty_app_takes_everything_without_asking_and_keeps_the_file_exactly() {
    let t = app().await;

    let (status, text, review) = t.preview(&file()).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_no_secret(&text);
    assert_eq!(review["conflict_count"], 0);
    let channels = review["channels"].as_array().unwrap();
    assert_eq!(channels.len(), 2);
    assert!(channels.iter().all(|c| c["existing"].is_null()));
    assert_eq!(
        channels[0]["url"],
        "https://feeds.example.test/a?filter=***&token=***"
    );
    // The review lists the rules in file order with their values.
    let phrases: Vec<_> = channels[1]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["match"].clone())
        .collect();
    assert_eq!(
        phrases,
        [json!("B1"), Value::Null, json!("B3"), json!("B4")]
    );
    assert_eq!(channels[0]["rules"][1]["episode"], -24);
    assert_eq!(channels[0]["rules"][1]["regex"], true);
    assert!(t.all().await.is_empty(), "a preview changes nothing");

    let (status, text, result) = t.apply(&file(), json!([])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_no_secret(&text);
    assert_eq!(result["counts"]["channels_added"], 2);
    assert_eq!(result["counts"]["rules_added"], 7);
    assert_eq!(result["counts"]["rules_removed"], 0);
    assert_eq!(result["counts"]["channels_unchanged"], 0);

    let stored = t.all().await;
    assert_eq!(stored.len(), 2);
    let a = &stored[0];
    assert_eq!(a.channel.position, 0);
    assert_eq!(
        a.channel.url,
        format!("https://feeds.example.test/a?filter=1080p&token={TOKEN_A}")
    );
    assert_eq!(a.channel.secret_query, ["filter", "token"]);
    assert_eq!(a.channel.base_dir, "/media/a");
    assert_eq!(a.channel.excludes, ["[Batch]", "(720p)"]);
    assert_eq!(
        a.rules
            .iter()
            .map(|r| r.r#match.as_deref())
            .collect::<Vec<_>>(),
        [Some("Keep1"), Some(r"^Regex \d+$"), Some("Fresh")]
    );
    assert_eq!(a.rules[0].episode, 1, "an omitted episode reads as 1");
    assert_eq!(
        (
            a.rules[1].episode,
            a.rules[1].regex,
            a.rules[1].case_insensitive
        ),
        (-24, true, true)
    );
    assert_eq!(a.rules[2].episode, 13);
    assert_eq!(stored[1].channel.secret_query, ["token"]);
    // `match: ''` is stored as a rule waiting for its title, never as ''.
    assert_eq!(stored[1].rules[1].r#match, None);
    assert_eq!(stored[1].rules[1].directory, "B/waiting");
    assert_eq!(stored[1].rules[2].episode, 0);
    assert_eq!(stored[1].rules.len(), 4);
}

#[tokio::test]
async fn replace_shows_the_rules_that_go_and_reports_them_separately() {
    let t = app().await;
    let (input, rules) = existing_a();
    let a = t
        .store
        .create_channel_with_rules(input, rules)
        .await
        .unwrap();

    let (status, text, review) = t.preview(&file()).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_no_secret(&text);
    assert_eq!(review["conflict_count"], 1);
    let existing = &review["channels"][0]["existing"];
    assert_eq!(existing["id"], a.channel.id);
    assert_eq!(existing["version"], a.channel.version);
    assert_eq!(existing["rule_count"], 5);
    assert_eq!(
        existing["removed_rules"],
        json!([
            { "match": "Gone1", "directory": "gone1" },
            { "match": "Gone2", "directory": "gone2" },
            { "match": "Gone3", "directory": "gone3" },
        ])
    );
    let kept: Vec<_> = review["channels"][0]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["keeps_existing_rule"].as_bool().unwrap())
        .collect();
    assert_eq!(kept, [true, true, false]);
    assert!(review["channels"][1]["existing"].is_null());
    assert_eq!(t.all().await, vec![a.clone()], "a preview changes nothing");

    let (status, text, result) = t.apply(&file(), json!([choice(0, &a, "replace")])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_no_secret(&text);
    assert_eq!(result["counts"]["channels_replaced"], 1);
    assert_eq!(result["counts"]["channels_added"], 1);
    assert_eq!(result["counts"]["rules_removed"], 3);
    assert_eq!(result["counts"]["rules_kept"], 2);
    assert_eq!(result["counts"]["rules_added"], 1 + 4);
    assert_eq!(
        result["replaced"][0]["removed_rules"],
        existing["removed_rules"]
    );

    let stored = t.all().await;
    assert_eq!(stored.len(), 2);
    let after = &stored[0];
    assert_eq!(after.channel.id, a.channel.id);
    assert_eq!(after.channel.version, a.channel.version + 1);
    assert_eq!(after.channel.base_dir, "/media/a");
    assert_eq!(
        after
            .rules
            .iter()
            .map(|r| r.directory.as_str())
            .collect::<Vec<_>>(),
        ["A/keep1", "A/regex", "A/fresh"]
    );
    // The two overlapping rules kept their IDs.
    assert_eq!(after.rules[0].id, a.rules[0].id);
    assert_eq!(after.rules[1].id, a.rules[2].id);
    assert!(a.rules.iter().all(|r| r.id != after.rules[2].id));
    assert_eq!(after.rules[1].episode, -24);
    assert!(
        stored[1].channel.url.contains(TOKEN_B),
        "the new channel came last"
    );
}

#[tokio::test]
async fn add_copies_the_channel_with_new_ids_and_leaves_the_existing_one_alone() {
    let t = app().await;
    let (input, rules) = existing_a();
    let a = t
        .store
        .create_channel_with_rules(input, rules)
        .await
        .unwrap();

    let (status, text, _) = t.apply(&file(), json!([choice(0, &a, "add")])).await;
    assert_eq!(status, StatusCode::OK, "{text}");

    let stored = t.all().await;
    assert_eq!(stored.len(), 3);
    assert_eq!(stored[0], a);
    assert_ne!(stored[1].channel.id, a.channel.id);
    assert_eq!(stored[1].rules.len(), 3);
    assert!(stored[1]
        .rules
        .iter()
        .all(|n| a.rules.iter().all(|o| o.id != n.id)));
}

#[tokio::test]
async fn skip_leaves_everything_and_choices_must_cover_every_conflict() {
    let t = app().await;
    let (input, rules) = existing_a();
    let a = t
        .store
        .create_channel_with_rules(input, rules)
        .await
        .unwrap();
    let b = t
        .store
        .create_channel(ChannelInput::new(
            "https://feeds.example.test/b?token=old",
            "/old-b",
        ))
        .await
        .unwrap();
    let b = ChannelWithRules {
        channel: b,
        rules: vec![],
    };
    let before = t.all().await;

    // One of two conflicting channels chosen, none chosen, and a choice for a
    // file channel that does not conflict: all refused, nothing changes.
    for choices in [
        json!([choice(0, &a, "replace")]),
        json!([]),
        json!([
            choice(0, &a, "skip"),
            choice(1, &b, "skip"),
            choice(2, &b, "skip")
        ]),
    ] {
        let (status, text, body) = t.apply(&file(), choices).await;
        assert_eq!(status, StatusCode::CONFLICT, "{text}");
        assert_eq!(body["error"], "conflict");
        assert!(body["message"].as_str().unwrap().contains("다시 검토"));
        assert_eq!(t.all().await, before);
    }

    let (status, text, result) = t
        .apply(
            &file(),
            json!([choice(0, &a, "skip"), choice(1, &b, "skip")]),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_no_secret(&text);
    assert_eq!(result["counts"]["channels_skipped"], 2);
    assert_eq!(result["counts"]["channels_unchanged"], 2);
    assert_eq!(result["skipped"].as_array().unwrap().len(), 2);
    assert_eq!(t.all().await, before);
}

#[tokio::test]
async fn a_channel_changed_after_the_review_is_refused_not_replaced() {
    let t = app().await;
    let (input, rules) = existing_a();
    let a = t
        .store
        .create_channel_with_rules(input, rules)
        .await
        .unwrap();
    let reviewed = choice(0, &a, "replace");

    // Someone edits the channel after the user reviewed it.
    t.store
        .update_channel(&a.channel.id, a.channel.version, a.channel.to_input())
        .await
        .unwrap();
    let before = t.all().await;

    let (status, _, body) = t.apply(&file(), json!([reviewed])).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "conflict");
    assert_eq!(t.all().await, before);
}

#[tokio::test]
async fn files_that_cannot_be_read_fail_with_a_reason_and_change_nothing() {
    let t = app().await;
    let (input, rules) = existing_a();
    t.store
        .create_channel_with_rules(input, rules)
        .await
        .unwrap();
    let before = t.all().await;

    let missing_directory = format!(
        "- url: https://x.test/rss?token={TOKEN_A}\n  directory: /m\n  rules:\n    - match: a\n"
    );
    let cases = [
        ("PNG\u{0}\u{1}binary: [", "YAML"),
        ("just prose, not a channel list", "채널 목록이 아니"),
        ("[]", "가져올 채널이 없"),
        (
            "- url: https://x.test/rss\n  rules: []\n",
            "1번째 채널에 `directory`",
        ),
        (missing_directory.as_str(), "`directory`가 없"),
    ];
    for (content, reason) in cases {
        for uri in ["/api/import/legacy/preview", "/api/import/legacy/apply"] {
            let (status, text, body) = t
                .post(uri, json!({ "content": content, "choices": [] }))
                .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{content}: {text}");
            assert_eq!(body["error"], "invalid");
            let message = body["message"].as_str().unwrap();
            assert!(message.contains(reason), "{message}");
            assert_no_secret(&text);
        }
        assert_eq!(t.all().await, before);
    }

    // A body that is not the expected JSON is a plain refusal too.
    let (status, _, body) = t
        .post_raw("/api/import/legacy/preview", "not json".into())
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid");
}

#[tokio::test]
async fn a_database_error_mid_apply_leaves_the_state_from_before() {
    let t = app().await;
    let (input, rules) = existing_a();
    let a = t
        .store
        .create_channel_with_rules(input, rules)
        .await
        .unwrap();
    let before = t.all().await;

    // A is replaced and B's first rules are written, then the database fails
    // on B's last rule.
    let raw = rusqlite::Connection::open(&t.path).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER inject_failure BEFORE INSERT ON rules
         WHEN NEW.directory = 'B/four'
         BEGIN SELECT RAISE(ABORT, 'injected failure'); END;",
    )
    .unwrap();
    let (status, text, body) = t.apply(&file(), json!([choice(0, &a, "replace")])).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{text}");
    assert_eq!(body["error"], "internal");
    assert_no_secret(&text);
    assert_eq!(t.all().await, before, "A and B are as they were");

    raw.execute_batch("DROP TRIGGER inject_failure").unwrap();
    let (status, text, _) = t.apply(&file(), json!([choice(0, &a, "replace")])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(t.all().await.len(), 2);
}
