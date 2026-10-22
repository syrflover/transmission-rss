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
async fn a_write_with_the_command_is_stored_with_it_or_not_at_all() {
    let (_dir, db, commands) = store().await;
    db.run(|c| {
        c.execute_batch("CREATE TABLE marks (name TEXT NOT NULL)")
            .map_err(DbError::Sqlite)
    })
    .await
    .unwrap();
    let mark = |name: &'static str, refuse: bool| {
        move |tx: &rusqlite::Transaction<'_>| {
            tx.execute("INSERT INTO marks (name) VALUES (?1)", [name])
                .unwrap();
            if refuse {
                Err(name)
            } else {
                Ok(())
            }
        }
    };
    let marks = || async {
        db.run(|c| {
            let read = || -> rusqlite::Result<Vec<String>> {
                let mut stmt = c.prepare("SELECT name FROM marks ORDER BY rowid")?;
                let names = stmt.query_map([], |r| r.get(0))?;
                names.collect()
            };
            read().map_err(DbError::Sqlite)
        })
        .await
        .unwrap()
    };

    // A refusal rolls back what the write did and stores no command.
    let refused = commands
        .accept_with(new("cmd-1", "{}", Some("7")), 1_000, mark("refused", true))
        .await
        .unwrap();
    assert_eq!(refused.unwrap_err(), "refused");
    assert!(marks().await.is_empty());
    assert_eq!(commands.get("cmd-1").await.unwrap(), None);

    let created = commands
        .accept_with(new("cmd-1", "{}", Some("7")), 1_000, mark("first", false))
        .await
        .unwrap();
    assert!(matches!(created, Ok(Accepted::Created(_))), "{created:?}");
    assert_eq!(marks().await, ["first"]);

    // A known ID and an open command of the subject do not run the write.
    for (id, payload) in [("cmd-1", "{}"), ("cmd-1", "{\"x\":1}"), ("cmd-2", "{}")] {
        let accepted = commands
            .accept_with(new(id, payload, Some("7")), 2_000, mark("again", true))
            .await
            .unwrap();
        assert!(
            matches!(
                accepted,
                Ok(Accepted::Existing(_) | Accepted::Mismatch(_) | Accepted::Busy(_))
            ),
            "{id} {payload}: {accepted:?}"
        );
    }
    assert_eq!(marks().await, ["first"]);
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
async fn a_claim_passes_over_the_excluded_commands_without_giving_them_up() {
    let (_dir, _db, commands) = store().await;
    for (id, subject) in [("a", "1"), ("b", "2"), ("c", "3")] {
        commands
            .accept(new(id, "{}", Some(subject)), 1_000)
            .await
            .unwrap();
    }
    // `a` is running here; `b` was left for a later look after its last start.
    commands.claim_next(2_000).await.unwrap().unwrap();
    for _ in 0..MAX_ATTEMPTS {
        commands
            .claim_next_excluding(2_000, vec!["a".into()])
            .await
            .unwrap()
            .unwrap();
    }

    let excluded = vec!["a".to_owned(), "b".to_owned()];
    let claimed = commands
        .claim_next_excluding(3_000, excluded.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!((claimed.id.as_str(), claimed.attempts), ("c", 1));
    assert!(commands
        .claim_next_excluding(3_000, vec!["a".into(), "b".into(), "c".into()])
        .await
        .unwrap()
        .is_none());
    // Passed over, `b` is still open; the next claim that looks at it gives it up.
    let b = commands.get("b").await.unwrap().unwrap();
    assert_eq!((b.state, b.attempts), (CommandState::Running, MAX_ATTEMPTS));
    assert!(commands
        .claim_next_excluding(4_000, vec!["a".into(), "c".into()])
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        commands.get("b").await.unwrap().unwrap().state,
        CommandState::Failed
    );
}

#[tokio::test]
async fn the_running_count_leaves_out_the_excluded_commands() {
    let (_dir, _db, commands) = store().await;
    for (id, subject) in [("a", "1"), ("b", "2"), ("c", "3")] {
        commands
            .accept(new(id, "{}", Some(subject)), 1_000)
            .await
            .unwrap();
    }
    commands.claim_next(2_000).await.unwrap().unwrap();
    commands
        .claim_next_excluding(2_000, vec!["a".into()])
        .await
        .unwrap()
        .unwrap();

    assert_eq!(commands.running_count().await.unwrap(), 2);
    assert_eq!(
        commands
            .running_count_excluding(vec!["a".into(), "c".into()])
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        commands
            .running_count_excluding(vec!["a".into(), "b".into()])
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn running_commands_are_counted_without_the_excluded_kinds() {
    let (_dir, _db, commands) = store().await;
    for (id, kind, subject) in [("a", "receive_once", "1"), ("b", "other_kind", "2")] {
        let mut command = new(id, "{}", Some(subject));
        command.kind = kind.to_owned();
        commands.accept(command, 1_000).await.unwrap();
    }
    commands.claim_next(2_000).await.unwrap().unwrap();
    commands
        .claim_next_excluding(2_000, vec!["a".into()])
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        commands
            .running_count_excluding_kinds(Vec::new(), vec!["other_kind".into()])
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        commands
            .running_count_excluding_kinds(vec!["a".into()], vec!["other_kind".into()])
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        commands
            .running_count_excluding_kinds(Vec::new(), Vec::new())
            .await
            .unwrap(),
        2
    );
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

#[tokio::test]
async fn the_first_name_noted_on_a_running_command_stays_through_its_next_start() {
    let (_dir, _db, commands) = store().await;
    commands
        .accept(new("cmd-1", r#"{"item_id":1}"#, Some("1")), 1_000)
        .await
        .unwrap();
    assert_eq!(
        commands.note_original_name("cmd-1", "a.mkv").await.unwrap(),
        None,
        "a waiting command is not running"
    );

    let first = commands.claim_next(1_100).await.unwrap().unwrap();
    assert_eq!(first.original_name, None);
    assert_eq!(
        commands.note_original_name("cmd-1", "a.mkv").await.unwrap(),
        Some("a.mkv".to_owned())
    );
    // A later name does not replace it.
    assert_eq!(
        commands.note_original_name("cmd-1", "b.mkv").await.unwrap(),
        Some("a.mkv".to_owned())
    );
    let unchanged = commands.get("cmd-1").await.unwrap().unwrap();
    assert_eq!(unchanged.updated_at, first.updated_at);

    let again = commands.claim_next(1_300).await.unwrap().unwrap();
    assert_eq!(again.original_name.as_deref(), Some("a.mkv"));
}

/// A store announcing on a channel, and what a listener on it has heard so far.
async fn announcing_store() -> (tempfile::TempDir, CommandStore, Listener) {
    let (dir, db, _) = store().await;
    let changes = Changes::default();
    let listener = Listener(changes.subscribe());
    (dir, CommandStore::new(db).announcing(changes), listener)
}

struct Listener(tokio::sync::broadcast::Receiver<crate::changes::Change>);

impl Listener {
    /// The IDs of the commands announced since the last call, each with its
    /// kind checked.
    fn heard(&mut self) -> Vec<String> {
        let mut ids = Vec::new();
        while let Ok(change) = self.0.try_recv() {
            assert_eq!(change.kind, ChangeKind::Command);
            ids.push(change.id.expect("a command change has an ID"));
        }
        ids
    }
}

#[tokio::test]
async fn a_command_is_announced_when_it_is_accepted_started_and_ended() {
    let (_dir, commands, mut listener) = announcing_store().await;

    commands
        .accept(new("cmd-1", "{}", None), 1_000)
        .await
        .unwrap();
    assert_eq!(listener.heard(), ["cmd-1"]);

    commands.claim_next(1_100).await.unwrap().unwrap();
    assert_eq!(listener.heard(), ["cmd-1"]);

    assert!(commands
        .finish("cmd-1", CommandState::Done, done(), 1_200)
        .await
        .unwrap());
    assert_eq!(listener.heard(), ["cmd-1"]);
}

#[tokio::test]
async fn a_command_ended_with_an_unconfirmed_add_is_announced_too() {
    let (_dir, commands, mut listener) = announcing_store().await;
    commands
        .accept(new("cmd-1", "{}", None), 1_000)
        .await
        .unwrap();
    commands.claim_next(1_100).await.unwrap().unwrap();
    listener.heard();

    assert!(commands
        .finish_with_unconfirmed_add("cmd-1", CommandState::Failed, done(), 1_200)
        .await
        .unwrap());

    assert_eq!(listener.heard(), ["cmd-1"]);
}

#[tokio::test]
async fn a_refused_or_repeated_accept_announces_nothing() {
    let (_dir, commands, mut listener) = announcing_store().await;
    commands
        .accept(new("cmd-1", "{}", Some("7")), 1_000)
        .await
        .unwrap();
    listener.heard();

    // The same delivery again, the same ID with other content, and a second
    // open command for the subject.
    commands
        .accept(new("cmd-1", "{}", Some("7")), 1_100)
        .await
        .unwrap();
    commands
        .accept(new("cmd-1", r#"{"other":1}"#, Some("7")), 1_200)
        .await
        .unwrap();
    commands
        .accept(new("cmd-2", "{}", Some("7")), 1_300)
        .await
        .unwrap();

    assert!(listener.heard().is_empty());
}

#[tokio::test]
async fn a_claim_with_nothing_to_hand_out_and_a_finish_that_ended_nothing_announce_nothing() {
    let (_dir, commands, mut listener) = announcing_store().await;
    assert!(commands.claim_next(1_000).await.unwrap().is_none());
    assert!(!commands
        .finish("nobody", CommandState::Done, done(), 1_100)
        .await
        .unwrap());

    commands
        .accept(new("cmd-1", "{}", None), 1_200)
        .await
        .unwrap();
    commands.claim_next(1_300).await.unwrap().unwrap();
    commands
        .finish("cmd-1", CommandState::Done, done(), 1_400)
        .await
        .unwrap();
    listener.heard();

    // Ended already: a second end changes nothing.
    assert!(!commands
        .finish("cmd-1", CommandState::Failed, done(), 1_500)
        .await
        .unwrap());

    assert!(listener.heard().is_empty());
}

#[tokio::test]
async fn a_command_given_up_on_the_way_is_announced_beside_the_one_claimed_instead() {
    let (_dir, commands, mut listener) = announcing_store().await;
    commands
        .accept(new("stuck", "{}", Some("7")), 1_000)
        .await
        .unwrap();
    commands
        .accept(new("next", "{}", Some("8")), 1_100)
        .await
        .unwrap();
    for _ in 0..MAX_ATTEMPTS {
        commands.claim_next(2_000).await.unwrap().unwrap();
    }
    listener.heard();

    let claimed = commands.claim_next(3_000).await.unwrap().unwrap();

    assert_eq!(claimed.id, "next");
    assert_eq!(listener.heard(), ["stuck", "next"]);
}

#[tokio::test]
async fn noting_what_no_screen_shows_announces_nothing() {
    let (_dir, commands, mut listener) = announcing_store().await;
    commands
        .accept(new("cmd-1", "{}", None), 1_000)
        .await
        .unwrap();
    commands.claim_next(1_100).await.unwrap().unwrap();
    listener.heard();

    commands.note_unconfirmed_add("cmd-1", 1_200).await.unwrap();
    commands.note_original_name("cmd-1", "name").await.unwrap();

    assert!(listener.heard().is_empty());
}

#[tokio::test]
async fn many_commands_are_read_by_id_in_one_query() {
    let (_dir, _db, commands) = store().await;
    for n in 1..=3 {
        commands
            .accept(
                new(&format!("cmd-{n}"), &format!("{{\"n\":{n}}}"), None),
                1_000,
            )
            .await
            .unwrap();
    }

    let found = commands
        .get_many(vec!["cmd-3".into(), "cmd-1".into(), "cmd-9".into()])
        .await
        .unwrap();

    let mut ids: Vec<_> = found.keys().cloned().collect();
    ids.sort();
    assert_eq!(ids, ["cmd-1", "cmd-3"]);
    assert_eq!(
        found["cmd-3"],
        commands.get("cmd-3").await.unwrap().unwrap()
    );
    assert!(commands.get_many(Vec::new()).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_subscription_hears_what_the_store_announces() {
    let (_dir, commands, _listener) = announcing_store().await;
    let mut heard = commands.subscribe();

    commands
        .accept(new("cmd-1", "{}", None), 1_000)
        .await
        .unwrap();

    let change = heard.try_recv().expect("the accept was announced");
    assert_eq!(
        (change.kind, change.id.as_deref()),
        (ChangeKind::Command, Some("cmd-1"))
    );
}
