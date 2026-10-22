//! The batch's own rules: the order, the stop, the refusal, the skip, the
//! restart and the wait. A fake commands' loop (`Loop`) stands in for the
//! worker: it takes the `work_archive` commands the batch stores with each
//! rule's deletion and ends them as the test says. What archiving one rule
//! does (the deletion with the stored move, which rule keeps the folder) is
//! tested where it lives (`trss-collect` `work_archive/tests.rs`), and the
//! batch with the real worker in `tests/it/archive_move.rs`.

use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::json;
use tokio::task::JoinHandle;

use trss_collect::store::channels::{ChannelInput, ChannelStore, Rule, RuleInput};
use trss_core::{
    changes::{Change, Changes},
    commands::{Accepted, NewCommand},
    Db,
};
use trss_library::discovery::{Scan, ScannedWork, WorkRead};

use super::*;

/// Short enough that a test which depends on the fallback read is quick, long
/// enough that one which depends on an event does not pass by the fallback.
const FALLBACK: Duration = Duration::from_millis(25);
const NOW: i64 = 1_000;
const COLLECT: &str = "/media/current";

struct Scene {
    db: Db,
    commands: CommandStore,
    channels: ChannelStore,
    settings: SettingsStore,
    library: LibraryStore,
    wakes: Wakes,
    clock: Clock,
    listener: tokio::sync::broadcast::Receiver<Change>,
    rules: Vec<Rule>,
    rule_ids: Vec<String>,
}

impl Scene {
    /// `rules` rules on one channel saving into `Work {n}/Season 01`, whose
    /// work folders the library knows below the collect folder.
    async fn new(rules: usize) -> Scene {
        let directories: Vec<String> = (0..rules).map(|n| format!("Work {n}/Season 01")).collect();
        Scene::with(&directories.iter().map(String::as_str).collect::<Vec<_>>()).await
    }

    /// A rule `Work {n}` saving into each of `directories`.
    async fn with(directories: &[&str]) -> Scene {
        let db = Db::open_blocking(":memory:").unwrap();
        let changes = Changes::default();
        let channels = ChannelStore::new(db.clone());
        let settings = SettingsStore::new(db.clone());
        settings
            .put_collection(0, COLLECT.into(), Some("/media/archive".into()))
            .await
            .unwrap();
        let library = LibraryStore::new(db.clone());
        let works = (0..directories.len())
            .map(|n| {
                WorkRead::Read(ScannedWork {
                    dir_name: format!("Work {n}"),
                    seasons: BTreeSet::from([1]),
                    files: Vec::new(),
                    unrecognized: Vec::new(),
                })
            })
            .collect();
        library
            .add_folder(COLLECT.into(), Scan { works }, NOW, &[])
            .await
            .unwrap();
        let channel = channels
            .create_channel(ChannelInput::new("https://feed.example/rss"))
            .await
            .unwrap();
        let mut made = Vec::new();
        for (n, directory) in directories.iter().enumerate() {
            let rule = channels
                .create_rule(
                    &channel.id,
                    RuleInput {
                        r#match: Some(format!("Work {n}")),
                        directory: (*directory).to_owned(),
                        ..RuleInput::default()
                    },
                )
                .await
                .unwrap();
            made.push(rule);
        }
        Scene {
            listener: changes.subscribe(),
            commands: CommandStore::new(db.clone()).announcing(changes),
            db,
            rule_ids: made.iter().map(|rule| rule.id.clone()).collect(),
            rules: made,
            channels,
            settings,
            library,
            wakes: Wakes::default(),
            clock: Arc::new(|| NOW),
        }
    }

    fn ctx(&self) -> Ctx<'_> {
        Ctx {
            commands: &self.commands,
            channels: &self.channels,
            settings: &self.settings,
            wakes: &self.wakes,
            clock: &self.clock,
            reread_every: FALLBACK,
        }
    }

    /// The work folders of the first `n` rules.
    fn works(&self, n: std::ops::Range<usize>) -> Vec<String> {
        n.map(|n| format!("Work {n}")).collect()
    }

    /// Whether the `n`th rule is still there.
    async fn has_rule(&self, n: usize) -> bool {
        self.channels
            .get_rule(&self.rule_ids[n])
            .await
            .unwrap()
            .is_some()
    }

    /// The library's work of the `n`th rule's work folder.
    async fn work_id(&self, n: usize) -> String {
        self.library
            .work_in_folder(COLLECT, &format!("Work {n}"))
            .await
            .unwrap()
            .unwrap()
    }

    /// The batch of the first `n` rules, stored as the web would.
    async fn batch(&self, id: &str, n: usize) -> Command {
        let batch = ArchiveBatch {
            rules: self
                .rule_ids
                .iter()
                .take(n)
                .enumerate()
                .map(|(i, rule_id)| BatchRule {
                    rule_id: rule_id.clone(),
                    name: format!("작품 {i}"),
                })
                .collect(),
        };
        let Accepted::Created(command) = self
            .commands
            .accept(
                NewCommand {
                    id: id.to_owned(),
                    kind: KIND.to_owned(),
                    payload: batch.canonical(),
                    subject: Some(batch.subject()),
                },
                NOW,
            )
            .await
            .unwrap()
        else {
            panic!("the batch is new");
        };
        // The worker claims it before it runs.
        self.commands
            .claim_next_excluding(NOW, Vec::new())
            .await
            .unwrap();
        command
    }

    async fn run(&self, batch: &Command) -> Result<Finished, Retry> {
        run(&self.ctx(), batch, &CancellationToken::new()).await
    }

    /// [`run`] in a task of its own, for a test that looks at the store while
    /// the batch waits.
    fn spawn_run(
        &self,
        batch: &Command,
        cancel: &CancellationToken,
    ) -> JoinHandle<Result<Finished, Retry>> {
        let (commands, channels, settings, wakes, clock) = (
            self.commands.clone(),
            self.channels.clone(),
            self.settings.clone(),
            self.wakes.clone(),
            self.clock.clone(),
        );
        let (batch, cancel) = (batch.clone(), cancel.clone());
        tokio::spawn(async move {
            let ctx = Ctx {
                commands: &commands,
                channels: &channels,
                settings: &settings,
                wakes: &wakes,
                clock: &clock,
                reread_every: FALLBACK,
            };
            run(&ctx, &batch, &cancel).await
        })
    }

    /// What an earlier run of `batch` did with its `n`th rule: deleted it with
    /// the move of its work folder, in the state `end` says (still open when
    /// `None`).
    async fn store_child(&self, batch: &Command, n: usize, end: Option<(CommandState, Outcome)>) {
        let id = child_id(&batch.id, n);
        let archived = work_archive::archive_rule(
            &self.channels,
            &self.commands,
            Some(COLLECT),
            &self.rules[n],
            id.clone(),
            NOW - 10,
        )
        .await
        .unwrap();
        assert!(
            matches!(archived, Ok(RuleArchived::Moving(_))),
            "{archived:?}"
        );
        if let Some((state, outcome)) = end {
            self.commands
                .claim_next_excluding(NOW - 5, vec![batch.id.clone()])
                .await
                .unwrap();
            self.commands
                .finish(&id, state, outcome, NOW - 5)
                .await
                .unwrap();
        }
    }

    /// What the stored batch looks like to the screen.
    async fn view(&self, id: &str) -> BatchView {
        let batch = self.commands.get(id).await.unwrap().unwrap();
        let stores = Stores {
            commands: &self.commands,
            channels: &self.channels,
            settings: &self.settings,
            library: &self.library,
        };
        view(&stores, &batch).await.unwrap()
    }

    /// The change announcements about `id` heard so far.
    fn announced(&mut self, id: &str) -> usize {
        let mut count = 0;
        while let Ok(change) = self.listener.try_recv() {
            if change.kind == ChangeKind::Command && change.id.as_deref() == Some(id) {
                count += 1;
            }
        }
        count
    }
}

/// What the fake loop does with the move of a work folder.
type Decide = Arc<dyn Fn(&str) -> (CommandState, Outcome) + Send + Sync>;

fn archived(result: &str) -> (CommandState, Outcome) {
    (
        CommandState::Done,
        Outcome {
            result: result.to_owned(),
            reason: None,
        },
    )
}

fn failed(reason: &str) -> (CommandState, Outcome) {
    (
        CommandState::Failed,
        Outcome {
            result: "failed".to_owned(),
            reason: Some(reason.to_owned()),
        },
    )
}

/// A stand-in for the commands' loop: starts when the batch rings, takes each
/// open command except the ones in `exclude`, in the order they were stored,
/// and ends it as `decide` says of its work folder. `store` is the store it
/// ends them through; one that is not the batch's has no channel in common
/// with it.
struct Loop {
    task: JoinHandle<()>,
    /// The work folders it moved, in the order it moved them.
    ran: Arc<Mutex<Vec<String>>>,
}

impl Loop {
    fn start(scene: &Scene, store: CommandStore, exclude: &[&str], decide: Decide) -> Loop {
        let ran = Arc::new(Mutex::new(Vec::new()));
        let task = tokio::spawn({
            let exclude: Vec<String> = exclude.iter().map(|id| (*id).to_owned()).collect();
            let (wakes, ran) = (scene.wakes.clone(), ran.clone());
            async move {
                loop {
                    wakes.commands_rung().await;
                    while let Some(command) = store
                        .claim_next_excluding(NOW, exclude.clone())
                        .await
                        .unwrap()
                    {
                        let moved: WorkArchive = serde_json::from_str(&command.payload).unwrap();
                        ran.lock().unwrap().push(moved.work.clone());
                        let (state, outcome) = decide(&moved.work);
                        store
                            .finish(&command.id, state, outcome, NOW)
                            .await
                            .unwrap();
                    }
                }
            }
        });
        Loop { task, ran }
    }

    fn ran(&self) -> Vec<String> {
        self.ran.lock().unwrap().clone()
    }
}

impl Drop for Loop {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn all_archived() -> Decide {
    Arc::new(|_| archived("moved"))
}

// --- the request -----------------------------------------------------------------------

fn request(ids: &[&str]) -> ArchiveBatch {
    ArchiveBatch {
        rules: ids
            .iter()
            .map(|id| BatchRule {
                rule_id: (*id).to_owned(),
                name: format!("작품 {id}"),
            })
            .collect(),
    }
}

#[test]
fn a_request_holds_any_number_of_different_rules_with_fields_of_sane_length() {
    assert_eq!(request(&["a"]).refusal(), None);
    // No cap on how many: the screen sends every suggestion it shows.
    let many: Vec<String> = (0..250).map(|n| format!("rule-{n}")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();
    assert_eq!(request(&many).refusal(), None);

    assert!(request(&[]).refusal().is_some());
    assert!(request(&["a", "b", "a"]).refusal().is_some());

    let longest_id = "x".repeat(MAX_ID_BYTES);
    assert_eq!(request(&[&longest_id]).refusal(), None);
    let too_long_id = "x".repeat(MAX_ID_BYTES + 1);
    assert!(request(&[&too_long_id]).refusal().is_some());

    let named = |name: String| ArchiveBatch {
        rules: vec![BatchRule {
            rule_id: "a".to_owned(),
            name,
        }],
    };
    // Characters, not bytes: Korean titles are three bytes each.
    assert_eq!(named("작".repeat(MAX_NAME_CHARS)).refusal(), None);
    assert!(named("작".repeat(MAX_NAME_CHARS + 1)).refusal().is_some());
}

#[test]
fn a_payload_with_a_field_it_does_not_know_is_not_read() {
    let ok = json!({"rules": [{"rule_id": "a", "name": "n"}]});
    assert!(serde_json::from_value::<ArchiveBatch>(ok).is_ok());
    for bad in [
        json!({"rules": [{"rule_id": "a", "name": "n"}], "extra": 1}),
        json!({"rules": [{"rule_id": "a", "name": "n", "extra": 1}]}),
        json!({"rules": [{"rule_id": "a"}]}),
        json!({}),
    ] {
        assert!(serde_json::from_value::<ArchiveBatch>(bad).is_err());
    }
}

#[test]
fn the_moves_have_ids_made_of_the_batch_and_the_place() {
    assert_eq!(child_id("0123456789abcdef", 0), "0123456789abcdef-0");
    assert_eq!(child_id("0123456789abcdef", 12), "0123456789abcdef-12");
}

// --- running ---------------------------------------------------------------------------

#[tokio::test]
async fn the_rules_are_deleted_in_the_order_given_each_with_the_move_of_its_work_folder() {
    let scene = Scene::new(3).await;
    let batch = scene.batch("batch-order-1", 3).await;
    let fake = Loop::start(&scene, scene.commands.clone(), &[&batch.id], all_archived());

    let finished = scene.run(&batch).await.unwrap();

    assert_eq!(finished.state, CommandState::Done);
    assert_eq!(finished.outcome.result, ARCHIVED);
    assert_eq!(fake.ran(), scene.works(0..3));
    for n in 0..3 {
        let child = scene
            .commands
            .get(&child_id(&batch.id, n))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(child.kind, work_archive::KIND);
        assert_eq!(child.subject, Some(format!("Work {n}")));
        assert_eq!(child.state, CommandState::Done);
        assert!(!scene.has_rule(n).await);
    }
}

#[tokio::test]
async fn a_rule_is_not_archived_before_the_move_before_it_has_ended() {
    let scene = Scene::new(2).await;
    let batch = scene.batch("batch-seq-001", 2).await;
    let cancel = CancellationToken::new();
    // No loop: the first move stays pending.
    let running = scene.spawn_run(&batch, &cancel);
    tokio::time::sleep(FALLBACK * 6).await;

    let first = scene.commands.get(&child_id(&batch.id, 0)).await;
    assert!(first.unwrap().is_some());
    assert!(!scene.has_rule(0).await);
    let second = scene.commands.get(&child_id(&batch.id, 1)).await;
    assert!(second.unwrap().is_none());
    assert!(scene.has_rule(1).await);
    assert!(!running.is_finished());
    cancel.cancel();
    running.await.unwrap().unwrap_err();
}

#[tokio::test]
async fn the_batch_stops_at_the_first_move_that_failed_and_archives_no_later_rule() {
    let scene = Scene::new(4).await;
    let batch = scene.batch("batch-fail-01", 4).await;
    let fake = Loop::start(
        &scene,
        scene.commands.clone(),
        &[&batch.id],
        Arc::new(|work| {
            if work == "Work 1" {
                failed("Transmission이 폴더를 옮기지 못했어요.")
            } else {
                archived("moved")
            }
        }),
    );

    let finished = scene.run(&batch).await.unwrap();

    assert_eq!(finished.state, CommandState::Failed);
    assert_eq!(finished.outcome.result, stopped_at(RULE_FAILED, 1));
    assert_eq!(
        finished.outcome.reason.as_deref(),
        Some("Transmission이 폴더를 옮기지 못했어요.")
    );
    assert_eq!(fake.ran(), scene.works(0..2));
    // The rule whose move failed is deleted; the later ones are untouched.
    assert!(!scene.has_rule(1).await);
    for later in 2..4 {
        let child = scene.commands.get(&child_id(&batch.id, later)).await;
        assert_eq!(child.unwrap(), None, "no move for rule {later}");
        assert!(scene.has_rule(later).await, "rule {later} is kept");
    }
}

#[tokio::test]
async fn a_rule_whose_work_folder_is_moving_already_stops_the_batch_as_refused_and_stays() {
    let scene = Scene::new(3).await;
    let batch = scene.batch("batch-busy-01", 3).await;
    // The work detail moves the second rule's work folder, and its move is
    // still open.
    scene
        .commands
        .accept(
            WorkArchive {
                work: "Work 1".into(),
            }
            .command("move-from-detail".into()),
            NOW,
        )
        .await
        .unwrap();
    let fake = Loop::start(
        &scene,
        scene.commands.clone(),
        &[&batch.id, "move-from-detail"],
        all_archived(),
    );

    let finished = tokio::time::timeout(Duration::from_secs(5), scene.run(&batch))
        .await
        .expect("the batch did not hang")
        .unwrap();

    assert_eq!(finished.state, CommandState::Failed);
    assert_eq!(finished.outcome.result, stopped_at(REFUSED, 1));
    assert_eq!(finished.outcome.reason.as_deref(), Some(WORK_BUSY));
    assert_eq!(fake.ran(), scene.works(0..1));
    assert!(scene.has_rule(1).await, "the refused rule is kept");
    let third = scene.commands.get(&child_id(&batch.id, 2)).await.unwrap();
    assert_eq!(
        third, None,
        "the rule after the refused one was not archived"
    );
    assert!(scene.has_rule(2).await);
}

#[tokio::test]
async fn a_rule_that_is_gone_is_skipped_and_one_with_no_work_folder_is_only_deleted() {
    let mut scene = Scene::with(&[
        "Work 0/Season 01",
        "Work 1/Season 01",
        // The collect folder itself: no work folder to move.
        "",
        "Work 3/Season 01",
    ])
    .await;
    let batch = scene.batch("batch-skip-01", 4).await;
    scene.announced(&batch.id);
    // The second rule was deleted before its turn.
    scene
        .channels
        .delete_rule(&scene.rule_ids[1], scene.rules[1].version)
        .await
        .unwrap();
    let fake = Loop::start(&scene, scene.commands.clone(), &[&batch.id], all_archived());

    let finished = scene.run(&batch).await.unwrap();

    assert_eq!(finished.state, CommandState::Done, "{finished:?}");
    assert_eq!(fake.ran(), ["Work 0", "Work 3"]);
    for n in 0..4 {
        assert!(!scene.has_rule(n).await, "rule {n} is gone");
    }
    for skipped in [1, 2] {
        let child = scene.commands.get(&child_id(&batch.id, skipped)).await;
        assert_eq!(child.unwrap(), None, "no move for rule {skipped}");
    }
    // Every rule counts as progress, the skipped ones too.
    assert_eq!(scene.announced(&batch.id), 4);
}

#[tokio::test]
async fn progress_is_announced_for_the_batch_after_each_finished_rule() {
    let mut scene = Scene::new(3).await;
    let batch = scene.batch("batch-news-01", 3).await;
    // What was announced about the batch before the run (its claim).
    scene.announced(&batch.id);
    let _fake = Loop::start(&scene, scene.commands.clone(), &[&batch.id], all_archived());

    scene.run(&batch).await.unwrap();

    assert_eq!(scene.announced(&batch.id), 3);
}

#[tokio::test]
async fn a_batch_run_again_skips_the_finished_rules_and_follows_the_open_move() {
    let scene = Scene::new(3).await;
    let batch = scene.batch("batch-again-1", 3).await;
    // An earlier run archived the first rule and the second, whose move is
    // still open, then the process stopped.
    scene.store_child(&batch, 0, Some(archived("moved"))).await;
    scene.store_child(&batch, 1, None).await;
    let before = scene
        .commands
        .get(&child_id(&batch.id, 0))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.state, CommandState::Done);

    // The commands' loop of the new process takes what is open.
    let fake = Loop::start(&scene, scene.commands.clone(), &[&batch.id], all_archived());
    let finished = scene.run(&batch).await.unwrap();

    assert_eq!(finished.state, CommandState::Done);
    assert_eq!(
        fake.ran(),
        scene.works(1..3),
        "the finished rule is not archived again"
    );
    let after = scene.commands.get(&before.id).await.unwrap().unwrap();
    assert_eq!(after, before, "its move was left as it was");
    assert_eq!(after.attempts, 1);
}

#[tokio::test]
async fn a_batch_run_again_after_a_move_failed_stops_at_that_rule_without_archiving_the_rest() {
    let scene = Scene::new(3).await;
    let batch = scene.batch("batch-again-2", 3).await;
    scene
        .store_child(&batch, 0, Some(failed("옮기지 못했어요.")))
        .await;
    let fake = Loop::start(&scene, scene.commands.clone(), &[&batch.id], all_archived());

    let finished = scene.run(&batch).await.unwrap();

    assert_eq!(finished.outcome.result, stopped_at(RULE_FAILED, 0));
    assert_eq!(finished.outcome.reason.as_deref(), Some("옮기지 못했어요."));
    assert!(fake.ran().is_empty());
    assert_eq!(
        scene.commands.get(&child_id(&batch.id, 1)).await.unwrap(),
        None
    );
    assert!(scene.has_rule(1).await);
}

#[tokio::test]
async fn a_command_of_another_kind_stored_under_the_id_stops_the_batch() {
    let scene = Scene::new(2).await;
    let batch = scene.batch("batch-clash-01", 2).await;
    scene
        .commands
        .accept(
            NewCommand {
                id: child_id(&batch.id, 0),
                kind: "watch_rescan".to_owned(),
                payload: json!({ "folder_id": "f" }).to_string(),
                subject: Some("f".to_owned()),
            },
            NOW,
        )
        .await
        .unwrap();

    let finished = scene.run(&batch).await.unwrap();

    assert_eq!(finished.state, CommandState::Failed);
    assert_eq!(finished.outcome.result, stopped_at(RULE_FAILED, 0));
    assert!(scene.has_rule(0).await);
}

#[tokio::test]
async fn a_batch_an_earlier_version_left_skips_its_finished_archive_and_stops_at_an_open_one() {
    let scene = Scene::new(3).await;
    let batch = scene.batch("batch-before-1", 3).await;
    // That version's `보관` kept the rule as archived, under the child ID; the
    // upgrade deleted the rule it had archived.
    let old = |n: usize| NewCommand {
        id: child_id(&batch.id, n),
        kind: rule_archive::KIND.to_owned(),
        payload: json!({ "rule_id": scene.rule_ids[n], "direction": "archive" }).to_string(),
        subject: Some(scene.rule_ids[n].clone()),
    };
    scene.commands.accept(old(0), NOW - 10).await.unwrap();
    scene
        .commands
        .claim_next_excluding(NOW - 5, vec![batch.id.clone()])
        .await
        .unwrap();
    let (state, outcome) = archived("moved");
    scene
        .commands
        .finish(&child_id(&batch.id, 0), state, outcome, NOW - 5)
        .await
        .unwrap();
    let first = &scene.rules[0];
    scene
        .channels
        .delete_rule(&first.id, first.version)
        .await
        .unwrap();
    scene.commands.accept(old(1), NOW - 5).await.unwrap();
    // The worker fails that one by itself; this loop moves only what the batch stores.
    let open = child_id(&batch.id, 1);
    let fake = Loop::start(
        &scene,
        scene.commands.clone(),
        &[&batch.id, &open],
        all_archived(),
    );

    let finished = scene.run(&batch).await.unwrap();

    assert_eq!(finished.state, CommandState::Failed);
    assert_eq!(finished.outcome.result, stopped_at(REFUSED, 1));
    assert_eq!(finished.outcome.reason.as_deref(), Some(FROM_BEFORE));
    assert!(fake.ran().is_empty());
    assert!(scene.has_rule(1).await);
    assert!(scene.has_rule(2).await);
    assert_eq!(
        scene.commands.get(&child_id(&batch.id, 2)).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn the_wait_ends_by_its_fallback_when_no_event_reaches_it() {
    let scene = Scene::new(2).await;
    let batch = scene.batch("batch-quiet-1", 2).await;
    // The loop ends the moves through a store of its own on the same database:
    // what it announces goes to a channel nobody on the batch's side listens to.
    let quiet = CommandStore::new(scene.db.clone());
    let _fake = Loop::start(&scene, quiet, &[&batch.id], all_archived());

    let finished = tokio::time::timeout(Duration::from_secs(5), scene.run(&batch))
        .await
        .expect("the batch did not hang")
        .unwrap();

    assert_eq!(finished.state, CommandState::Done);
}

#[tokio::test]
async fn a_change_of_the_move_ends_the_wait_before_the_fallback() {
    let scene = Scene::new(1).await;
    let batch = scene.batch("batch-quick-1", 1).await;
    let slow = Ctx {
        reread_every: Duration::from_secs(600),
        ..scene.ctx()
    };
    let _fake = Loop::start(&scene, scene.commands.clone(), &[&batch.id], all_archived());

    let finished = tokio::time::timeout(
        Duration::from_secs(5),
        run(&slow, &batch, &CancellationToken::new()),
    )
    .await
    .expect("the event ended the wait")
    .unwrap();

    assert_eq!(finished.state, CommandState::Done);
}

#[tokio::test]
async fn a_batch_rings_the_commands_loop_when_it_stores_a_move() {
    let scene = Scene::new(1).await;
    let batch = scene.batch("batch-ring-01", 1).await;
    let cancel = CancellationToken::new();
    let running = scene.spawn_run(&batch, &cancel);

    tokio::time::timeout(Duration::from_secs(5), scene.wakes.commands_rung())
        .await
        .expect("the loop was rung");

    cancel.cancel();
    assert!(matches!(running.await.unwrap(), Err(Retry::Stopped)));
}

#[tokio::test]
async fn a_shutdown_during_the_wait_leaves_the_batch_to_run_again() {
    let scene = Scene::new(2).await;
    let batch = scene.batch("batch-stop-01", 2).await;
    let cancel = CancellationToken::new();
    let running = scene.spawn_run(&batch, &cancel);
    tokio::time::sleep(FALLBACK * 3).await;

    cancel.cancel();
    let outcome = running.await.unwrap();

    assert!(matches!(outcome, Err(Retry::Stopped)), "{outcome:?}");
    // Nothing ended: the first move is still open and the second rule was not
    // archived.
    let first = scene.commands.get(&child_id(&batch.id, 0)).await;
    assert!(first.unwrap().unwrap().state.is_open());
    let second = scene.commands.get(&child_id(&batch.id, 1)).await;
    assert_eq!(second.unwrap(), None);
    assert!(scene.has_rule(1).await);
}

// --- the view --------------------------------------------------------------------------

#[tokio::test]
async fn the_view_counts_the_leading_rules_that_are_archived_with_their_moves_and_names_the_one_in_turn(
) {
    let scene = Scene::new(3).await;
    let batch = scene.batch("batch-view-01", 3).await;

    let view = scene.view(&batch.id).await;
    assert_eq!(
        (view.state, view.total, view.finished),
        (CommandState::Running, 3, 0)
    );
    assert!(view.results.is_empty());
    let current = view.current.unwrap();
    assert_eq!(
        (current.rule_id.as_str(), current.name.as_str()),
        (scene.rule_ids[0].as_str(), "작품 0")
    );
    assert_eq!(view.failure, None);

    // The first rule done, the second's move open.
    scene.store_child(&batch, 0, Some(archived("kept"))).await;
    scene.store_child(&batch, 1, None).await;
    let view = scene.view(&batch.id).await;
    assert_eq!(view.finished, 1);
    let [result] = view.results.as_slice() else {
        panic!("one result: {:?}", view.results);
    };
    assert_eq!(
        (result.rule_id.as_str(), result.name.as_str()),
        (scene.rule_ids[0].as_str(), "작품 0")
    );
    let command = result.command.as_ref().unwrap();
    assert_eq!(command.id, child_id(&batch.id, 0));
    assert_eq!(command.outcome.as_ref().unwrap().result, "kept");
    assert_eq!(result.work_id, Some(scene.work_id(0).await));
    let current = view.current.unwrap();
    assert_eq!(
        (current.rule_id.as_str(), current.name.as_str()),
        (scene.rule_ids[1].as_str(), "작품 1")
    );
}

#[tokio::test]
async fn the_view_counts_a_rule_gone_with_no_move_as_archived() {
    let scene = Scene::new(2).await;
    let batch = scene.batch("batch-view-04", 2).await;
    scene
        .channels
        .delete_rule(&scene.rule_ids[0], scene.rules[0].version)
        .await
        .unwrap();

    let view = scene.view(&batch.id).await;

    assert_eq!(view.finished, 1);
    assert_eq!(view.results[0].command, None);
    assert_eq!(view.results[0].work_id, None);
    assert_eq!(view.current.unwrap().rule_id, scene.rule_ids[1]);
}

#[tokio::test]
async fn the_view_of_a_batch_that_ended_has_no_current_rule() {
    let scene = Scene::new(2).await;
    let batch = scene.batch("batch-view-02", 2).await;
    let _fake = Loop::start(&scene, scene.commands.clone(), &[&batch.id], all_archived());
    let finished = scene.run(&batch).await.unwrap();
    scene
        .commands
        .finish(&batch.id, finished.state, finished.outcome, NOW)
        .await
        .unwrap();

    let view = scene.view(&batch.id).await;

    assert_eq!(
        (view.state, view.total, view.finished),
        (CommandState::Done, 2, 2)
    );
    assert_eq!(view.results.len(), 2);
    assert_eq!(view.results[1].work_id, Some(scene.work_id(1).await));
    assert_eq!((view.current, view.failure), (None, None));
}

#[tokio::test]
async fn the_view_of_a_failed_batch_names_the_rule_it_stopped_at_why_and_the_work_to_move_again() {
    let scene = Scene::new(3).await;
    let batch = scene.batch("batch-view-03", 3).await;
    scene.store_child(&batch, 0, Some(archived("kept"))).await;
    scene
        .store_child(&batch, 1, Some(failed("옮기지 못했어요.")))
        .await;
    scene
        .commands
        .finish(
            &batch.id,
            CommandState::Failed,
            Outcome {
                result: stopped_at(RULE_FAILED, 1),
                reason: Some("옮기지 못했어요.".to_owned()),
            },
            NOW,
        )
        .await
        .unwrap();

    let view = scene.view(&batch.id).await;

    assert_eq!(
        (view.state, view.finished, view.current),
        (CommandState::Failed, 1, None)
    );
    assert_eq!(
        view.failure,
        Some(BatchFailure {
            rule_id: scene.rule_ids[1].clone(),
            name: "작품 1".to_owned(),
            reason: "옮기지 못했어요.".to_owned(),
            kind: FailureKind::ArchiveFailed,
            work_id: Some(scene.work_id(1).await),
        })
    );

    // A refused rule is still there, with no move to show.
    let batch = scene.batch("batch-view-05", 3).await;
    scene
        .commands
        .finish(
            &batch.id,
            CommandState::Failed,
            Outcome {
                result: stopped_at(REFUSED, 2),
                reason: Some(WORK_BUSY.to_owned()),
            },
            NOW,
        )
        .await
        .unwrap();
    let failure = scene.view(&batch.id).await.failure.unwrap();
    assert_eq!(
        (failure.kind, failure.work_id, failure.rule_id),
        (FailureKind::Refused, None, scene.rule_ids[2].clone())
    );
}

#[tokio::test]
async fn a_batch_the_claim_gave_up_names_no_rule_though_its_open_move_ends_after_it() {
    use trss_core::commands::MAX_ATTEMPTS;

    let scene = Scene::new(3).await;
    let batch = scene.batch("batch-giveup-1", 3).await;
    // The process stopped in the batch again and again.
    for _ in 1..MAX_ATTEMPTS {
        scene
            .commands
            .claim_next_excluding(NOW, Vec::new())
            .await
            .unwrap()
            .unwrap();
    }
    // The first rule is archived and the second's move is open.
    scene.store_child(&batch, 0, Some(archived("moved"))).await;
    scene.store_child(&batch, 1, None).await;

    // The next claim gives the batch up (it is the older) and hands out the move.
    let claimed = scene
        .commands
        .claim_next_excluding(NOW, Vec::new())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(claimed.id, child_id(&batch.id, 1));
    let stored = scene.commands.get(&batch.id).await.unwrap().unwrap();
    assert_eq!(stored.state, CommandState::Failed);

    // The move still ends `done` afterwards.
    scene
        .commands
        .finish(&claimed.id, CommandState::Done, archived("moved").1, NOW)
        .await
        .unwrap();
    let view = scene.view(&batch.id).await;

    assert_eq!(view.state, CommandState::Failed);
    assert_eq!(view.finished, 2);
    assert_eq!(
        view.failure, None,
        "the third rule was never archived: it is not the one the batch stopped at"
    );
    assert_eq!(view.current, None);
}
