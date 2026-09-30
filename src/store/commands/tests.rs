use super::*;

async fn store() -> (tempfile::TempDir, Db, CommandStore) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    let commands = CommandStore::new(db.clone());
    (dir, db, commands)
}

fn new(id: &str, payload: &str, subject: Option<&str>) -> NewCommand {
    NewCommand {
        id: id.to_owned(),
        kind: "receive_once".to_owned(),
        payload: payload.to_owned(),
        subject: subject.map(str::to_owned),
    }
}

fn done() -> Outcome {
    Outcome {
        result: "received".to_owned(),
        reason: None,
    }
}

#[tokio::test]
async fn a_new_command_is_stored_pending() {
    let (_dir, _db, commands) = store().await;

    let Accepted::Created(command) = commands
        .accept(new("cmd-1", r#"{"item_id":7}"#, Some("7")), 1_000)
        .await
        .unwrap()
    else {
        panic!("expected a new command");
    };

    assert_eq!(command.id, "cmd-1");
    assert_eq!(command.state, CommandState::Pending);
    assert_eq!(command.attempts, 0);
    assert_eq!((command.created_at, command.updated_at), (1_000, 1_000));
    assert_eq!(command.finished_at, None);
    assert_eq!(command.outcome, None);
    assert_eq!(commands.get("cmd-1").await.unwrap(), Some(command));
    assert_eq!(commands.get("cmd-2").await.unwrap(), None);
}

#[tokio::test]
async fn the_same_id_with_the_same_content_returns_the_stored_command() {
    let (_dir, _db, commands) = store().await;
    let first = commands
        .accept(new("cmd-1", r#"{"item_id":7}"#, Some("7")), 1_000)
        .await
        .unwrap();
    let Accepted::Created(created) = first else {
        panic!("expected a new command");
    };

    // Delivered again later, even after the worker took it and finished it.
    commands.claim_next(1_500).await.unwrap().unwrap();
    commands
        .finish("cmd-1", CommandState::Done, done(), 2_000)
        .await
        .unwrap();
    let again = commands
        .accept(new("cmd-1", r#"{"item_id":7}"#, Some("7")), 3_000)
        .await
        .unwrap();

    let Accepted::Existing(existing) = again else {
        panic!("expected the stored command, got {again:?}");
    };
    assert_eq!(existing.id, created.id);
    assert_eq!(existing.state, CommandState::Done);
    assert_eq!(existing.outcome, Some(done()));
    assert_eq!(existing.created_at, 1_000, "the repeat stores nothing new");
}

#[tokio::test]
async fn the_same_id_with_other_content_is_refused_and_changes_nothing() {
    let (_dir, _db, commands) = store().await;
    commands
        .accept(new("cmd-1", r#"{"item_id":7}"#, Some("7")), 1_000)
        .await
        .unwrap();

    let other_payload = commands
        .accept(new("cmd-1", r#"{"item_id":8}"#, Some("8")), 2_000)
        .await
        .unwrap();
    let Accepted::Mismatch(stored) = other_payload else {
        panic!("expected a mismatch, got {other_payload:?}");
    };
    assert_eq!(stored.payload, r#"{"item_id":7}"#);

    let mut other_kind = new("cmd-1", r#"{"item_id":7}"#, Some("7"));
    other_kind.kind = "something_else".to_owned();
    assert!(matches!(
        commands.accept(other_kind, 2_000).await.unwrap(),
        Accepted::Mismatch(_)
    ));

    let stored = commands.get("cmd-1").await.unwrap().unwrap();
    assert_eq!(stored.payload, r#"{"item_id":7}"#);
    assert_eq!(stored.kind, "receive_once");
}

#[tokio::test]
async fn a_second_open_command_for_the_same_subject_is_refused() {
    let (_dir, _db, commands) = store().await;
    commands
        .accept(new("cmd-1", r#"{"item_id":7}"#, Some("7")), 1_000)
        .await
        .unwrap();

    let second = commands
        .accept(new("cmd-2", r#"{"item_id":7,"x":1}"#, Some("7")), 1_100)
        .await
        .unwrap();
    let Accepted::Busy(open) = second else {
        panic!("expected busy, got {second:?}");
    };
    assert_eq!(open.id, "cmd-1");
    assert_eq!(commands.get("cmd-2").await.unwrap(), None);

    // A different subject is independent, and so is the same subject once the
    // first command has ended.
    assert!(matches!(
        commands
            .accept(new("cmd-3", r#"{"item_id":8}"#, Some("8")), 1_200)
            .await
            .unwrap(),
        Accepted::Created(_)
    ));
    commands.claim_next(1_300).await.unwrap().unwrap();
    commands
        .finish("cmd-1", CommandState::Failed, done(), 1_400)
        .await
        .unwrap();
    assert!(matches!(
        commands
            .accept(new("cmd-2", r#"{"item_id":7}"#, Some("7")), 1_500)
            .await
            .unwrap(),
        Accepted::Created(_)
    ));
}

#[tokio::test]
async fn deliveries_at_the_same_time_store_one_command() {
    let (_dir, _db, commands) = store().await;

    let deliveries = (0..8).map(|_| {
        let commands = commands.clone();
        tokio::spawn(async move {
            commands
                .accept(new("cmd-1", r#"{"item_id":7}"#, Some("7")), 1_000)
                .await
                .unwrap()
        })
    });
    let mut created = 0;
    for delivery in deliveries.collect::<Vec<_>>() {
        if matches!(delivery.await.unwrap(), Accepted::Created(_)) {
            created += 1;
        }
    }

    assert_eq!(created, 1);
}

#[tokio::test]
async fn the_first_accepted_open_command_is_claimed_first_and_becomes_running() {
    let (_dir, _db, commands) = store().await;
    // The order is the order of acceptance, whatever the clock said.
    for (id, at) in [("a", 3_000), ("b", 1_000), ("c", 2_000)] {
        commands.accept(new(id, id, Some(id)), at).await.unwrap();
    }

    assert!(commands.has_open().await.unwrap());
    let first = commands.claim_next(5_000).await.unwrap().unwrap();
    assert_eq!(first.id, "a");
    assert_eq!(first.state, CommandState::Running);
    assert_eq!(first.attempts, 1);
    assert_eq!(first.updated_at, 5_000);

    commands
        .finish("a", CommandState::Done, done(), 5_100)
        .await
        .unwrap();
    assert_eq!(commands.claim_next(5_200).await.unwrap().unwrap().id, "b");
    commands
        .finish("b", CommandState::Done, done(), 5_300)
        .await
        .unwrap();
    assert_eq!(commands.claim_next(5_400).await.unwrap().unwrap().id, "c");
    commands
        .finish("c", CommandState::Done, done(), 5_500)
        .await
        .unwrap();
    assert!(commands.claim_next(5_600).await.unwrap().is_none());
    assert!(!commands.has_open().await.unwrap());
}

#[tokio::test]
async fn a_command_left_running_is_handed_out_again() {
    let (_dir, _db, commands) = store().await;
    commands
        .accept(new("cmd-1", "{}", Some("7")), 1_000)
        .await
        .unwrap();

    // A worker started it and died: the command stays running.
    let first = commands.claim_next(2_000).await.unwrap().unwrap();
    assert_eq!((first.state, first.attempts), (CommandState::Running, 1));

    let again = commands.claim_next(3_000).await.unwrap().unwrap();
    assert_eq!(again.id, "cmd-1");
    assert_eq!((again.state, again.attempts), (CommandState::Running, 2));
}

#[tokio::test]
async fn a_command_that_keeps_ending_its_worker_is_given_up() {
    let (_dir, _db, commands) = store().await;
    commands
        .accept(new("stuck", "{}", Some("7")), 1_000)
        .await
        .unwrap();
    commands
        .accept(new("next", "{}", Some("8")), 1_100)
        .await
        .unwrap();

    for attempt in 1..=MAX_ATTEMPTS {
        let claimed = commands.claim_next(2_000).await.unwrap().unwrap();
        assert_eq!((claimed.id.as_str(), claimed.attempts), ("stuck", attempt));
    }

    // The next claim gives up on it and moves on.
    let claimed = commands.claim_next(3_000).await.unwrap().unwrap();
    assert_eq!(claimed.id, "next");
    let stuck = commands.get("stuck").await.unwrap().unwrap();
    assert_eq!(stuck.state, CommandState::Failed);
    assert!(stuck.outcome.unwrap().reason.is_some());
    assert_eq!(stuck.finished_at, Some(3_000));
}

#[tokio::test]
async fn a_command_ends_once() {
    let (_dir, _db, commands) = store().await;
    commands
        .accept(new("cmd-1", "{}", None), 1_000)
        .await
        .unwrap();
    commands.claim_next(1_100).await.unwrap().unwrap();

    let failed = Outcome {
        result: "add_failed".to_owned(),
        reason: Some("Transmission에 넣지 못했어요.".to_owned()),
    };
    assert!(commands
        .finish("cmd-1", CommandState::Failed, failed.clone(), 1_200)
        .await
        .unwrap());
    assert!(!commands
        .finish("cmd-1", CommandState::Done, done(), 1_300)
        .await
        .unwrap());

    let stored = commands.get("cmd-1").await.unwrap().unwrap();
    assert_eq!(stored.state, CommandState::Failed);
    assert_eq!(stored.outcome, Some(failed));
    assert_eq!(stored.finished_at, Some(1_200));
    assert!(commands.claim_next(1_400).await.unwrap().is_none());
}

#[tokio::test]
async fn open_commands_are_found_by_subject() {
    let (_dir, _db, commands) = store().await;
    commands
        .accept(new("cmd-1", "{}", Some("7")), 1_000)
        .await
        .unwrap();
    commands
        .accept(new("cmd-2", "{}", Some("8")), 1_100)
        .await
        .unwrap();
    commands.claim_next(1_200).await.unwrap().unwrap();
    commands
        .finish("cmd-1", CommandState::Done, done(), 1_300)
        .await
        .unwrap();

    let open = commands
        .open_for_subjects(
            "receive_once",
            vec!["7".to_owned(), "8".to_owned(), "9".to_owned()],
        )
        .await
        .unwrap();

    assert_eq!(open.len(), 1);
    assert_eq!(open["8"].id, "cmd-2");
    assert!(commands
        .open_for_subjects("other_kind", vec!["8".to_owned()])
        .await
        .unwrap()
        .is_empty());
    assert!(commands
        .open_for_subjects("receive_once", vec![])
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_command_survives_reopening_the_database() {
    let (dir, db, commands) = store().await;
    commands
        .accept(new("cmd-1", r#"{"item_id":7}"#, Some("7")), 1_000)
        .await
        .unwrap();
    commands.claim_next(1_100).await.unwrap().unwrap();
    drop((db, commands));

    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    let commands = CommandStore::new(db);
    let stored = commands.get("cmd-1").await.unwrap().unwrap();
    assert_eq!(stored.state, CommandState::Running);
    assert_eq!(
        commands.claim_next(2_000).await.unwrap().unwrap().attempts,
        2
    );
}

#[tokio::test]
async fn only_started_and_unended_commands_count_as_running() {
    let (_dir, _db, commands) = store().await;
    for (id, subject) in [("cmd-1", "1"), ("cmd-2", "2")] {
        commands
            .accept(new(id, r#"{"item_id":1}"#, Some(subject)), 1_000)
            .await
            .unwrap();
    }
    assert_eq!(commands.running_count().await.unwrap(), 0, "pending");

    let claimed = commands.claim_next(1_100).await.unwrap().unwrap();
    assert_eq!(commands.running_count().await.unwrap(), 1);

    commands
        .finish(&claimed.id, CommandState::Done, done(), 1_300)
        .await
        .unwrap();
    assert_eq!(commands.running_count().await.unwrap(), 0, "ended");
}

#[tokio::test]
async fn unconfirmed_adds_are_counted_from_a_point_in_time() {
    let (_dir, _db, commands) = store().await;
    for (id, subject) in [("cmd-1", "1"), ("cmd-2", "2"), ("cmd-3", "3")] {
        commands
            .accept(new(id, r#"{"item_id":1}"#, Some(subject)), 1_000)
            .await
            .unwrap();
    }
    let failed = Outcome {
        result: "add_failed".to_owned(),
        reason: Some("no answer".to_owned()),
    };
    let first = commands.claim_next(1_100).await.unwrap().unwrap();
    commands
        .finish_with_unconfirmed_add(&first.id, CommandState::Failed, failed.clone(), 1_200)
        .await
        .unwrap();
    let second = commands.claim_next(1_300).await.unwrap().unwrap();
    commands
        .finish(&second.id, CommandState::Failed, failed.clone(), 1_400)
        .await
        .unwrap();
    let third = commands.claim_next(1_500).await.unwrap().unwrap();
    commands
        .finish_with_unconfirmed_add(&third.id, CommandState::Failed, failed, 1_600)
        .await
        .unwrap();

    assert_eq!(commands.unconfirmed_adds_since(None).await.unwrap(), 2);
    assert_eq!(
        commands.unconfirmed_adds_since(Some(1_200)).await.unwrap(),
        2
    );
    assert_eq!(
        commands.unconfirmed_adds_since(Some(1_201)).await.unwrap(),
        1
    );
    assert_eq!(
        commands.unconfirmed_adds_since(Some(1_601)).await.unwrap(),
        0
    );
}

#[tokio::test]
async fn an_unanswered_add_noted_on_a_running_command_stays_through_the_next_start_and_a_give_up() {
    let (_dir, _db, commands) = store().await;
    commands
        .accept(new("cmd-1", r#"{"item_id":1}"#, Some("1")), 1_000)
        .await
        .unwrap();
    assert!(
        !commands.note_unconfirmed_add("cmd-1", 1_050).await.unwrap(),
        "a waiting command is not running"
    );

    let first = commands.claim_next(1_100).await.unwrap().unwrap();
    assert!(!first.add_unconfirmed);
    assert!(commands.note_unconfirmed_add("cmd-1", 1_200).await.unwrap());
    // Still running, so not yet counted among the ended ones.
    assert_eq!(commands.unconfirmed_adds_since(None).await.unwrap(), 0);

    let again = commands.claim_next(1_300).await.unwrap().unwrap();
    assert!(again.add_unconfirmed);
    for now in 1_400..(1_400 + MAX_ATTEMPTS - 2) {
        commands.claim_next(now).await.unwrap().unwrap();
    }
    assert!(commands.claim_next(2_000).await.unwrap().is_none());

    let given_up = commands.get("cmd-1").await.unwrap().unwrap();
    assert_eq!(given_up.state, CommandState::Failed);
    assert!(given_up.add_unconfirmed);
    assert_eq!(
        commands.unconfirmed_adds_since(Some(2_000)).await.unwrap(),
        1
    );
}
