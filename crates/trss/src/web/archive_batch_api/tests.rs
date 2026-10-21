//! The `rule_archive_batch` request through `POST /api/commands` and the
//! batch's view through `GET /api/archive-batches`. How the batch runs is
//! tested in `archive_batch/tests.rs` and `tests/it/archive_move.rs`.

use axum::{
    http::{Method, StatusCode},
    Router,
};
use serde_json::{json, Value};

use super::*;
use crate::{
    archive_batch::{
        child_id, stopped_at, ArchiveBatch, BatchRule, ARCHIVED, REFUSED, RULE_BUSY, RULE_FAILED,
    },
    web::testing,
};
use trss_collect::commands::rule_archive::{self, Direction, RuleArchive};
use trss_core::{
    commands::{Accepted, CommandState, NewCommand, Outcome},
    Db,
};

const BATCH: &str = "batch-0123456789";

struct App {
    state: AppState,
    router: Router,
}

impl App {
    fn new() -> App {
        let state = AppState::new(Db::open_blocking(":memory:").unwrap());
        let router = testing::api(&state);
        App { state, router }
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

    /// Stores a batch of rules `r0`, `r1`, ... directly, as `POST` would.
    async fn store_batch(&self, id: &str, rules: usize) {
        let batch = batch(rules);
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

    /// Stores the command the batch `id` made for its `n`th rule, ended as
    /// `end` says or left pending.
    async fn store_child(&self, id: &str, n: usize, end: Option<(CommandState, Outcome)>) {
        let payload = RuleArchive {
            rule_id: format!("r{n}"),
            direction: Direction::Archive,
            receive: Vec::new(),
        };
        let child = child_id(id, n);
        self.state
            .commands
            .accept(
                NewCommand {
                    id: child.clone(),
                    kind: rule_archive::KIND.to_owned(),
                    payload: payload.canonical(),
                    subject: Some(payload.subject()),
                },
                1_100,
            )
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
    let app = App::new();

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
    let app = App::new();
    let (first, one) = app.post(BATCH, payload(2)).await;

    let (second, two) = app.post(BATCH, payload(2)).await;

    assert_eq!((first, second), (StatusCode::ACCEPTED, StatusCode::OK));
    assert_eq!(one, two);
}

#[tokio::test]
async fn the_same_id_with_other_rules_is_refused() {
    let app = App::new();
    app.post(BATCH, payload(2)).await;

    let (status, body) = app.post(BATCH, payload(3)).await;

    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["current"]["id"], BATCH);
}

#[tokio::test]
async fn a_batch_without_rules_or_with_a_rule_twice_or_with_a_field_too_long_is_refused_and_stores_nothing(
) {
    let app = App::new();
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
    let app = App::new();
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
    let app = App::new();
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
    let app = App::new();
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

// --- the view --------------------------------------------------------------------------

#[tokio::test]
async fn a_running_batch_says_how_far_it_is_and_which_rule_is_in_turn() {
    let app = App::new();
    app.store_batch(BATCH, 3).await;
    app.state.commands.claim_next(1_050).await.unwrap();
    app.store_child(BATCH, 0, Some(archived())).await;
    app.store_child(BATCH, 1, None).await;

    let (status, body) = app.get(&format!("/api/archive-batches/{BATCH}")).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({
            "id": BATCH,
            "state": "running",
            "total": 3,
            "finished": 1,
            "current": { "rule_id": "r1", "name": "작품 1" },
            "failure": null,
        })
    );
}

#[tokio::test]
async fn a_batch_that_archived_every_rule_says_so() {
    let app = App::new();
    app.store_batch(BATCH, 2).await;
    app.store_child(BATCH, 0, Some(archived())).await;
    app.store_child(BATCH, 1, Some(archived())).await;
    app.end_batch(BATCH, CommandState::Done, ARCHIVED, None)
        .await;

    let (status, body) = app.get(&format!("/api/archive-batches/{BATCH}")).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body,
        json!({
            "id": BATCH,
            "state": "done",
            "total": 2,
            "finished": 2,
            "current": null,
            "failure": null,
        })
    );
}

#[tokio::test]
async fn a_batch_stopped_by_a_rules_archive_says_which_rule_and_why() {
    let app = App::new();
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
        body,
        json!({
            "id": BATCH,
            "state": "failed",
            "total": 3,
            "finished": 1,
            "current": null,
            "failure": {
                "rule_id": "r1",
                "name": "작품 1",
                "reason": "Transmission이 폴더를 옮기지 못했어요.",
                "kind": "archive_failed",
            },
        })
    );
}

#[tokio::test]
async fn a_batch_stopped_by_a_rule_that_could_not_be_started_says_it_was_refused() {
    let app = App::new();
    app.store_batch(BATCH, 2).await;
    app.store_child(BATCH, 0, Some(archived())).await;
    app.end_batch(
        BATCH,
        CommandState::Failed,
        &stopped_at(REFUSED, 1),
        Some(RULE_BUSY),
    )
    .await;

    let (status, body) = app.get(&format!("/api/archive-batches/{BATCH}")).await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["failure"],
        json!({
            "rule_id": "r1",
            "name": "작품 1",
            "reason": RULE_BUSY,
            "kind": "refused",
        })
    );
    assert_eq!(body["finished"], 1);
}

#[tokio::test]
async fn an_unknown_id_or_a_command_of_another_kind_is_not_a_batch() {
    let app = App::new();
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
    let app = App::new();
    let (status, body) = app.get("/api/archive-batches/current").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    app.store_batch(BATCH, 2).await;
    let (status, body) = app.get("/api/archive-batches/current").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], BATCH);
    assert_eq!(body["state"], "pending");
    assert_eq!(
        body["current"],
        json!({ "rule_id": "r0", "name": "작품 0" })
    );

    app.end_batch(BATCH, CommandState::Done, ARCHIVED, None)
        .await;
    let (status, body) = app.get("/api/archive-batches/current").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    // The ended batch is still read by its ID.
    let (status, _) = app.get(&format!("/api/archive-batches/{BATCH}")).await;
    assert_eq!(status, StatusCode::OK);
}
