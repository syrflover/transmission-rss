use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

use trss_core::{
    settings::{CollectionSettings, SettingsStore},
    Db,
};
use trss_legacy::store::{
    channels::{ChannelInput, ChannelStore, RuleInput},
    library::LibraryStore,
    setup::{SetupStore, Step},
};

use super::*;

const TOKEN_A: &str = "sekret-token-A-123";
const TOKEN_B: &str = "sekret-token-B-456";

struct App {
    _dir: TempDir,
    path: std::path::PathBuf,
    store: ChannelStore,
    settings: SettingsStore,
    library: LibraryStore,
    setup: SetupStore,
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
        library: state.library.clone(),
        setup: state.setup.clone(),
        app: Router::new().nest("/api", crate::api::router().with_state(state)),
    }
}

impl App {
    /// The scratch folder the app's database lives in; it exists.
    fn root(&self) -> String {
        self._dir.path().to_str().unwrap().to_owned()
    }

    /// `content` with `/media` and `/downloads` moved under the scratch folder
    /// and created there: a collect folder the import sets has to exist.
    fn real(&self, content: &str) -> String {
        let root = self.root();
        for name in ["media", "downloads"] {
            std::fs::create_dir_all(format!("{root}/{name}")).unwrap();
        }
        content
            .replace("/media", &format!("{root}/media"))
            .replace("/downloads", &format!("{root}/downloads"))
    }

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

    let (status, text, review) = t.preview(&t.real(&file())).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_no_secret(&text);
    assert_eq!(review["conflict_count"], 0);
    // No collect folder yet: the file's folders have `/media` in common.
    assert_eq!(review["collect_folder"]["current"], Value::Null);
    let media = format!("{}/media", t.root());
    assert_eq!(review["collect_folder"]["will_set"], media.as_str());
    let channels = review["channels"].as_array().unwrap();
    assert_eq!(channels.len(), 2);
    assert!(channels.iter().all(|c| c["existing"].is_null()));
    assert!(channels.iter().all(|c| c["not_imported"].is_null()));
    assert_eq!(channels[0]["directory"], format!("{media}/a").as_str());
    assert_eq!(channels[1]["directory"], format!("{media}/b").as_str());
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

    let (status, text, result) = t.apply(&t.real(&file()), json!([])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_no_secret(&text);
    assert_eq!(result["collect_folder_set"], media.as_str());
    let collection = t.collection().await.unwrap();
    assert_eq!(
        (collection.folder.as_str(), collection.archive_folder),
        (media.as_str(), None)
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
async fn replacing_keeps_the_title_waiting_subscriptions_of_the_channel() {
    use trss_legacy::store::anissia::Anime;
    use trss_legacy::store::channels::{NewSubscription, SubtitleMode};

    let t = app().await;
    t.set_folder("/media").await;
    let a = t
        .store
        .create_channel_with_rules(
            ChannelInput::new("https://feeds.example.test/a?filter=1080p&token=old"),
            vec![RuleInput {
                r#match: Some("Gone".into()),
                directory: "gone".into(),
                ..RuleInput::default()
            }],
        )
        .await
        .unwrap();
    // A subscription created in the app for a work that has not aired yet.
    let waiting = t
        .store
        .create_subscription_rule(
            &a.channel.id,
            RuleInput {
                r#match: None,
                directory: "Waiting Work".into(),
                ..RuleInput::default()
            },
            NewSubscription {
                anime: Anime {
                    anime_no: 77,
                    subject: "Waiting Work".into(),
                    original_subject: None,
                    week: 3,
                    air_time: Some("22:00".into()),
                    start_date: None,
                    end_date: None,
                    status: "ON".into(),
                    fetched_at: 1,
                },
                subtitles: SubtitleMode::Undecided,
                creator: None,
                subscribed_at: 100,
            },
        )
        .await
        .unwrap();
    let a = t.all().await.remove(0);
    assert_eq!(a.rules.len(), 2);

    let content = format!(
        "- url: https://feeds.example.test/a?filter=1080p&token={TOKEN_A}
  directory: /media/a
  rules:
    - match: Fresh
      directory: A/fresh
"
    );
    // The preview says the subscription stays and does not list it as lost.
    let (status, text, review) = t.preview(&content).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let existing = &review["channels"][0]["existing"];
    assert_eq!(existing["rule_count"], 2);
    assert_eq!(existing["title_waiting_kept"], 1);
    assert_eq!(
        existing["removed_rules"],
        json!([{ "match": "Gone", "directory": "gone" }])
    );

    let (status, text, result) = t.apply(&content, json!([choice(0, &a, "replace")])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(result["counts"]["rules_removed"], 1);
    assert_eq!(result["counts"]["rules_added"], 1);
    assert_eq!(result["counts"]["title_waiting_kept"], 1);
    assert_eq!(result["replaced"][0]["title_waiting_kept"], 1);
    assert_eq!(result["replaced"][0]["added_rules"], 1);

    // The ordinary rule was replaced; the waiting subscription is the same
    // rule, with the same subscription row, after the file's rule.
    let stored = t.all().await.remove(0);
    assert_eq!(stored.rules.len(), 2);
    assert_eq!(stored.rules[0].r#match.as_deref(), Some("Fresh"));
    assert_eq!(stored.rules[0].directory, "a/A/fresh");
    assert_ne!(stored.rules[0].id, a.rules[0].id);
    assert_eq!(stored.rules[1], waiting_after(&waiting, 1));
}

#[tokio::test]
async fn replacing_keeps_the_folder_of_a_subscription_when_the_files_folder_is_no_work_folder() {
    use trss_legacy::store::anissia::Anime;
    use trss_legacy::store::channels::{NewSubscription, SubtitleMode};

    let t = app().await;
    t.set_folder("/media").await;
    let subscribe = |channel: &str, no: i64, phrase: &str, directory: &str| {
        let store = t.store.clone();
        let (channel, phrase, directory) =
            (channel.to_owned(), phrase.to_owned(), directory.to_owned());
        async move {
            store
                .create_subscription_rule(
                    &channel,
                    RuleInput {
                        r#match: Some(phrase),
                        directory,
                        ..RuleInput::default()
                    },
                    NewSubscription {
                        anime: Anime {
                            anime_no: no,
                            subject: format!("작품 {no}"),
                            original_subject: None,
                            week: 3,
                            air_time: Some("22:00".into()),
                            start_date: None,
                            end_date: None,
                            status: "ON".into(),
                            fetched_at: 1,
                        },
                        subtitles: SubtitleMode::Undecided,
                        creator: None,
                        subscribed_at: 100,
                    },
                )
                .await
                .unwrap()
        }
    };
    let a = t
        .store
        .create_channel_with_rules(
            ChannelInput::new("https://feeds.example.test/a?filter=1080p&token=old"),
            vec![RuleInput {
                r#match: Some("Plain".into()),
                directory: "plain old".into(),
                ..RuleInput::default()
            }],
        )
        .await
        .unwrap();
    subscribe(&a.channel.id, 1, "Sub", "Sub Work").await;
    subscribe(&a.channel.id, 2, "Climb", "Climb Work").await;
    subscribe(&a.channel.id, 3, "Fine", "Fine Old").await;
    let a = t.all().await.remove(0);

    // The channel's folder is the collect folder itself. The file gives `Sub`
    // no folder of its own (`.`), `Climb` a `..` folder and `Fine` a work
    // folder; the plain rule `Plain` has no folder either.
    let content = format!(
        "- url: https://feeds.example.test/a?filter=1080p&token={TOKEN_A}
  directory: /media
  rules:
    - match: Plain
      directory: .
    - match: Sub
      directory: .
    - match: Climb
      directory: ../elsewhere
    - match: Fine
      directory: Fine New
"
    );

    // The preview names the subscriptions whose folder stays.
    let (status, text, review) = t.preview(&content).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let kept: Vec<_> = review["channels"][0]["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["folder_kept"].clone())
        .collect();
    assert_eq!(
        kept,
        [
            Value::Null,
            json!("Sub Work"),
            json!("Climb Work"),
            Value::Null
        ]
    );

    let (status, text, result) = t.apply(&content, json!([choice(0, &a, "replace")])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(result["counts"]["folders_kept"], 2);
    assert_eq!(
        result["replaced"][0]["folders_kept"],
        json!([
            { "rule": 1, "match": "Sub", "directory": "Sub Work" },
            { "rule": 2, "match": "Climb", "directory": "Climb Work" },
        ])
    );

    let stored = t.all().await.remove(0);
    let folders: Vec<_> = stored
        .rules
        .iter()
        .map(|r| (r.r#match.as_deref().unwrap(), r.directory.as_str()))
        .collect();
    // The rule without a subscription took the file's folder, as before; the
    // subscriptions kept theirs, except the one with a work folder in the file.
    assert_eq!(
        folders,
        [
            ("Plain", "."),
            ("Sub", "Sub Work"),
            ("Climb", "Climb Work"),
            ("Fine", "Fine New"),
        ]
    );
    // They are still subscriptions, and the rule screen can save them.
    assert!(stored.rules[1..].iter().all(|r| r.subscription.is_some()));
    assert!(trss_core::folders::is_work_folder(
        &stored.rules[1].directory
    ));
}

/// `rule` as it is stored after it moved to `position`.
fn waiting_after(
    rule: &trss_legacy::store::channels::Rule,
    position: i64,
) -> trss_legacy::store::channels::Rule {
    trss_legacy::store::channels::Rule {
        position,
        ..rule.clone()
    }
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

    let (status, text, _) = t
        .apply(&t.real(&file()), json!([choice(0, &a, "add")]))
        .await;
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
        let (status, text, body) = t.apply(&t.real(&file()), choices).await;
        assert_eq!(status, StatusCode::CONFLICT, "{text}");
        assert_eq!(body["error"], "conflict");
        assert!(body["message"].as_str().unwrap().contains("다시 검토"));
        assert_eq!(t.all().await, before);
    }

    let (status, text, result) = t
        .apply(
            &t.real(&file()),
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

    let (status, _, body) = t.apply(&t.real(&file()), json!([reviewed])).await;
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
    let (status, text, body) = t
        .apply(&t.real(&file()), json!([choice(0, &a, "replace")]))
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{text}");
    assert_eq!(body["error"], "internal");
    assert_no_secret(&text);
    assert_eq!(t.all().await, before, "A and B are as they were");

    raw.execute_batch("DROP TRIGGER inject_failure").unwrap();
    let (status, text, _) = t
        .apply(&t.real(&file()), json!([choice(0, &a, "replace")]))
        .await;
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
async fn an_import_that_sets_the_collect_folder_also_makes_it_a_watch_folder() {
    let t = app().await;
    let (status, text, result) = t.apply(&t.real(&file()), json!([])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let media = format!("{}/media", t.root());
    assert_eq!(result["collect_folder_set"], media.as_str());

    let folders = t.library.folders().await.unwrap();
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].path, media);
    assert!(folders[0].automatic);
}

#[tokio::test]
async fn an_import_whose_collect_folder_would_sit_inside_a_registered_watch_folder_changes_nothing()
{
    let t = app().await;
    let content = t.real(&file());
    // The root of the scratch folder is a watch folder, and `/media` is in it.
    let root = t.root();
    t.library
        .add_folder(
            root.clone(),
            trss_legacy::discovery::Scan::default(),
            1,
            &[],
        )
        .await
        .unwrap();
    let (status, text, result) = t.apply(&content, json!([])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert!(
        result["message"].as_str().unwrap().contains("안에 있어요"),
        "{text}"
    );
    assert!(t.all().await.is_empty());
    assert_eq!(t.collection().await, None);
    assert_eq!(t.library.folders().await.unwrap().len(), 1);
}

#[tokio::test]
async fn differing_channel_folders_set_their_common_parent_and_prefix_the_rules() {
    let t = app().await;

    let (_, text, review) = t.preview(&t.real(&folders_file())).await;
    assert_no_secret(&text);
    let downloads = format!("{}/downloads", t.root());
    assert_eq!(review["collect_folder"]["will_set"], downloads.as_str());
    assert_eq!(
        review["channels"][0]["rules"][0]["directory"],
        "Shows (current)/Clevatess/Season 02"
    );

    let (status, text, result) = t.apply(&t.real(&folders_file()), json!([])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(result["collect_folder_set"], downloads.as_str());
    assert_eq!(t.collection().await.unwrap().folder, downloads);

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
    let content = t.real(&format!(
        "- url: https://feeds.example.test/a?filter=1080p&token={TOKEN_A}
  directory: /media/a
  rules:
    - match: Keep1
      directory: X
"
    ));
    // A single channel's folder would be the collect folder itself.
    std::fs::create_dir_all(format!("{}/media/a", t.root())).unwrap();

    let (status, text, result) = t.apply(&content, json!([choice(0, &a, "skip")])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(result["collect_folder_set"], Value::Null);
    assert_eq!(t.collection().await, None);
}

#[tokio::test]
async fn an_apply_that_creates_or_changes_nothing_does_not_finish_the_import_step() {
    let t = app().await;
    let (input, rules) = existing_a();
    let a = t
        .store
        .create_channel_with_rules(input, rules)
        .await
        .unwrap();
    t.set_folder(&t.root()).await;
    let content = t.real(&format!(
        "- url: https://feeds.example.test/a?filter=1080p&token={TOKEN_A}
  directory: /media/a
  rules:
    - match: Keep1
      directory: X
"
    ));
    let done = || async {
        t.setup
            .first_run()
            .await
            .unwrap()
            .unwrap()
            .done(Step::Import)
    };
    assert!(!done().await);

    // Every channel skipped: nothing was created or changed.
    let (status, text, result) = t.apply(&content, json!([choice(0, &a, "skip")])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(result["counts"]["channels_skipped"], 1);
    assert!(!done().await);

    // A channel that is added counts.
    let (status, text, result) = t.apply(&content, json!([choice(0, &a, "add")])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(result["counts"]["channels_added"], 1);
    assert!(done().await);
}

#[tokio::test]
async fn replacing_a_channel_finishes_the_import_step() {
    let t = app().await;
    let (input, rules) = existing_a();
    let a = t
        .store
        .create_channel_with_rules(input, rules)
        .await
        .unwrap();
    t.set_folder(&t.root()).await;
    let content = t.real(&format!(
        "- url: https://feeds.example.test/a?filter=1080p&token={TOKEN_A}
  directory: /media/a
  rules:
    - match: Other
      directory: X
"
    ));
    assert!(!t
        .setup
        .first_run()
        .await
        .unwrap()
        .unwrap()
        .done(Step::Import));
    let (status, text, _) = t.apply(&content, json!([choice(0, &a, "replace")])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(t
        .setup
        .first_run()
        .await
        .unwrap()
        .unwrap()
        .done(Step::Import));
}

/// A one-channel file whose folder is `folder`.
fn file_in(folder: &str) -> String {
    channel_in("a", folder)
}

/// One channel, `name` telling it from the others, in `folder`.
fn channel_in(name: &str, folder: &str) -> String {
    format!(
        "- url: https://feeds.example.test/{name}?token={TOKEN_A}
  directory: {folder}
  rules:
    - match: A1
      directory: Show
"
    )
}

/// Both steps refuse `content` with `400 invalid`, a message that holds
/// `needle` and no secret, and nothing is imported or set.
async fn assert_refused(t: &App, content: &str, needle: &str) {
    let (status, text, body) = t.preview(content).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert_eq!(body["error"], "invalid");
    assert!(body["message"].as_str().unwrap().contains(needle), "{text}");
    assert_no_secret(&text);

    let (status, text, body) = t.apply(content, json!([])).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    assert_eq!(body["error"], "invalid");
    assert!(body["message"].as_str().unwrap().contains(needle), "{text}");
    assert!(t.all().await.is_empty());
    assert_eq!(t.collection().await, None);
}

#[tokio::test]
async fn a_collect_folder_the_import_would_set_must_be_an_existing_directory() {
    let t = app().await;
    let root = t.root();

    // A folder that does not exist (as the settings screen would say).
    assert_refused(&t, &file_in(&format!("{root}/missing/a")), "찾지 못했어요").await;

    // A file, not a directory.
    std::fs::write(format!("{root}/a-file"), "x").unwrap();
    assert_refused(&t, &file_in(&format!("{root}/a-file")), "폴더가 아니에요").await;
}

#[tokio::test]
async fn an_import_that_would_set_a_relative_or_root_collect_folder_is_refused() {
    let t = app().await;

    assert_refused(&t, &file_in("relative/a"), "전체 경로가 아니어서").await;
    assert_refused(&t, &file_in("/"), "`/`가 되어").await;
    let two = format!("{}{}", file_in("/media/a"), channel_in("b", "/downloads/b"));
    assert_refused(&t, &two, "설정에서 수집 폴더를 먼저").await;

    // Once the collect folder is set in settings, the same file imports.
    let media = t.real("/media");
    t.set_folder(&media).await;
    let (status, text, result) = t.apply(&file_in(&format!("{media}/a")), json!([])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(result["counts"]["channels_added"], 1);
}

#[tokio::test]
async fn a_channel_folder_that_climbs_out_of_the_collect_folder_is_not_imported() {
    let t = app().await;
    let media = t.real("/media");
    t.set_folder(&media).await;
    let content = format!(
        "{}{}",
        file_in(&format!("{media}/a")),
        channel_in("b", &format!("{media}/../downloads"))
    );

    let (status, text, review) = t.preview(&content).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert!(review["channels"][0]["not_imported"].is_null());
    let reason = review["channels"][1]["not_imported"].as_str().unwrap();
    assert!(reason.contains("`..`"), "{reason}");

    let (status, text, result) = t.apply(&content, json!([])).await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(result["counts"]["channels_added"], 1);
    assert_eq!(result["counts"]["channels_not_imported"], 1);
    assert_eq!(t.all().await.len(), 1);
}
