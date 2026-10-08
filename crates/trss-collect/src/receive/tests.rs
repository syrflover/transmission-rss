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
            underivable: Underivable::Remove,
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
            underivable: Underivable::Keep,
            ..self.cycle_job()
        }
    }

    async fn rename(&self, job: &RenameJob<'_>) -> RenameResult {
        rename(&self.ctx, job, &CancellationToken::new()).await
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
async fn the_cycles_rename_stops_once_it_has_removed_a_torrent_trname_cannot_name() {
    let w = World::new().await;
    // Saved straight into the work folder, without a `Season NN` folder:
    // `trname` has no name for it.
    let work = w.season.parent().unwrap().to_owned();
    w.tr.preload(FakeTorrent::new(HASH, RELEASE).in_dir(&work).bot());
    let job = RenameJob {
        save_path: &work,
        ..w.cycle_job()
    };
    assert_eq!(w.rename(&job).await, RenameResult::Removed);
    assert_eq!(w.tr.calls_of("torrent-get").len(), 1);
    assert_eq!(w.tr.calls_of("torrent-remove").len(), 1);
    assert!(w.tr.torrents().is_empty());
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
