//! The `rule_archive_batch` request through `POST /api/commands` and the
//! batch's view through `GET /api/archive-batches`. How the batch runs, and
//! how the view counts and names the rules, is tested in
//! `archive_batch/tests.rs` and `tests/it/archive_move.rs`.

use std::sync::Arc;

use axum::{
    http::{Method, StatusCode},
    Router,
};
use serde_json::{json, Value};

use super::*;
use crate::{
    archive_batch::{
        child_id, stopped_at, ArchiveBatch, BatchRule, ARCHIVED, REFUSED, RULE_FAILED, WORK_BUSY,
    },
    web::testing,
};
use trss_anissia::{Anissia, AnissiaConfig};
use trss_collect::{
    commands::work_archive::WorkArchive,
    store::{
        channels::{ChannelInput, RuleInput},
        history::{HistoryResult, Observation},
        status::{read_day, ChannelReadResult},
    },
};
use trss_core::{
    commands::{Accepted, CommandState, NewCommand, Outcome},
    Clock, Db, Millis,
};
use trss_library::discovery::{Scan, ScannedWork, WorkRead};

const BATCH: &str = "batch-0123456789";

/// The clock the suggestions are read by: 2026-10-01 12:00 in Seoul.
const NOW: Millis = 1_790_780_400_000 + 12 * 60 * 60 * 1000;
const DAY: Millis = 24 * 60 * 60 * 1000;
const WEEK: Millis = 7 * DAY;

struct App {
    db: Db,
    state: AppState,
    router: Router,
    channel_id: String,
    /// Three rules saving into `Work {n}/Season 01`, whose work folders the
    /// library knows below the collect folder. None is suggested for archiving
    /// until a test makes it quiet ([`App::quiet`]).
    rule_ids: Vec<String>,
}

impl App {
    async fn new() -> App {
        let db = Db::open_blocking(":memory:").unwrap();
        let clock: Clock = Arc::new(|| NOW);
        let anissia = Anissia::new(db.clone(), AnissiaConfig::default(), clock);
        let state = AppState::new(db.clone()).with_anissia(anissia);
        state
            .settings
            .put_collection(0, "/media".into(), Some("/archive".into()))
            .await
            .unwrap();
        let works = (0..3)
            .map(|n| {
                WorkRead::Read(ScannedWork {
                    dir_name: format!("Work {n}"),
                    seasons: Default::default(),
                    files: Vec::new(),
                    unrecognized: Vec::new(),
                })
            })
            .collect();
        state
            .library
            .add_folder("/media".into(), Scan { works }, 1_000, &[])
            .await
            .unwrap();
        let rules = (0..3)
            .map(|n| RuleInput {
                r#match: Some(format!("Work {n}")),
                directory: format!("Work {n}/Season 01"),
                ..RuleInput::default()
            })
            .collect();
        let channel = state
            .channels
            .create_channel_with_rules(ChannelInput::new("https://feed.example/rss"), rules)
            .await
            .unwrap();
        let router = testing::api(&state);
        App {
            db,
            state,
            router,
            channel_id: channel.channel.id,
            rule_ids: channel.rules.into_iter().map(|rule| rule.id).collect(),
        }
    }

    /// Makes the rules `ns` suggested for archiving on `새 항목 없음`: they
    /// started collecting 8 weeks ago, the channel has been seen since a year
    /// ago and read every day, and nothing came for them. When the suggestions
    /// appear is the collect crate's rule (`archive_suggestions` tests).
    async fn quiet(&self, ns: &[usize]) {
        for &n in ns {
            let id = self.rule_ids[n].clone();
            self.db
                .run::<_, trss_core::DbError, _>(move |c| {
                    c.execute(
                        "UPDATE rule_started SET started_at = ?2 WHERE rule_id = ?1",
                        rusqlite::params![id, NOW - 8 * WEEK],
                    )?;
                    Ok(())
                })
                .await
                .unwrap();
        }
        self.record(NOW - 52 * WEEK, "[Other] Old - 01", None).await;
        for day in read_day(NOW) - 60..=read_day(NOW) {
            self.state
                .status
                .record_reads(
                    day * DAY,
                    vec![ChannelReadResult {
                        channel_id: self.channel_id.clone(),
                        ok: true,
                    }],
                    vec![self.channel_id.clone()],
                )
                .await
                .unwrap();
        }
    }

    /// An item of the channel titled `title` seen at `at`, received by the rule `n`.
    async fn record(&self, at: Millis, title: &str, received_by: Option<usize>) {
        self.state
            .history
            .record(
                at,
                vec![Observation {
                    channel_id: self.channel_id.clone(),
                    channel_label: "feed.example".into(),
                    identity_key: format!("title:{title}:{at}"),
                    title: title.to_owned(),
                    link: "https://feed.example/item".into(),
                    result: if received_by.is_some() {
                        HistoryResult::Received
                    } else {
                        HistoryResult::NoMatch
                    },
                    rule_id: received_by.map(|n| self.rule_ids[n].clone()),
                    torrent_hash: None,
                    reason: None,
                }],
            )
            .await
            .unwrap();
    }

    /// The IDs of the rules suggested for archiving now.
    async fn suggested(&self) -> Vec<String> {
        let (status, body) = self.get("/api/archive-suggestions").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["suggestions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["rule_id"].as_str().unwrap().to_owned())
            .collect()
    }

    /// The request of the rules `ns` named as the screen names them, `작품 {n}`.
    fn of_rules(&self, ns: &[usize]) -> Value {
        let rules: Vec<Value> = ns
            .iter()
            .map(|&n| json!({ "rule_id": self.rule_ids[n], "name": format!("작품 {n}") }))
            .collect();
        json!({ "rules": rules })
    }

    /// The library's work of the `n`th rule's work folder.
    async fn work_id(&self, n: usize) -> String {
        self.state
            .library
            .work_in_folder("/media", &format!("Work {n}"))
            .await
            .unwrap()
            .unwrap()
    }

    async fn call(&self, method: Method, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
        testing::call(&self.router, method, uri, body).await
    }

    async fn post(&self, id: &str, payload: Value) -> (StatusCode, Value) {
        let body = json!({ "id": id, "kind": "rule_archive_batch", "payload": payload });
        self.call(Method::POST, "/api/commands", Some(body)).await
    }

    async fn get(&self, uri: &str) -> (StatusCode, Value) {
        self.call(Method::GET, uri, None).await
    }

    /// Stores a batch of the first `rules` rules directly, as `POST` would.
    async fn store_batch(&self, id: &str, rules: usize) {
        let batch = ArchiveBatch {
            rules: self.rule_ids[..rules]
                .iter()
                .enumerate()
                .map(|(n, rule_id)| BatchRule {
                    rule_id: rule_id.clone(),
                    name: format!("작품 {n}"),
                })
                .collect(),
        };
        let accepted = self
            .state
            .commands
            .accept(
                NewCommand {
                    id: id.to_owned(),
                    kind: archive_batch::KIND.to_owned(),
                    payload: batch.canonical(),
                    subject: Some(batch.subject()),
                },
                1_000,
            )
            .await
            .unwrap();
        assert!(matches!(accepted, Accepted::Created(_)));
    }

    /// Stores the move the batch `id` made for its `n`th rule, ended as `end`
    /// says or left pending.
    async fn store_child(&self, id: &str, n: usize, end: Option<(CommandState, Outcome)>) {
        let child = child_id(id, n);
        let payload = WorkArchive {
            work: format!("Work {n}"),
        };
        self.state
            .commands
            .accept(payload.command(child.clone()), 1_100)
            .await
            .unwrap();
        if let Some((state, outcome)) = end {
            self.state
                .commands
                .claim_next_excluding(1_200, vec![id.to_owned()])
                .await
                .unwrap();
            self.state
                .commands
                .finish(&child, state, outcome, 1_300)
                .await
                .unwrap();
        }
    }

    async fn end_batch(&self, id: &str, state: CommandState, result: &str, reason: Option<&str>) {
        self.state
            .commands
            .claim_next(1_400)
            .await
            .unwrap()
            .unwrap();
        self.state
            .commands
            .finish(
                id,
                state,
                Outcome {
                    result: result.to_owned(),
                    reason: reason.map(str::to_owned),
                },
                1_500,
            )
            .await
            .unwrap();
    }
}

/// A batch of `rules` rules that are not there: the request tests do not need
/// rules, and a rule that is gone is skipped when its turn comes.
fn batch(rules: usize) -> ArchiveBatch {
    ArchiveBatch {
        rules: (0..rules)
            .map(|n| BatchRule {
                rule_id: format!("r{n}"),
                name: format!("작품 {n}"),
            })
            .collect(),
    }
}

/// The view without its `results`, whose commands carry times.
fn without_results(view: &Value) -> Value {
    let mut view = view.clone();
    view.as_object_mut().unwrap().remove("results");
    view
}

fn payload(rules: usize) -> Value {
    serde_json::to_value(batch(rules)).unwrap()
}

fn archived() -> (CommandState, Outcome) {
    (
        CommandState::Done,
        Outcome {
            result: "moved".to_owned(),
            reason: None,
        },
    )
}

// --- the request -----------------------------------------------------------------------

#[tokio::test]
async fn a_batch_is_accepted_as_one_command_with_its_rules_in_order() {
    let app = App::new().await;

    let (status, body) = app.post(BATCH, payload(3)).await;

    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(body["id"], BATCH);
    assert_eq!(body["kind"], "rule_archive_batch");
    assert_eq!(body["state"], "pending");
    let stored = app.state.commands.get(BATCH).await.unwrap().unwrap();
    assert_eq!(stored.subject.as_deref(), Some("archive-batch"));
    let stored: ArchiveBatch = serde_json::from_str(&stored.payload).unwrap();
    assert_eq!(stored, batch(3));
    // Only the batch is stored: its rules' commands are made when it runs.
    assert_eq!(
        app.state.commands.get(&child_id(BATCH, 0)).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn the_same_batch_sent_again_is_the_command_already_stored() {
    let app = App::new().await;
    let (first, one) = app.post(BATCH, payload(2)).await;

    let (second, two) = app.post(BATCH, payload(2)).await;

    assert_eq!((first, second), (StatusCode::ACCEPTED, StatusCode::OK));
    assert_eq!(one, two);
}

#[tokio::test]
async fn the_same_id_with_other_rules_is_refused() {
    let app = App::new().await;
    app.post(BATCH, payload(2)).await;

    let (status, body) = app.post(BATCH, payload(3)).await;

    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["current"]["id"], BATCH);
}

#[tokio::test]
async fn a_batch_without_rules_or_with_a_rule_twice_or_with_a_field_too_long_is_refused_and_stores_nothing(
) {
    let app = App::new().await;
    let mut twice = payload(2);
    twice["rules"][1]["rule_id"] = json!("r0");
    let mut long_id = payload(1);
    long_id["rules"][0]["rule_id"] = json!("x".repeat(65));
    let mut long_name = payload(1);
    long_name["rules"][0]["name"] = json!("작".repeat(501));
    let cases = [
        ("empty", payload(0)),
        ("a rule twice", twice),
        ("a rule ID of 65 bytes", long_id),
        ("a name of 501 characters", long_name),
    ];

    for (case, body) in cases {
        let (status, answer) = app.post(BATCH, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{case}: {answer}");
        assert_eq!(answer["error"], "invalid", "{case}");
        assert_eq!(app.state.commands.get(BATCH).await.unwrap(), None, "{case}");
    }
}

#[tokio::test]
async fn a_batch_of_any_length_is_accepted_and_the_fields_at_their_limits_are_kept() {
    let app = App::new().await;
    let mut long = payload(250);
    long["rules"][0]["rule_id"] = json!("x".repeat(64));
    long["rules"][0]["name"] = json!("작".repeat(500));

    let (status, body) = app.post(BATCH, long).await;

    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let stored = app.state.commands.get(BATCH).await.unwrap().unwrap();
    let stored: ArchiveBatch = serde_json::from_str(&stored.payload).unwrap();
    assert_eq!(stored.rules.len(), 250);
}

#[tokio::test]
async fn a_payload_with_a_field_the_batch_does_not_have_is_refused() {
    let app = App::new().await;
    let cases = [
        ("a field of the batch", json!({ "rules": [], "extra": 1 })),
        (
            "a field of a rule",
            json!({ "rules": [{ "rule_id": "r", "name": "n", "extra": 1 }] }),
        ),
        (
            "a rule with no name",
            json!({ "rules": [{ "rule_id": "r" }] }),
        ),
        ("no rules", json!({})),
        ("rules that are not a list", json!({ "rules": "r" })),
    ];

    for (case, body) in cases {
        let (status, answer) = app.post(BATCH, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{case}: {answer}");
        assert_eq!(app.state.commands.get(BATCH).await.unwrap(), None, "{case}");
    }
}

#[tokio::test]
async fn a_second_batch_is_refused_while_one_is_open_and_told_which() {
    let app = App::new().await;
    app.post(BATCH, payload(2)).await;

    let (status, body) = app.post("another-batch-01", payload(1)).await;

    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        body["message"],
        "다른 일괄 보관이 진행 중이에요. 그 결과가 나올 때까지 기다려 주세요."
    );
    assert_eq!(body["current"]["id"], BATCH);
    assert_eq!(
        app.state.commands.get("another-batch-01").await.unwrap(),
        None
    );

    // Once the first has ended, another can be sent.
    app.end_batch(BATCH, CommandState::Done, ARCHIVED, None)
        .await;
    let (status, body) = app.post("another-batch-01", payload(1)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
}

#[tokio::test]
async fn a_batch_naming_a_rule_that_is_not_suggested_now_is_refused_and_stores_nothing() {
    let app = App::new().await;
    app.quiet(&[0]).await;
    assert_eq!(app.suggested().await, [app.rule_ids[0].clone()]);

    let (status, body) = app.post(BATCH, app.of_rules(&[0, 1])).await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        body["message"],
        "보관 제안이 바뀌었어요. ‘작품 1’ 규칙은 이제 제안되지 않아요. 바뀐 제안을 확인한 뒤 다시 보관해 주세요."
    );
    assert_eq!(app.state.commands.get(BATCH).await.unwrap(), None);
}

#[tokio::test]
async fn a_rule_that_received_after_the_screen_read_the_suggestions_is_refused() {
    let app = App::new().await;
    app.quiet(&[0, 1]).await;
    let shown = app.of_rules(&[0, 1]);
    assert_eq!(app.suggested().await.len(), 2);

    // The first rule receives an item before the screen sends what it showed.
    app.record(NOW - DAY, "[SubsPlease] Work 0 - 09 (1080p)", Some(0))
        .await;
    let (status, body) = app.post(BATCH, shown).await;

    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["message"].as_str().unwrap().contains("‘작품 0’"),
        "{body}"
    );
    assert_eq!(app.state.commands.get(BATCH).await.unwrap(), None);
    // The rule that is still suggested goes on its own.
    let (status, body) = app.post(BATCH, app.of_rules(&[1])).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
}

#[tokio::test]
async fn a_batch_of_suggested_rules_and_rules_that_are_gone_is_accepted() {
    let app = App::new().await;
    app.quiet(&[0, 1]).await;
    let mut request = app.of_rules(&[0, 1]);
    request["rules"]
        .as_array_mut()
        .unwrap()
        .insert(1, json!({ "rule_id": "deleted-rule", "name": "지운 작품" }));

    let (status, body) = app.post(BATCH, request).await;

    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let stored = app.state.commands.get(BATCH).await.unwrap().unwrap();
    let stored: ArchiveBatch = serde_json::from_str(&stored.payload).unwrap();
    assert_eq!(stored.rules.len(), 3);
}

#[tokio::test]
async fn a_batch_sent_again_after_its_rule_stopped_being_suggested_is_still_the_stored_command() {
    let app = App::new().await;
    app.quiet(&[0]).await;
    let (first, _) = app.post(BATCH, app.of_rules(&[0])).await;
    assert_eq!(first, StatusCode::ACCEPTED);
    // The rule receives an item, and the screen, which did not hear the
    // answer, asks again with the same ID.
    app.record(NOW - DAY, "[SubsPlease] Work 0 - 09 (1080p)", Some(0))
        .await;
    assert!(app.suggested().await.is_empty());

    let (status, body) = app.post(BATCH, app.of_rules(&[0])).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], BATCH);
}

// --- the view --------------------------------------------------------------------------

#[tokio::test]
async fn a_running_batch_says_how_far_it_is_and_which_rule_is_in_turn() {
    let app = App::new().await;
    app.store_batch(BATCH, 3).await;
    app.state.commands.claim_next(1_050).await.unwrap();
    app.store_child(BATCH, 0, Some(archived())).await;
    app.store_child(BATCH, 1, None).await;

    let (status, body) = app.get(&format!("/api/archive-batches/{BATCH}")).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        without_results(&body),
        json!({
            "id": BATCH,
            "state": "running",
            "total": 3,
            "finished": 1,
            "current": { "rule_id": app.rule_ids[1], "name": "작품 1" },
            "failure": null,
        })
    );
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["rule_id"], app.rule_ids[0].as_str());
    assert_eq!(results[0]["name"], "작품 0");
    assert_eq!(results[0]["command"]["id"], child_id(BATCH, 0));
    assert_eq!(results[0]["command"]["kind"], "work_archive");
    assert_eq!(results[0]["command"]["outcome"]["result"], "moved");
    assert_eq!(results[0]["work_id"], app.work_id(0).await);
}

#[tokio::test]
async fn a_batch_that_archived_every_rule_says_so() {
    let app = App::new().await;
    app.store_batch(BATCH, 2).await;
    app.store_child(BATCH, 0, Some(archived())).await;
    app.store_child(BATCH, 1, Some(archived())).await;
    app.end_batch(BATCH, CommandState::Done, ARCHIVED, None)
        .await;

    let (status, body) = app.get(&format!("/api/archive-batches/{BATCH}")).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        without_results(&body),
        json!({
            "id": BATCH,
            "state": "done",
            "total": 2,
            "finished": 2,
            "current": null,
            "failure": null,
        })
    );
    assert_eq!(body["results"].as_array().unwrap().len(), 2);
    assert_eq!(body["results"][1]["work_id"], app.work_id(1).await);
}

#[tokio::test]
async fn a_batch_stopped_by_a_rules_archive_says_which_rule_and_why() {
    let app = App::new().await;
    app.store_batch(BATCH, 3).await;
    app.store_child(BATCH, 0, Some(archived())).await;
    app.store_child(
        BATCH,
        1,
        Some((
            CommandState::Failed,
            Outcome {
                result: "failed".to_owned(),
                reason: Some("Transmission이 폴더를 옮기지 못했어요.".to_owned()),
            },
        )),
    )
    .await;
    app.end_batch(
        BATCH,
        CommandState::Failed,
        &stopped_at(RULE_FAILED, 1),
        Some("Transmission이 폴더를 옮기지 못했어요."),
    )
    .await;

    let (status, body) = app.get(&format!("/api/archive-batches/{BATCH}")).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        without_results(&body),
        json!({
            "id": BATCH,
            "state": "failed",
            "total": 3,
            "finished": 1,
            "current": null,
            "failure": {
                "rule_id": app.rule_ids[1],
                "name": "작품 1",
                "reason": "Transmission이 폴더를 옮기지 못했어요.",
                "kind": "archive_failed",
                "work_id": app.work_id(1).await,
            },
        })
    );
}

#[tokio::test]
async fn a_batch_stopped_by_a_rule_that_could_not_be_archived_says_it_was_refused() {
    let app = App::new().await;
    app.store_batch(BATCH, 2).await;
    app.store_child(BATCH, 0, Some(archived())).await;
    app.end_batch(
        BATCH,
        CommandState::Failed,
        &stopped_at(REFUSED, 1),
        Some(WORK_BUSY),
    )
    .await;

    let (status, body) = app.get(&format!("/api/archive-batches/{BATCH}")).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["failure"],
        json!({
            "rule_id": app.rule_ids[1],
            "name": "작품 1",
            "reason": WORK_BUSY,
            "kind": "refused",
            "work_id": null,
        })
    );
    assert_eq!(body["finished"], 1);
}

#[tokio::test]
async fn an_unknown_id_or_a_command_of_another_kind_is_not_a_batch() {
    let app = App::new().await;
    app.store_batch(BATCH, 1).await;
    app.store_child(BATCH, 0, None).await;

    let (status, body) = app.get("/api/archive-batches/nothing-here-01").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], "not_found");

    let (status, body) = app
        .get(&format!("/api/archive-batches/{}", child_id(BATCH, 0)))
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn the_current_batch_is_the_open_one_and_there_is_none_when_none_is_open() {
    let app = App::new().await;
    let (status, body) = app.get("/api/archive-batches/current").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    app.store_batch(BATCH, 2).await;
    let (status, body) = app.get("/api/archive-batches/current").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], BATCH);
    assert_eq!(body["state"], "pending");
    assert_eq!(
        body["current"],
        json!({ "rule_id": app.rule_ids[0], "name": "작품 0" })
    );

    app.end_batch(BATCH, CommandState::Done, ARCHIVED, None)
        .await;
    let (status, body) = app.get("/api/archive-batches/current").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    // The ended batch is still read by its ID.
    let (status, _) = app.get(&format!("/api/archive-batches/{BATCH}")).await;
    assert_eq!(status, StatusCode::OK);
}
