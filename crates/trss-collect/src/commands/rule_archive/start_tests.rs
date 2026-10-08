//! A rule that starts collecting while its work folder is in the archive
//! folder (ticket 0123): what the web decides before it makes or turns on the
//! rule ([`plan_start`], [`archived_work`], [`ask_start`]) and the order the
//! `start` command keeps (move first, turn on after). The move's checks and
//! renames are in `work_folder/tests.rs`; the command around the move with
//! Transmission is in `run_tests.rs`, and a cycle running beside it, with the
//! order of the commands, in trss-worker.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

use trss_core::{
    commands::{Accepted, Command, CommandStore},
    Db,
};
use trss_library::{
    discovery::{EpisodeFile, FileKind, Scan, ScannedWork, WorkRead},
    live::{LiveConfig, LiveWatch},
};

use super::*;
use crate::{
    store::channels::{ChannelInput, RuleInput},
    test_world::{files as listing, write},
};

fn both(collect: &Path, archive: &Path) -> Option<(String, Option<String>)> {
    Some((
        collect.display().to_string(),
        Some(archive.display().to_string()),
    ))
}

struct Folders {
    _tmp: TempDir,
    collect: PathBuf,
    archive: PathBuf,
}

fn folders() -> Folders {
    let tmp = tempfile::tempdir().unwrap();
    let collect = tmp.path().join("Shows (current)");
    let archive = tmp.path().join("Shows");
    fs::create_dir(&collect).unwrap();
    fs::create_dir(&archive).unwrap();
    Folders {
        _tmp: tmp,
        collect,
        archive,
    }
}

#[tokio::test]
async fn a_rule_waits_for_its_work_folder_only_when_the_archive_folder_holds_it() {
    let f = folders();
    write(&f.archive.join("A/Season 02/e01.mkv"), "x");
    write(&f.collect.join("OnlyHere/Season 01/e01.mkv"), "x");
    let settings = both(&f.collect, &f.archive);
    let waits = StartPlan::MoveFirst { work: "A".into() };

    // Any save folder in the work folder, whatever the season.
    assert_eq!(plan_start(settings.clone(), "A/Season 03").await, waits);
    assert_eq!(plan_start(settings.clone(), "A").await, waits);
    assert_eq!(plan_start(settings.clone(), "./A//Season 03/").await, waits);
    // The work folder is in the collect folder only, or nowhere.
    assert_eq!(
        plan_start(settings.clone(), "OnlyHere/Season 02").await,
        StartPlan::Now
    );
    assert_eq!(
        plan_start(settings.clone(), "Nowhere/Season 01").await,
        StartPlan::Now
    );
    // Both folders hold it: it merges into the collect folder's.
    write(&f.collect.join("A/Season 01/e01.mkv"), "x");
    assert_eq!(plan_start(settings.clone(), "A/Season 03").await, waits);
}

#[tokio::test]
async fn a_rule_without_a_work_folder_or_an_archive_folder_does_not_wait() {
    let f = folders();
    write(&f.archive.join("A/Season 02/e01.mkv"), "x");
    let settings = both(&f.collect, &f.archive);

    // It saves into the collect folder itself, or outside it.
    assert_eq!(plan_start(settings.clone(), "").await, StartPlan::Now);
    assert_eq!(plan_start(settings.clone(), ".").await, StartPlan::Now);
    assert_eq!(
        plan_start(settings.clone(), "../Shows/A").await,
        StartPlan::Now
    );
    assert_eq!(
        plan_start(settings.clone(), "/elsewhere/A").await,
        StartPlan::Now
    );
    // No archive folder to bring anything out of, or no collect folder.
    let no_archive = Some((f.collect.display().to_string(), None));
    assert_eq!(plan_start(no_archive, "A/Season 03").await, StartPlan::Now);
    assert_eq!(plan_start(None, "A/Season 03").await, StartPlan::Now);
}

#[tokio::test]
async fn an_archive_entry_that_is_a_link_or_a_file_still_makes_the_rule_wait() {
    let f = folders();
    // The move refuses a link out of the folders, and says so; the rule must
    // not collect beside it meanwhile.
    std::os::unix::fs::symlink(f.collect.parent().unwrap(), f.archive.join("Linked")).unwrap();
    write(&f.archive.join("Plain"), "a file, not a folder");
    let settings = both(&f.collect, &f.archive);
    for name in ["Linked", "Plain"] {
        assert_eq!(
            plan_start(settings.clone(), &format!("{name}/Season 01")).await,
            StartPlan::MoveFirst { work: name.into() },
            "{name}"
        );
    }
}

/// A library with `/…/Shows (current)` and `/…/Shows` registered, each holding
/// the works named, as the library records them.
async fn library(f: &Folders, collect: &[&str], archive: &[&str]) -> LibraryStore {
    let db = Db::open_blocking(":memory:").unwrap();
    let library = LibraryStore::new(db);
    let scan = |names: &[&str]| Scan {
        works: names
            .iter()
            .map(|name| {
                WorkRead::Read(ScannedWork {
                    dir_name: (*name).to_owned(),
                    seasons: BTreeSet::from([1]),
                    files: vec![EpisodeFile {
                        path: format!("Season 01/{name} S01E01.mkv"),
                        kind: FileKind::Video,
                        season: 1,
                        episode: "01".to_owned(),
                    }],
                    unrecognized: Vec::new(),
                })
            })
            .collect(),
    };
    let (first, _) = library
        .add_folder(f.collect.display().to_string(), scan(collect), 100, &[])
        .await
        .unwrap();
    library
        .add_folder(
            f.archive.display().to_string(),
            scan(archive),
            100,
            &[first],
        )
        .await
        .unwrap();
    library
}

#[tokio::test]
async fn the_form_learns_of_an_archived_work_from_the_library_not_the_disk() {
    let f = folders();
    // The library knows `A` (archive only) and `B` (both folders); the disk holds
    // nothing at all, so what is told cannot come from it.
    let library = library(&f, &["B", "C"], &["A", "B"]).await;
    let settings = both(&f.collect, &f.archive);
    let told = |directory: &'static str| {
        let (settings, library) = (settings.clone(), library.clone());
        async move { archived_work(settings, &library, directory).await.unwrap() }
    };

    let a = told("A/Season 03").await.unwrap();
    assert_eq!(a.work, "A");
    assert_eq!(a.archive_folder, f.archive.display().to_string());
    assert_eq!(a.collect_folder, f.collect.display().to_string());
    assert!(!a.merges);
    // The collect folder has a work of that name too: the archive's merges into it.
    assert!(told("B/Season 02").await.unwrap().merges);
    // A work only the collect folder has, or none, is not an archived work.
    assert_eq!(told("C/Season 02").await, None);
    assert_eq!(told("New/Season 01").await, None);
    // No work folder in the typed folder.
    assert_eq!(told("").await, None);
    assert_eq!(told("../Shows/A").await, None);
    // No archive folder: nothing is archived.
    let no_archive = Some((f.collect.display().to_string(), None));
    assert_eq!(
        archived_work(no_archive, &library, "A/Season 03")
            .await
            .unwrap(),
        None
    );
    // Folders typed with a trailing slash are the registered ones.
    let slashed = Some((
        format!("{}/", f.collect.display()),
        Some(format!("{}/", f.archive.display())),
    ));
    assert!(archived_work(slashed, &library, "A/Season 03")
        .await
        .unwrap()
        .is_some());
}

#[test]
fn a_start_is_a_payload_of_its_own() {
    let payload = RuleArchive {
        rule_id: "r1".into(),
        direction: Direction::Start,
        receive: Vec::new(),
    };
    assert_eq!(
        payload.canonical(),
        r#"{"rule_id":"r1","direction":"start"}"#
    );
    assert_eq!(Direction::Start.code(), "start");
    assert_eq!(Direction::Resume.code(), "resume");
    let resume = RuleArchive {
        rule_id: "r1".into(),
        direction: Direction::Resume,
        receive: Vec::new(),
    };
    assert_eq!(
        resume.canonical(),
        r#"{"rule_id":"r1","direction":"resume"}"#
    );
    assert_ne!(payload.canonical(), resume.canonical());
}

struct World {
    folders: Folders,
    ctx: ArchiveContext,
    commands: CommandStore,
    channels: ChannelStore,
}

/// A paused rule `Alpha` saving to `directory`, with the collect folder and
/// (when `with_archive`) the archive folder set, and a Transmission nothing
/// listens to: a move that gets as far as asking it fails there.
async fn world(directory: &str, with_archive: bool) -> (World, Rule) {
    let folders = folders();
    let db = Db::open_blocking(":memory:").unwrap();
    let settings = SettingsStore::new(db.clone());
    settings
        .put_collection(
            0,
            folders.collect.display().to_string(),
            with_archive.then(|| folders.archive.display().to_string()),
        )
        .await
        .unwrap();
    let channels = ChannelStore::new(db.clone());
    let created = channels
        .create_channel_with_rules(
            ChannelInput::new("http://feed.test/rss"),
            vec![RuleInput {
                r#match: Some("Alpha".into()),
                directory: directory.to_owned(),
                state: RuleState::Paused,
                ..RuleInput::default()
            }],
        )
        .await
        .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let ctx = ArchiveContext {
        channels: channels.clone(),
        settings,
        library: LibraryStore::new(db.clone()),
        live: LiveWatch::new(LiveConfig::default()),
        transmission: TransmissionLink {
            url: format!("http://{addr}/transmission/rpc").parse().unwrap(),
            http: transmission::http_client(Duration::from_secs(2)).unwrap(),
        },
        moves: MovePolicy::default(),
        redactor: transmission::Redactor::none(),
        commands: CommandStore::new(db.clone()),
    };
    let rule = created.rules.into_iter().next().unwrap();
    assert_eq!(rule.state, RuleState::Paused);
    (
        World {
            folders,
            ctx,
            commands: CommandStore::new(db),
            channels,
        },
        rule,
    )
}

impl World {
    /// Runs the `start` command of `rule` as the worker does.
    async fn start(&self, rule: &Rule) -> Finished {
        self.run(rule, Direction::Start).await
    }

    /// Runs the command of `direction` (accepted at 1,000) for `rule`.
    async fn run(&self, rule: &Rule, direction: Direction) -> Finished {
        let Accepted::Created(command) = ask_start(&self.commands, &rule.id, direction, 1_000)
            .await
            .unwrap()
        else {
            panic!("the command was not stored");
        };
        self.run_again(&command).await
    }

    /// Stores the `start` of `rule` carrying the ticked items `receive` (at
    /// 1,000), unless `receive` is empty.
    async fn start_receiving(&self, rule: &Rule, receive: Vec<i64>) -> Option<Command> {
        if receive.is_empty() {
            return None;
        }
        let Accepted::Created(command) =
            ask_start_receiving(&self.commands, &rule.id, Direction::Start, receive, 1_000)
                .await
                .unwrap()
        else {
            panic!("the command was not stored");
        };
        Some(command)
    }

    /// Runs `command` (at 2,000), as a worker's start of it does.
    async fn run_again(&self, command: &Command) -> Finished {
        let clock: Clock = Arc::new(|| 2_000);
        run_on(
            &self.ctx,
            command,
            Arc::new(RealDisk),
            Arc::new(()),
            &clock,
            &CancellationToken::new(),
        )
        .await
        .unwrap()
    }

    /// Every open command, in the order a worker's look claims them.
    async fn claim_all(&self) -> Vec<Command> {
        let mut claimed: Vec<Command> = Vec::new();
        let ids = |claimed: &[Command]| claimed.iter().map(|c| c.id.clone()).collect();
        while let Some(next) = self
            .commands
            .claim_next_excluding(3_000, ids(&claimed))
            .await
            .unwrap()
        {
            claimed.push(next);
        }
        claimed
    }

    async fn state(&self, rule: &Rule) -> RuleState {
        self.channels
            .get_rule(&rule.id)
            .await
            .unwrap()
            .unwrap()
            .state
    }
}

#[tokio::test]
async fn a_start_that_cannot_move_the_folder_leaves_the_rule_off_and_says_why() {
    let (w, rule) = world("A/Season 03", true).await;
    // The same file on both sides: the move refuses before it moves anything.
    write(&w.folders.collect.join("A/Season 02/e01.mkv"), "new");
    write(&w.folders.archive.join("A/Season 02/e01.mkv"), "old");
    write(&w.folders.archive.join("A/Season 01/e01.mkv"), "old");
    let (collect_before, archive_before) =
        (listing(&w.folders.collect), listing(&w.folders.archive));

    let finished = w.start(&rule).await;

    assert_eq!(finished.state, CommandState::Failed);
    assert_eq!(finished.outcome.result, FAILED);
    let reason = finished.outcome.reason.unwrap();
    assert!(reason.contains("`Season 02/e01.mkv`"), "{reason}");
    assert!(reason.contains("아무것도 옮기지 않았어요"), "{reason}");
    // The rule collects nothing, and nothing moved.
    assert_eq!(w.state(&rule).await, RuleState::Paused);
    assert_eq!(listing(&w.folders.collect), collect_before);
    assert_eq!(listing(&w.folders.archive), archive_before);
}

#[tokio::test]
async fn a_start_that_cannot_reach_transmission_leaves_the_rule_off() {
    let (w, rule) = world("A/Season 03", true).await;
    write(&w.folders.archive.join("A/Season 02/e01.mkv"), "old");

    let finished = w.start(&rule).await;

    assert_eq!(finished.state, CommandState::Failed);
    let reason = finished.outcome.reason.unwrap();
    assert!(
        reason.contains("Transmission에 연결하지 못해서"),
        "{reason}"
    );
    assert_eq!(w.state(&rule).await, RuleState::Paused);
    assert_eq!(listing(&w.folders.archive), ["A/Season 02/e01.mkv"]);
    assert!(listing(&w.folders.collect).is_empty());
}

#[tokio::test]
async fn a_start_with_no_archive_folder_or_no_work_folder_just_turns_the_rule_on() {
    // The archive folder was cleared after the start was stored.
    let (w, rule) = world("A/Season 03", false).await;
    let finished = w.start(&rule).await;
    assert_eq!(finished.state, CommandState::Done);
    assert_eq!(finished.outcome.result, KEPT);
    assert!(finished
        .outcome
        .reason
        .unwrap()
        .contains("보관 폴더를 정하지 않아서"));
    assert_eq!(w.state(&rule).await, RuleState::Active);

    // The rule saves into the collect folder itself: no work folder to bring in.
    let (w, rule) = world("", true).await;
    write(&w.folders.archive.join("A/Season 02/e01.mkv"), "old");
    let finished = w.start(&rule).await;
    assert_eq!(finished.state, CommandState::Done);
    assert_eq!(finished.outcome.result, KEPT);
    assert_eq!(w.state(&rule).await, RuleState::Active);
    assert_eq!(listing(&w.folders.archive), ["A/Season 02/e01.mkv"]);
}

#[tokio::test]
async fn a_start_is_stored_once_per_rule_while_it_is_open() {
    let (w, rule) = world("A/Season 03", true).await;

    let Accepted::Created(first) = ask_start(&w.commands, &rule.id, Direction::Start, 1_000)
        .await
        .unwrap()
    else {
        panic!("not stored");
    };
    assert_eq!(first.kind, KIND);
    assert_eq!(first.subject.as_deref(), Some(rule.id.as_str()));
    assert_eq!(
        first.payload,
        format!(r#"{{"rule_id":"{}","direction":"start"}}"#, rule.id)
    );
    // Another `rule_archive` of the rule, a start or a resume, waits for it.
    for direction in [Direction::Start, Direction::Resume] {
        assert!(matches!(
            ask_start(&w.commands, &rule.id, direction, 1_001)
                .await
                .unwrap(),
            Accepted::Busy(_)
        ));
    }
}

#[tokio::test]
async fn a_new_rule_is_turned_on_as_if_it_had_collected_since_it_was_made_and_a_switch_since_it_was_pressed(
) {
    // No archive folder is left, so each command only turns its rule on.
    let (w, rule) = world("A/Season 03", false).await;
    assert_eq!(rule.resumed_at, None);
    w.start(&rule).await;
    let started = w.channels.get_rule(&rule.id).await.unwrap().unwrap();
    assert_eq!(started.state, RuleState::Active);
    // What came in while the rule waited for the move is not past.
    assert_eq!(started.resumed_at, None);

    let (w, rule) = world("A/Season 03", false).await;
    let finished = w.run(&rule, Direction::Resume).await;
    assert_eq!(finished.state, CommandState::Done);
    let resumed = w.channels.get_rule(&rule.id).await.unwrap().unwrap();
    assert_eq!(resumed.state, RuleState::Active);
    // The command was accepted at 1,000 and ran at 2,000: it is on since the press.
    assert_eq!(resumed.resumed_at, Some(1_000));
}

#[test]
fn a_start_carries_the_ticked_items_and_other_commands_are_as_before() {
    let payload = RuleArchive {
        rule_id: "r1".into(),
        direction: Direction::Start,
        receive: vec![42, 7],
    };
    assert_eq!(
        payload.canonical(),
        r#"{"rule_id":"r1","direction":"start","receive":[42,7]}"#
    );
    let read: RuleArchive = serde_json::from_str(&payload.canonical()).unwrap();
    assert_eq!(read, payload);
    // A command stored before items were carried reads as one with none.
    let earlier: RuleArchive =
        serde_json::from_str(r#"{"rule_id":"r1","direction":"start"}"#).unwrap();
    assert!(earlier.receive.is_empty());
}

#[tokio::test]
async fn a_start_receives_the_ticked_items_in_order_once_the_rule_is_on_and_once_each() {
    // No archive folder: the start only turns the rule on, which is the step
    // the receives wait for (the move itself is in `run_tests.rs`).
    let (w, rule) = world("A/Season 03", false).await;
    let command = w.start_receiving(&rule, vec![42, 7]).await.unwrap();

    let finished = w.run_again(&command).await;

    assert_eq!(finished.state, CommandState::Done);
    assert_eq!(finished.outcome.result, KEPT);
    let reason = finished.outcome.reason.unwrap();
    assert!(reason.contains("보관 폴더를 정하지 않아서"), "{reason}");
    assert!(
        reason.ends_with("구독할 때 체크한 지난 항목 2개를 이어서 받아요."),
        "{reason}"
    );
    assert_eq!(w.state(&rule).await, RuleState::Active);

    // A start cut short after the receives were accepted runs again: each
    // item is still asked once.
    w.run_again(&command).await;

    // The worker takes the start first, then the receives in the order ticked.
    let claimed: Vec<_> = w
        .claim_all()
        .await
        .into_iter()
        .map(|c| (c.id, c.kind, c.payload))
        .collect();
    let receive = |item: i64| {
        (
            format!("{}-receive-{item}", command.id),
            receive_once::KIND.to_owned(),
            format!(r#"{{"item_id":{item},"rule_id":"{}"}}"#, rule.id),
        )
    };
    assert_eq!(
        claimed,
        [
            (command.id.clone(), KIND.to_owned(), command.payload.clone()),
            receive(42),
            receive(7),
        ]
    );
}

#[tokio::test]
async fn a_start_that_does_not_turn_the_rule_on_receives_none_of_the_ticked_items() {
    let (w, rule) = world("A/Season 03", true).await;
    write(&w.folders.archive.join("A/Season 02/e01.mkv"), "old");
    let command = w.start_receiving(&rule, vec![42, 7]).await.unwrap();

    // Transmission cannot be reached: the move fails and the rule stays off.
    let finished = w.run_again(&command).await;

    assert_eq!(finished.state, CommandState::Failed);
    assert!(!finished.outcome.reason.unwrap().contains("체크한"));
    assert_eq!(w.state(&rule).await, RuleState::Paused);
    let claimed: Vec<String> = w.claim_all().await.into_iter().map(|c| c.id).collect();
    assert_eq!(claimed, [command.id]);
}

/// The rule on, as a rule saved straight into an archived work's folder is.
async fn active(w: &World, rule: &Rule) -> Rule {
    w.channels
        .set_rule_state(&rule.id, RuleState::Active, 777)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn receiving_into_an_archived_work_folder_pauses_the_rule_and_stores_a_start_once() {
    let (w, rule) = world("A/Season 03", true).await;
    write(&w.folders.archive.join("A/Season 02/e01.mkv"), "x");
    let rule = active(&w, &rule).await;

    let guard = move_before_receiving(&w.channels, &w.ctx.settings, &w.commands, &rule, 5_000)
        .await
        .unwrap();
    assert_eq!(
        guard,
        Receiving::MoveFirst {
            work: "A".into(),
            asked: true
        }
    );
    // The rule is off, with the time it was turned on, and a start is open.
    let stored = w.channels.get_rule(&rule.id).await.unwrap().unwrap();
    assert_eq!(stored.state, RuleState::Paused);
    assert_eq!(stored.resumed_at, Some(777));
    assert!(stored.version > rule.version);
    let Accepted::Busy(open) = ask_start(&w.commands, &rule.id, Direction::Resume, 5_001)
        .await
        .unwrap()
    else {
        panic!("the start is not open");
    };
    assert_eq!(
        open.payload,
        format!(r#"{{"rule_id":"{}","direction":"start"}}"#, rule.id)
    );

    // Asked again with the command open, it stores nothing and changes nothing.
    let rule = active(&w, &stored).await;
    let guard = move_before_receiving(&w.channels, &w.ctx.settings, &w.commands, &rule, 5_002)
        .await
        .unwrap();
    assert_eq!(
        guard,
        Receiving::MoveFirst {
            work: "A".into(),
            asked: false
        }
    );
    assert_eq!(w.state(&rule).await, RuleState::Active);
}

#[tokio::test]
async fn receiving_goes_on_when_the_archive_folder_does_not_hold_the_work_folder() {
    let (w, rule) = world("A/Season 03", true).await;
    write(&w.folders.collect.join("A/Season 02/e01.mkv"), "x");
    let rule = active(&w, &rule).await;
    let guard = move_before_receiving(&w.channels, &w.ctx.settings, &w.commands, &rule, 5_000)
        .await
        .unwrap();
    assert_eq!(guard, Receiving::Go);
    assert_eq!(w.state(&rule).await, RuleState::Active);
    assert!(!w.commands.has_open().await.unwrap());

    // With no archive folder set there is nothing to bring in either.
    let (w, rule) = world("A/Season 03", false).await;
    let rule = active(&w, &rule).await;
    let guard = move_before_receiving(&w.channels, &w.ctx.settings, &w.commands, &rule, 5_000)
        .await
        .unwrap();
    assert_eq!(guard, Receiving::Go);
}
