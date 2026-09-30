use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

use crate::store::channels::{ChannelInput, ChannelStore, RuleInput};
use crate::store::settings::{CollectionSettings, SettingsStore};
use crate::store::Db;

use super::*;

const TOKEN_A: &str = "sekret-token-A-123";
const TOKEN_B: &str = "sekret-token-B-456";

struct App {
    _dir: TempDir,
    path: std::path::PathBuf,
    store: ChannelStore,
    settings: SettingsStore,
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
        settings: state.settings.clone(),
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

    /// Applies as the screen does: it sends back the collect folder the review
    /// showed, which is the one set now.
    async fn apply(&self, content: &str, choices: Value) -> (StatusCode, String, Value) {
        let reviewed = self.collection().await.map(|c| c.folder);
        self.apply_reviewed(content, choices, reviewed).await
    }

    async fn apply_reviewed(
        &self,
        content: &str,
        choices: Value,
        reviewed_collect_folder: Option<String>,
    ) -> (StatusCode, String, Value) {
        self.post(
            "/api/import/legacy/apply",
            json!({
                "content": content,
                "choices": choices,
                "reviewed_collect_folder": reviewed_collect_folder,
            }),
        )
        .await
    }

    async fn collection(&self) -> Option<CollectionSettings> {
        self.settings.collection().await.unwrap()
    }

    async fn set_folder(&self, folder: &str) {
        let version = self.collection().await.map_or(0, |c| c.version);
        self.settings
            .put_collection(version, folder.to_owned(), None)
            .await
            .unwrap();
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
    - match: B2
      directory: B/two
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
        ChannelInput::new("https://feeds.example.test/a?filter=1080p&token=old"),
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
    // No collect folder yet: the file's folders have `/media` in common.
    assert_eq!(review["collect_folder"]["current"], Value::Null);
    assert_eq!(review["collect_folder"]["will_set"], "/media");
    let channels = review["channels"].as_array().unwrap();
    assert_eq!(channels.len(), 2);
    assert!(channels.iter().all(|c| c["existing"].is_null()));
    assert!(channels.iter().all(|c| c["not_imported"].is_null()));
    assert_eq!(channels[0]["directory"], "/media/a");
    assert_eq!(channels[1]["directory"], "/media/b");
    // The review lists the rule folders as they will be stored, under the
    // collect folder.
    assert_eq!(channels[0]["rules"][0]["directory"], "a/A/keep1");
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
        [json!("B1"), json!("B2"), json!("B3"), json!("B4")]
    );
    assert_eq!(channels[0]["rules"][1]["episode"], -24);
    assert_eq!(channels[0]["rules"][1]["regex"], true);
    assert!(t.all().await.is_empty(), "a preview changes nothing");
    assert_eq!(t.collection().await, None, "a preview sets no folder");

    let (status, text, result) = t.apply(&file(), json!([])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_no_secret(&text);
    assert_eq!(result["collect_folder_set"], "/media");
    let collection = t.collection().await.unwrap();
    assert_eq!(
        (collection.folder.as_str(), collection.archive_folder),
        ("/media", None)
    );
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
    assert_eq!(a.channel.excludes, ["[Batch]", "(720p)"]);
    assert_eq!(
        a.rules
            .iter()
            .map(|r| r.r#match.as_deref())
            .collect::<Vec<_>>(),
        [Some("Keep1"), Some(r"^Regex \d+$"), Some("Fresh")]
    );
    assert_eq!(
        a.rules
            .iter()
            .map(|r| r.directory.as_str())
            .collect::<Vec<_>>(),
        ["a/A/keep1", "a/A/regex", "a/A/fresh"],
        "what lies between the collect folder and the channel's folder goes in front"
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
    assert_eq!(stored[1].rules[1].r#match.as_deref(), Some("B2"));
    assert_eq!(stored[1].rules[1].directory, "b/B/two");
    assert_eq!(stored[1].rules[2].episode, 0);
    assert_eq!(stored[1].rules.len(), 4);
}

#[tokio::test]
async fn replace_shows_the_rules_that_go_and_reports_them_separately() {
    let t = app().await;
    t.set_folder("/media").await;
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
    assert_eq!(
        after
            .rules
            .iter()
            .map(|r| r.directory.as_str())
            .collect::<Vec<_>>(),
        ["a/A/keep1", "a/A/regex", "a/A/fresh"]
    );
    assert_eq!(result["collect_folder_set"], Value::Null);
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
        .create_channel(ChannelInput::new("https://feeds.example.test/b?token=old"))
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
         WHEN NEW.directory = 'b/B/four'
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

// --- folders ---------------------------------------------------------------------

/// A file whose channels sit in different folders under `/downloads`, and one
/// channel in `/elsewhere`.
fn folders_file() -> String {
    format!(
        r#"- url: https://feeds.example.test/a?token={TOKEN_A}
  directory: /downloads/Shows (current)
  rules:
    - match: A1
      directory: Clevatess/Season 02
    - match: A2
      directory: ""
- url: https://feeds.example.test/b?token={TOKEN_B}
  directory: /downloads/Movies
  rules:
    - match: B1
      directory: Dune
"#
    )
}

fn directories(channel: &ChannelWithRules) -> Vec<&str> {
    channel.rules.iter().map(|r| r.directory.as_str()).collect()
}

#[tokio::test]
async fn differing_channel_folders_set_their_common_parent_and_prefix_the_rules() {
    let t = app().await;

    let (_, text, review) = t.preview(&folders_file()).await;
    assert_no_secret(&text);
    assert_eq!(review["collect_folder"]["will_set"], "/downloads");
    assert_eq!(
        review["channels"][0]["rules"][0]["directory"],
        "Shows (current)/Clevatess/Season 02"
    );

    let (status, text, result) = t.apply(&folders_file(), json!([])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(result["collect_folder_set"], "/downloads");
    assert_eq!(t.collection().await.unwrap().folder, "/downloads");

    let stored = t.all().await;
    // The empty directory keeps its trailing slash, as the old save path had.
    assert_eq!(
        directories(&stored[0]),
        ["Shows (current)/Clevatess/Season 02", "Shows (current)/"]
    );
    assert_eq!(directories(&stored[1]), ["Movies/Dune"]);
}

#[tokio::test]
async fn with_a_collect_folder_set_a_channel_inside_it_is_prefixed_and_one_outside_is_reported() {
    let t = app().await;
    t.set_folder("/downloads/Shows (current)").await;
    let content = format!(
        "{}- url: https://feeds.example.test/c?token={TOKEN_A}
  directory: /downloads/Shows (current)/Old
  rules:
    - match: C1
      directory: Show
",
        folders_file()
    );

    let (status, text, review) = t.preview(&content).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_no_secret(&text);
    assert_eq!(
        review["collect_folder"]["current"],
        "/downloads/Shows (current)"
    );
    assert_eq!(review["collect_folder"]["will_set"], Value::Null);
    let channels = review["channels"].as_array().unwrap();
    assert!(channels[0]["not_imported"].is_null());
    let reason = channels[1]["not_imported"].as_str().unwrap();
    assert!(reason.contains("`/downloads/Movies`"), "{reason}");
    assert!(
        reason.contains("수집 폴더 `/downloads/Shows (current)` 밖"),
        "{reason}"
    );
    assert!(channels[2]["not_imported"].is_null());

    let (status, text, result) = t.apply(&content, json!([])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_no_secret(&text);
    assert_eq!(result["collect_folder_set"], Value::Null);
    assert_eq!(result["counts"]["channels_added"], 2);
    assert_eq!(result["counts"]["channels_not_imported"], 1);
    assert_eq!(result["not_imported"][0]["index"], 1);
    assert_eq!(
        result["not_imported"][0]["url"],
        "https://feeds.example.test/b?token=***"
    );
    assert_eq!(result["not_imported"][0]["reason"], reason);

    let stored = t.all().await;
    assert_eq!(stored.len(), 2, "the outside channel was not imported");
    assert_eq!(directories(&stored[0]), ["Clevatess/Season 02", ""]);
    assert_eq!(directories(&stored[1]), ["Old/Show"]);
    assert_eq!(
        t.collection().await.unwrap().folder,
        "/downloads/Shows (current)"
    );
}

#[tokio::test]
async fn a_channel_outside_the_collect_folder_needs_no_choice_even_if_it_exists_already() {
    let t = app().await;
    t.set_folder("/downloads/Shows (current)").await;
    let (input, rules) = existing_a();
    let a = t
        .store
        .create_channel_with_rules(input, rules)
        .await
        .unwrap();
    // The file's channel `a` is the existing one, but its folder is outside.
    let content = format!(
        "- url: https://feeds.example.test/a?filter=1080p&token={TOKEN_A}
  directory: /elsewhere
  rules:
    - match: Keep1
      directory: X
"
    );

    let (_, _, review) = t.preview(&content).await;
    assert_eq!(review["conflict_count"], 0);
    assert!(review["channels"][0]["existing"].is_null());
    assert!(review["channels"][0]["not_imported"].is_string());

    // No choices are needed, and the existing channel is not touched.
    let before = t.all().await;
    let (status, text, result) = t.apply(&content, json!([])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(result["counts"]["channels_not_imported"], 1);
    assert_eq!(result["counts"]["channels_added"], 0);
    assert_eq!(t.all().await, before);

    // A choice for a channel that is not imported is a stale review.
    let (status, _, body) = t.apply(&content, json!([choice(0, &a, "replace")])).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "conflict");
}

#[tokio::test]
async fn a_collect_folder_that_changed_since_the_review_makes_it_stale() {
    let t = app().await;

    // The review saw no folder; one is set before the apply.
    t.set_folder("/downloads").await;
    let (status, _, body) = t.apply_reviewed(&folders_file(), json!([]), None).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "conflict");
    assert!(t.all().await.is_empty());

    // The review saw another folder than the one that is set.
    let (status, _, _) = t
        .apply_reviewed(&folders_file(), json!([]), Some("/media".into()))
        .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(t.all().await.is_empty());
}

#[tokio::test]
async fn skipping_every_channel_does_not_set_the_collect_folder() {
    let t = app().await;
    let (input, rules) = existing_a();
    let a = t
        .store
        .create_channel_with_rules(input, rules)
        .await
        .unwrap();
    let content = format!(
        "- url: https://feeds.example.test/a?filter=1080p&token={TOKEN_A}
  directory: /media/a
  rules:
    - match: Keep1
      directory: X
"
    );

    let (status, text, result) = t.apply(&content, json!([choice(0, &a, "skip")])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(result["collect_folder_set"], Value::Null);
    assert_eq!(t.collection().await, None);
}
