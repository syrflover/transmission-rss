//! The add, record and rename of one item, against the fake Transmission
//! (`trss_transmission::fake`) and an app database in memory.

use super::*;

#[test]
fn each_kind_of_add_failure_has_its_own_sentence_before_transmissions_words() {
    let redactor = Redactor::none();
    assert_eq!(
        failure_reason(
            &AddError::Unreachable("connection refused".into()),
            &redactor
        ),
        "Transmission에 연결하지 못했어요: connection refused"
    );
    assert_eq!(
        failure_reason(&AddError::Rpc("operation timed out".into()), &redactor),
        "Transmission이 응답하지 않았어요: operation timed out"
    );
    assert_eq!(
        failure_reason(
            &AddError::Rejected("invalid or corrupt torrent file".into()),
            &redactor
        ),
        "Transmission이 토렌트를 받지 않았어요: invalid or corrupt torrent file"
    );
}

#[test]
fn only_an_add_that_got_no_answer_may_have_put_the_torrent_in() {
    let redactor = Redactor::none();
    // (the error, whether the request got no answer, whether Transmission refused)
    let cases: [(AddError, bool, bool); 3] = [
        // Never sent: Transmission cannot hold the torrent.
        (
            AddError::Unreachable("connection refused".into()),
            false,
            false,
        ),
        // Sent, and no answer in time: it may hold the torrent all the same.
        (AddError::Rpc("operation timed out".into()), true, false),
        // Answered and refused.
        (AddError::Rejected("duplicate torrent".into()), false, true),
    ];
    for (error, unanswered, refused) in cases {
        let reason = failure_reason(&error, &redactor);
        let failure = AddFailure { error, reason };
        assert_eq!(failure.unanswered(), unanswered, "{failure:?}");
        assert_eq!(failure.refused(), refused, "{failure:?}");
    }
}

#[test]
fn an_add_failure_reason_hides_secrets_and_is_cut_to_the_kept_length() {
    let mut redactor = Redactor::none();
    redactor.add("SECRETTOKEN0123456789");
    let reason = failure_reason(
        &AddError::Rejected(format!(
            "cannot use SECRETTOKEN0123456789 {}",
            "x".repeat(400)
        )),
        &redactor,
    );
    assert!(!reason.contains("SECRETTOKEN0123456789"), "{reason}");
    assert!(reason.starts_with("Transmission이 토렌트를 받지 않았어요: cannot use ***"));
    assert_eq!(reason.chars().count(), MAX_REASON_CHARS);
}

#[test]
fn a_trname_name_is_told_apart_from_a_release_name() {
    let dir = std::path::Path::new("/media/anime/Slime/Season 04");
    for name in [
        "Slime S04E38.mkv",
        "SLIME S04E38.mkv",
        "Slime S04E105.mkv",
        "Slime S04E05.5.mp4",
    ] {
        assert!(looks_renamed(name, dir), "{name}");
    }
    for name in [
        "[SubsPlease] Tensei Shitara Slime Datta Ken - 62 (1080p) [AAAA0006].mkv",
        "Tensura S04E62.mkv",
        "S04E05.mkv",
        "Slime.S04E05.1080p.WEB.mkv",
        "Slime S04E05 (1080p).mkv",
        "SlimeS04E05.mkv",
        "Slime Special.mkv",
    ] {
        assert!(!looks_renamed(name, dir), "{name}");
    }
    assert!(!looks_renamed(
        "Slime S04E38.mkv",
        std::path::Path::new("/")
    ));
}

// --- the rename -------------------------------------------------------------------

use std::time::Duration;

use tempfile::TempDir;
use trss_core::{commands::CommandStore, settings::SettingsStore, Db};
use trss_library::store::{library::LibraryStore, seasons::SeasonStore};
use trss_transmission::{
    fake::{FakeTorrent, FakeTransmission},
    RenamePolicy,
};

use crate::{
    context::TransmissionLink,
    store::{channels::ChannelStore, history::HistoryStore, revisions::RevisionStore},
};

const HASH: &str = "aaaa000000000000000000000000000000000006";
const RELEASE: &str = "[SubsPlease] Tensei Shitara Slime Datta Ken - 62 (1080p) [AAAA0006].mkv";
/// `RELEASE` with the rule's conversion of -24.
const RENAMED: &str = "Slime S04E38.mkv";
const ATTEMPTS: u32 = 5;

/// A fake Transmission, an app database in memory and the rule folder
/// `.../Slime/Season 04` of a rule that converts episode 62 to 38.
struct World {
    tr: FakeTransmission,
    ctx: ReceiveContext,
    _root: TempDir,
    season: std::path::PathBuf,
    redactor: Redactor,
}

impl World {
    async fn new() -> World {
        let tr = FakeTransmission::start().await;
        let db = Db::open_blocking(":memory:").unwrap();
        let root = tempfile::tempdir().unwrap();
        let season = root.path().join("Slime/Season 04");
        std::fs::create_dir_all(&season).unwrap();
        let ctx = ReceiveContext {
            channels: ChannelStore::new(db.clone()),
            settings: SettingsStore::new(db.clone()),
            history: HistoryStore::new(db.clone()),
            revisions: RevisionStore::new(db.clone()),
            commands: CommandStore::new(db.clone()),
            seasons: SeasonStore::new(db.clone()),
            library: LibraryStore::new(db),
            transmission: TransmissionLink {
                url: tr.url().parse().unwrap(),
                http: trss_transmission::http_client(Duration::from_secs(5)).unwrap(),
            },
            http: reqwest::Client::new(),
            rename: RenamePolicy {
                delay: Duration::from_millis(1),
                attempts: ATTEMPTS,
            },
            redactor: Redactor::none(),
        };
        World {
            tr,
            ctx,
            _root: root,
            season,
            redactor: Redactor::none(),
        }
    }

    /// Transmission holds `RELEASE` in the rule folder.
    fn holding(self) -> World {
        self.tr
            .preload(FakeTorrent::new(HASH, RELEASE).in_dir(&self.season).bot());
        self
    }

    /// The job the rule cycle gives a torrent it has just added.
    fn cycle_job(&self) -> RenameJob<'_> {
        RenameJob {
            hash: HASH,
            save_path: &self.season,
            episode: -24,
            mode: RenameMode::Added,
            original: Original::Current,
            note: None,
            redactor: &self.redactor,
        }
    }

    /// The job `다시 받기` gives its torrent, with the original name recorded.
    fn command_job(&self) -> RenameJob<'_> {
        RenameJob {
            original: Original::Recorded {
                command_id: "command".into(),
                name: Some(RELEASE.into()),
                added_before: false,
            },
            ..self.cycle_job()
        }
    }

    async fn rename(&self, job: &RenameJob<'_>) -> RenameResult {
        let mut transmission = self.ctx.transmission.client();
        rename(&self.ctx, &mut transmission, job, &CancellationToken::new()).await
    }

    fn name(&self) -> String {
        self.tr.torrent(HASH).name
    }
}

#[tokio::test]
async fn a_torrent_transmission_gives_no_file_count_for_is_asked_again_on_both_paths() {
    for cycle in [true, false] {
        let w = World::new().await.holding();
        let job = if cycle {
            w.cycle_job()
        } else {
            w.command_job()
        };
        w.tr.omit_file_count(true);
        let answers = w.tr.hold_answer("torrent-get");
        let (result, ()) = tokio::join!(w.rename(&job), async {
            answers.wait_arrived().await;
            w.tr.omit_file_count(false);
            answers.release_all();
        });
        assert_eq!(result, RenameResult::Renamed, "cycle: {cycle}");
        assert_eq!(w.name(), RENAMED);
        assert_eq!(w.tr.calls_of("torrent-get").len(), 2, "cycle: {cycle}");

        // One that never gives it leaves the name after the attempts.
        let w = World::new().await.holding();
        let job = if cycle {
            w.cycle_job()
        } else {
            w.command_job()
        };
        w.tr.omit_file_count(true);
        assert_eq!(
            w.rename(&job).await,
            RenameResult::Kept(NAME_NOT_CHANGED),
            "cycle: {cycle}"
        );
        assert_eq!(w.name(), RELEASE);
        assert_eq!(
            w.tr.calls_of("torrent-get").len(),
            ATTEMPTS as usize,
            "cycle: {cycle}"
        );
        assert!(w.tr.calls_of("torrent-rename-path").is_empty());
    }
}

// The cycle's rename stops as soon as nothing more can come of it, as the
// `다시 받기` one does: one `torrent-get` each, where it used to ask again
// until its attempts ran out.

#[tokio::test]
async fn the_cycles_rename_of_a_torrent_that_is_gone_stops_at_once() {
    let w = World::new().await;
    assert_eq!(w.rename(&w.cycle_job()).await, RenameResult::Unchanged);
    assert_eq!(w.tr.calls_of("torrent-get").len(), 1);
}

#[tokio::test]
async fn the_cycles_rename_of_a_torrent_with_several_files_stops_at_once() {
    let w = World::new().await;
    let mut torrent = FakeTorrent::new(HASH, RELEASE).in_dir(&w.season).bot();
    torrent.file_count = 2;
    w.tr.preload(torrent);
    assert_eq!(
        w.rename(&w.cycle_job()).await,
        RenameResult::Kept(SEVERAL_FILES)
    );
    assert_eq!(w.tr.calls_of("torrent-get").len(), 1);
    assert!(w.tr.calls_of("torrent-rename-path").is_empty());
    assert_eq!(w.name(), RELEASE);
}

#[tokio::test]
async fn a_new_torrent_trname_cannot_name_keeps_its_name_at_once_on_both_paths() {
    for cycle in [true, false] {
        let w = World::new().await;
        // Saved straight into the work folder, without a `Season NN` folder:
        // `trname` has no name for it.
        let work = w.season.parent().unwrap().to_owned();
        w.tr.preload(FakeTorrent::new(HASH, RELEASE).in_dir(&work).bot());
        let job = RenameJob {
            save_path: &work,
            ..if cycle {
                w.cycle_job()
            } else {
                w.command_job()
            }
        };
        assert_eq!(
            w.rename(&job).await,
            RenameResult::Kept(NAME_NOT_DERIVED),
            "cycle: {cycle}"
        );
        assert_eq!(w.tr.calls_of("torrent-get").len(), 1, "cycle: {cycle}");
        assert!(w.tr.calls_of("torrent-remove").is_empty());
        assert_eq!(w.name(), RELEASE);
    }
}

#[tokio::test]
async fn the_cycles_rename_of_a_name_that_is_right_already_stops_at_once() {
    let w = World::new().await;
    w.tr.preload(FakeTorrent::new(HASH, RENAMED).in_dir(&w.season).bot());
    // Without a conversion, the name `trname` gives is the one it has.
    let job = RenameJob {
        episode: 0,
        ..w.cycle_job()
    };
    assert_eq!(w.rename(&job).await, RenameResult::Unchanged);
    assert_eq!(w.tr.calls_of("torrent-get").len(), 1);
    assert!(w.tr.calls_of("torrent-rename-path").is_empty());
    assert_eq!(w.name(), RENAMED);
}

#[tokio::test]
async fn a_rename_whose_answer_was_lost_does_not_convert_the_episode_twice() {
    for cycle in [true, false] {
        let w = World::new().await.holding();
        let job = if cycle {
            w.cycle_job()
        } else {
            w.command_job()
        };
        // Transmission renames the file and the answer does not come back.
        w.tr.break_rename_answer_of(HASH);
        w.rename(&job).await;
        assert_eq!(w.name(), RENAMED, "cycle: {cycle}");
        assert_eq!(
            w.tr.calls_of("torrent-rename-path").len(),
            1,
            "cycle: {cycle}"
        );
    }
}

// --- a name that stays, a torrent met again, a command run again ---------------------------

use trss_core::commands::{Accepted, NewCommand};

impl World {
    /// The job of the rule cycle (`cycle`) or of a command that recorded
    /// `original_name` (`다시 받기`), in `save_path`.
    fn job_in<'a>(
        &'a self,
        cycle: bool,
        save_path: &'a std::path::Path,
        mode: RenameMode,
        original_name: &str,
    ) -> RenameJob<'a> {
        RenameJob {
            save_path,
            mode,
            original: if cycle {
                Original::Current
            } else {
                Original::Recorded {
                    command_id: "command".into(),
                    name: Some(original_name.to_owned()),
                    added_before: false,
                }
            },
            ..self.cycle_job()
        }
    }

    /// How often the torrent was told to rename its file.
    fn renames(&self) -> usize {
        self.tr.calls_of("torrent-rename-path").len()
    }

    /// A command, accepted and claimed as the worker does, whose name is
    /// recorded as the rename reads it.
    async fn claimed_command(&self) -> String {
        let id = "0b7d5a44-6c1e-4c62-9a6a-3f0c1d2e4b55";
        let new = NewCommand {
            id: id.to_owned(),
            kind: "receive_once".to_owned(),
            payload: r#"{"item_id":1}"#.to_owned(),
            subject: Some("1".to_owned()),
        };
        assert!(matches!(
            self.ctx.commands.accept(new, 1_000).await.unwrap(),
            Accepted::Created(_)
        ));
        self.ctx.commands.claim_next(1_000).await.unwrap().unwrap();
        id.to_owned()
    }
}

#[tokio::test]
async fn a_name_that_stays_is_kept_with_its_torrent_on_both_paths() {
    // (what, the file's name, a file in the folder that has the name `trname`
    // would give, the note)
    let cases = [
        (
            "a movie: the CRC32 would read as an episode",
            "[Group] Show Movie (BD 1080p) [ABCD1234].mkv",
            None,
            NAME_NOT_DERIVED,
        ),
        (
            "a batch",
            "[Group] Show Season 1 [BDRip 1920x1080 HEVC FLAC] (01-12).mkv",
            None,
            NAME_NOT_DERIVED,
        ),
        (
            "an episode zero: trname reads no episode from 00",
            "[SubsPlease] Show - 00 (1080p) [ABCD1234].mkv",
            None,
            NAME_NOT_DERIVED,
        ),
        (
            "a name another file has in the folder",
            RELEASE,
            Some(RENAMED),
            NAME_TAKEN,
        ),
    ];
    for cycle in [true, false] {
        for (what, name, taken_by, note) in cases {
            let w = World::new().await;
            w.tr.preload(FakeTorrent::new(HASH, name).in_dir(&w.season).bot());
            if let Some(taken) = taken_by {
                std::fs::write(w.season.join(taken), b"someone else's").unwrap();
            }
            let save_path = w.season.clone();

            let result = w
                .rename(&w.job_in(cycle, &save_path, RenameMode::Added, name))
                .await;

            let which = format!("{what}, cycle: {cycle}");
            assert_eq!(result, RenameResult::Kept(note), "{which}");
            assert_eq!(w.name(), name, "{which}");
            assert_eq!(w.renames(), 0, "{which}");
            assert!(w.tr.calls_of("torrent-remove").is_empty(), "{which}");
            if let Some(taken) = taken_by {
                assert_eq!(
                    std::fs::read(w.season.join(taken)).unwrap(),
                    b"someone else's",
                    "{which}"
                );
            }
        }
    }
}

#[tokio::test]
async fn a_torrent_met_again_is_renamed_only_while_it_is_an_unnamed_release_in_the_rules_folder() {
    // (what, the file's name, the folder it is in, what the rename gives, the
    // name afterwards)
    type Dir = fn(&World) -> std::path::PathBuf;
    let in_season: Dir = |w| w.season.clone();
    let cases: [(&str, &str, Dir, RenameResult, &str); 6] = [
        (
            "a name trname cannot derive one from",
            "Some Special Collection.mkv",
            in_season,
            RenameResult::Unchanged,
            "Some Special Collection.mkv",
        ),
        (
            "a name the rule gave already",
            RENAMED,
            in_season,
            RenameResult::Unchanged,
            RENAMED,
        ),
        (
            "a name given under the folder whose title was written in another case",
            "SLIME S04E38.mkv",
            |w| {
                w.season
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("SLIME/Season 04")
            },
            RenameResult::Unchanged,
            "SLIME S04E38.mkv",
        ),
        (
            "a release another rule received into its own folder",
            RELEASE,
            |w| {
                w.season
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("Other/Season 01")
            },
            RenameResult::Unchanged,
            RELEASE,
        ),
        (
            "a release whose own name ends in SxxEyy, left unrenamed",
            "Tensura S04E62.mkv",
            in_season,
            RenameResult::Renamed,
            RENAMED,
        ),
        (
            "a rename cut short in the rules folder",
            RELEASE,
            in_season,
            RenameResult::Renamed,
            RENAMED,
        ),
    ];
    for cycle in [true, false] {
        for (what, name, dir, expected, afterwards) in cases {
            let w = World::new().await;
            let folder = dir(&w);
            std::fs::create_dir_all(&folder).unwrap();
            w.tr.preload(FakeTorrent::new(HASH, name).in_dir(&folder).bot());
            let save_path = w.season.clone();

            let result = w
                .rename(&w.job_in(cycle, &save_path, RenameMode::Existing, name))
                .await;

            let which = format!("{what}, cycle: {cycle}");
            assert_eq!(result, expected, "{which}");
            assert_eq!(w.name(), afterwards, "{which}");
            assert_eq!(
                w.renames(),
                usize::from(expected == RenameResult::Renamed),
                "{which}"
            );
            assert!(w.tr.calls_of("torrent-remove").is_empty(), "{which}");
        }
    }
}

#[tokio::test]
async fn a_command_run_again_names_a_file_from_the_name_it_recorded_or_found() {
    // (what, the name the command recorded, whether an earlier start put the
    // torrent in, the file's name now, the folder, the episode conversion,
    // the name afterwards)
    #[allow(clippy::type_complexity)]
    let cases: [(&str, Option<&str>, bool, &str, &str, isize, &str); 4] = [
        (
            "no name recorded, the torrent put in before: a name the rule gave is left",
            None,
            true,
            "Sono Bisque Doll S01E24.mkv",
            "Sono Bisque Doll/Season 01",
            12,
            "Sono Bisque Doll S01E24.mkv",
        ),
        (
            "no name recorded, nothing added before: a release in another season's form is converted",
            None,
            false,
            "Sono Bisque Doll S01E25.mkv",
            "Sono Bisque Doll/Season 02",
            -24,
            "Sono Bisque Doll S02E01.mkv",
        ),
        (
            "the name recorded, a cycle renamed the file meanwhile: it is left",
            Some("[Group] Show - 05 [1080p].mkv"),
            true,
            "Show S01E16.mkv",
            "Show/Season 01",
            12,
            "Show S01E16.mkv",
        ),
        (
            "the name recorded, the earlier start did not rename: the recorded name is converted",
            Some("[Group] Show - 05 [1080p].mkv"),
            true,
            "[Group] Show - 05 [1080p].mkv",
            "Show/Season 01",
            12,
            "Show S01E16.mkv",
        ),
    ];
    for (what, recorded, added_before, name, folder, episode, afterwards) in cases {
        let w = World::new().await;
        let save_path = w.season.parent().unwrap().parent().unwrap().join(folder);
        std::fs::create_dir_all(&save_path).unwrap();
        w.tr.preload(FakeTorrent::new(HASH, name).in_dir(&save_path).bot());
        let command_id = w.claimed_command().await;
        if let Some(recorded) = recorded {
            w.ctx
                .commands
                .note_original_name(&command_id, recorded)
                .await
                .unwrap();
        }
        let job = RenameJob {
            save_path: &save_path,
            episode,
            original: Original::Recorded {
                command_id: command_id.clone(),
                name: recorded.map(str::to_owned),
                added_before,
            },
            ..w.cycle_job()
        };

        let result = w.rename(&job).await;

        assert_eq!(w.name(), afterwards, "{what}");
        let renamed = afterwards != name;
        assert_eq!(
            result,
            if renamed {
                RenameResult::Renamed
            } else {
                RenameResult::Unchanged
            },
            "{what}"
        );
        assert_eq!(w.renames(), usize::from(renamed), "{what}");
        // A name the command found is recorded before the rename, for the
        // next start; a name left alone needs none.
        let stored = w.ctx.commands.get(&command_id).await.unwrap().unwrap();
        match (recorded, renamed) {
            (Some(recorded), _) => {
                assert_eq!(stored.original_name.as_deref(), Some(recorded), "{what}")
            }
            (None, true) => assert_eq!(stored.original_name.as_deref(), Some(name), "{what}"),
            (None, false) => assert_eq!(stored.original_name, None, "{what}"),
        }
    }
}
