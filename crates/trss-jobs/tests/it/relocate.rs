//! A source's episode mapping changes: what follows it is re-evaluated, and
//! the copies the app applied move once a person confirms the relocation
//! (`docs/specs/library.md`, 자막의 회차 대응; ticket 0071,
//! `trss_jobs::place::relocate`).

use crate::{
    world::{cut_and_restart, Base, Shows, TodoStores},
    Handles,
};
use std::path::{Path, PathBuf};

use tokio_util::sync::CancellationToken;
use trss_core::{Db, DbError};
use trss_jobs::{
    mapping::{self, Saved, UserMapping},
    model::{Chosen, Outcome, PlanState, RemovalState},
    place::{
        records::{self, Confirmed, NewApplied, RowPlacing, StoredChoice},
        relocate::{self, Removal},
        replace::AWAITING_APPROVAL,
    },
    store::{JobDetail, JobError},
    todo::Todo,
    Created, Follow, JobState, NewItem, NewJob, Runner, Wait,
};
use trss_subtitles::{
    fake::{self, FakeSource},
    Sources,
};

const WORK: &str = "w1";
const CREATOR: &str = "제작자";
const SOURCE: &str = "src";
/// A subtitle beside a video that the app did not put there.
const MINE: &str =
    "[Script Info]\n[Events]\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,내 자막\n";

fn video(episode: u32) -> String {
    format!("Season 01/Show S01E{episode:02}.mkv")
}

fn copy(episode: u32) -> String {
    format!("Season 01/Show S01E{episode:02}.ass")
}

struct Setup {
    dir: tempfile::TempDir,
    db: Db,
    store: Handles,
    follow: Follow,
    runner: Runner,
}

impl Setup {
    fn work(&self) -> PathBuf {
        self.dir.path().join("shows/Show")
    }

    fn at(&self, path: &str) -> PathBuf {
        self.work().join(path)
    }

    fn read(&self, path: &str) -> Option<Vec<u8>> {
        std::fs::read(self.at(path)).ok()
    }

    /// What is left in the work folder's temporary folders: where a
    /// relocation renames a copy aside before it removes it, the staging
    /// folder of stores and applies, and the folder of releases before 0153.
    fn temps(&self) -> Vec<String> {
        let mut left = names(&self.at(".trss/aside"));
        left.extend(names(&self.at(".trss/staging")));
        left.extend(names(&self.at(".trss/tmp")));
        left
    }

    async fn sql(&self, sql: String) {
        self.db
            .run(move |c| c.execute_batch(&sql).map_err(DbError::from))
            .await
            .unwrap();
    }

    async fn count(&self, sql: &'static str) -> i64 {
        self.db
            .run(move |c| c.query_row(sql, [], |r| r.get(0)).map_err(DbError::from))
            .await
            .unwrap()
    }

    /// The episode the stored subtitle received from `name` is on.
    async fn stored_episode(&self, name: &'static str) -> Option<i64> {
        self.db
            .run(move |c| {
                c.query_row(
                    "SELECT s.episode FROM subtitle_stored s
                       JOIN subtitle_job_plan p ON p.stored_id = s.id
                      WHERE p.name = ?1 LIMIT 1",
                    [name],
                    |r| r.get(0),
                )
                .map_err(DbError::from)
            })
            .await
            .unwrap()
    }

    /// The stored subtitle received from `name`.
    async fn stored_id(&self, name: &'static str) -> String {
        self.db
            .run(move |c| {
                c.query_row(
                    "SELECT stored_id FROM subtitle_job_plan
                      WHERE name = ?1 AND stored_id IS NOT NULL LIMIT 1",
                    [name],
                    |r| r.get(0),
                )
                .map_err(DbError::from)
            })
            .await
            .unwrap()
    }

    /// The source's relocation jobs, oldest first.
    async fn relocations(&self) -> Vec<String> {
        self.db
            .run(|c| {
                let mut stmt = c.prepare(
                    "SELECT id FROM subtitle_jobs WHERE origin = 'relocate' ORDER BY seq",
                )?;
                let rows = stmt.query_map([], |r| r.get(0))?;
                rows.collect::<Result<Vec<String>, _>>()
                    .map_err(DbError::from)
            })
            .await
            .unwrap()
    }

    /// The source's one relocation job.
    async fn relocation(&self) -> String {
        let mut jobs = self.relocations().await;
        assert_eq!(jobs.len(), 1, "{jobs:?}");
        jobs.remove(0)
    }
}

/// A library with the work `Show` (season 1) whose episodes 2 to 5 have a
/// video, the source `src`, and the user's mapping `−12` for it.
async fn setup() -> Setup {
    let s = unmapped().await;
    remap(&s, -12).await;
    s
}

/// [`setup`] before the source has a mapping.
async fn unmapped() -> Setup {
    let base = Base::new().await;
    base.library(&Shows {
        episodes: (2..=5).collect(),
        source: true,
        ..Shows::default()
    })
    .await;
    let runner = base.runner(Sources::none().with_fake(FakeSource));
    Setup {
        follow: Follow::new(base.db.clone()),
        dir: base.dir,
        db: base.db,
        store: base.store,
        runner,
    }
}

/// The user saves the mapping `offset` for the source.
async fn remap(s: &Setup, offset: i64) {
    let version =
        s.db.run(|c| {
            Ok::<_, DbError>(
                mapping::read_in(c, WORK, 1)?
                    .remove(SOURCE)
                    .map_or(0, |m| m.version),
            )
        })
        .await
        .unwrap();
    let user = UserMapping::new(offset, Vec::new()).unwrap();
    let saved = s
        .follow
        .set_user_mapping(WORK, 1, SOURCE, version, user, 2_000_000)
        .await
        .unwrap();
    assert!(matches!(saved, Saved::Done(Some(_))), "{saved:?}");
}

/// A candidate's job for the fake post `path`, as Anissia's episode
/// `episode`, from the source when `source` says so.
async fn make(s: &Setup, command: &str, episode: &str, path: &str, source: bool) -> String {
    let job = NewJob {
        command_id: command.to_owned(),
        request: format!("{{\"c\":\"{command}\"}}"),
        origin: "pick".to_owned(),
        work_id: Some(WORK.to_owned()),
        season: Some(1),
        movie: None,
        anime_no: Some(7),
        source_id: source.then(|| SOURCE.to_owned()),
        creator: Some(CREATOR.to_owned()),
        revision_of: None,
        revises_attributed: false,
        items: vec![NewItem {
            observation_id: None,
            episode: episode.to_owned(),
            post_url: format!("https://{}{path}", fake::HOST),
            found_at: 500,
        }],
    };
    match s.store.requests.create(job, 900).await.unwrap() {
        Created::Created(id) => id,
        other => panic!("created: {other:?}"),
    }
}

/// The source's candidate of Anissia's episode `episode` (`14`: season
/// episode 2 under `−12`), received and applied.
async fn applied(s: &Setup, command: &str, episode: u32) -> String {
    let id = make(
        s,
        command,
        &episode.to_string(),
        &format!("/ok/Show-{episode}"),
        true,
    )
    .await;
    run(s).await;
    let d = detail(s, &id).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    id
}

async fn run(s: &Setup) {
    s.runner.run_ready(&CancellationToken::new()).await.unwrap();
}

async fn detail(s: &Setup, id: &str) -> JobDetail {
    s.store.views.detail(id).await.unwrap().unwrap()
}

async fn removals(s: &Setup, job: &str) -> Vec<Removal> {
    s.store.place.removals(job).await.unwrap()
}

/// The person's 배치 확인 of the relocation, as the table shows it.
async fn confirm(s: &Setup, job: &str) -> Confirmed {
    let (asked, whole) = s.store.place.placeable(job).await.unwrap().unwrap();
    assert!(whole);
    let placings = asked
        .iter()
        .map(|r| RowPlacing {
            position: r.position,
            episode: r.placed.as_ref().map(|p| p.episode),
            apply: true,
        })
        .collect();
    let planned = removals(s, job)
        .await
        .into_iter()
        .filter(|r| r.state == RemovalState::Planned)
        .map(|r| r.id)
        .collect();
    s.store
        .place
        .confirm_placement(job, placings, planned, None, 3_000_000)
        .await
        .unwrap()
}

fn queued(confirmed: Confirmed) {
    assert!(
        matches!(confirmed, Confirmed::Queued { .. }),
        "{confirmed:?}"
    );
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn waiting_for_placement(d: &JobDetail) {
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Placement)),
        "{:?} {:?}",
        d.row.note,
        d.events
    );
}

#[tokio::test]
async fn a_mapping_change_plans_a_relocation_and_moves_nothing() {
    let s = setup().await;
    applied(&s, "c1", 14).await;
    let before = s.read(&copy(2)).unwrap();
    assert_eq!(before, fake::ass("Show-14"));

    remap(&s, -11).await;

    // Nothing moved on the disk.
    assert_eq!(s.read(&copy(2)), Some(before));
    assert_eq!(s.read(&copy(3)), None);
    // The stored subtitle is on its new episode at once.
    assert_eq!(s.stored_episode("Show-14.ass").await, Some(3));
    // One table: the copy on episode 2 taken off, the subtitle applied on 3.
    let job = s.relocation().await;
    let d = detail(&s, &job).await;
    waiting_for_placement(&d);
    assert_eq!(d.row.origin, "relocate");
    assert_eq!(
        d.row.note.as_deref(),
        Some("회차 대응이 바뀌어 적용본 1개를 옮길 계획을 확인해 주세요")
    );
    let (asked, whole) = s.store.place.placeable(&job).await.unwrap().unwrap();
    assert!(whole);
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].placed.as_ref().map(|p| p.episode), Some(3));
    let off = removals(&s, &job).await;
    assert_eq!(
        off.iter()
            .map(|r| (r.episode, r.path.as_str(), r.state, r.position))
            .collect::<Vec<_>>(),
        [(
            2,
            copy(2).as_str(),
            RemovalState::Planned,
            Some(asked[0].position)
        )]
    );
    // It waits for a person (`회차 확인 필요`) among the placement waits.
    assert!(s
        .store
        .views
        .placement_waits()
        .await
        .unwrap()
        .iter()
        .any(|j| j.id == job));
}

#[tokio::test]
async fn a_relocation_waiting_for_its_table_is_one_to_do_naming_the_subtitle_it_moves_until_confirmed(
) {
    let s = setup().await;
    applied(&s, "c1", 14).await;
    remap(&s, -11).await;
    let job = s.relocation().await;

    let list = TodoStores::new(&s.db).list().await.unwrap();

    assert_eq!(list.count, 1);
    let Todo::PlacementCheck {
        key,
        work,
        origin,
        files,
        reason,
        job_id,
        ..
    } = &list.needs[0]
    else {
        panic!("{:?}", list.needs)
    };
    assert_eq!(key, &format!("placement:{job}"));
    assert_eq!(work.as_ref().map(|w| w.id.as_str()), Some(WORK));
    assert_eq!((origin.as_str(), job_id), ("relocate", &job));
    assert_eq!(files, &["Show-14.ass"]);
    assert_eq!(
        reason.as_deref(),
        Some("회차 대응이 바뀌어 적용본 1개를 옮길 계획을 확인해 주세요")
    );

    queued(confirm(&s, &job).await);
    let list = TodoStores::new(&s.db).list().await.unwrap();
    assert_eq!(list.count, 0);
}

#[tokio::test]
async fn confirming_the_relocation_takes_the_copy_off_and_applies_it_on_the_new_episode() {
    let s = setup().await;
    applied(&s, "c1", 14).await;
    let stored = s.count("SELECT count(*) FROM subtitle_stored").await;
    remap(&s, -11).await;
    let job = s.relocation().await;

    queued(confirm(&s, &job).await);
    run(&s).await;

    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(
        d.row.note.as_deref(),
        Some("옛 회차의 적용본 1개를 지우고 새 회차에 적용했어요")
    );
    assert_eq!(s.read(&copy(2)), None);
    assert_eq!(s.read(&copy(3)), Some(fake::ass("Show-14")));
    // The stored subtitle is the one it was: nothing new was kept.
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_stored").await,
        stored
    );
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_applied WHERE removed_at IS NULL")
            .await,
        1
    );
    assert_eq!(
        s.count(
            "SELECT count(*) FROM subtitle_applied WHERE removed_at IS NOT NULL AND episode = 2"
        )
        .await,
        1
    );
    assert_eq!(removals(&s, &job).await[0].state, RemovalState::Done);
    // The copy went through `.trss/aside/`, which it left empty; nothing
    // uses the `.trss/tmp/` of earlier releases.
    assert!(s.at(".trss/aside").is_dir());
    assert!(s.temps().is_empty(), "{:?}", s.temps());
    assert!(!s.at(".trss/tmp").exists());
    assert!(
        d.events
            .iter()
            .any(|e| e.message == format!("2화 적용본 {}을 지웠어요", copy(2))),
        "{:?}",
        d.events
    );
    let plan = s.store.place.plan(&job).await.unwrap();
    assert_eq!(plan[0].outcome, Some(Outcome::Applied));
}

#[tokio::test]
async fn a_new_episode_with_a_subtitle_waits_for_a_replacements_approval() {
    let s = setup().await;
    applied(&s, "c1", 14).await;
    std::fs::write(s.at(&copy(3)), MINE).unwrap();
    remap(&s, -11).await;
    let job = s.relocation().await;

    queued(confirm(&s, &job).await);
    run(&s).await;

    let d = detail(&s, &job).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Approval)),
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(d.row.note.as_deref(), Some(AWAITING_APPROVAL));
    // The old copy is gone; the episode's subtitle waits for the approval.
    assert_eq!(s.read(&copy(2)), None);
    assert_eq!(s.read(&copy(3)), Some(MINE.as_bytes().to_vec()));
    let plans = s.store.place.replacements(&job).await.unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].plan.state, PlanState::Open);
}

#[tokio::test]
async fn a_format_added_beside_the_first_moves_with_it() {
    let s = setup().await;
    // The release's ASS is applied on episode 2 and its SRT added beside it.
    let id = make(&s, "c1", "14", "/pack/Show-14.ass/Show-14.srt", true).await;
    run(&s).await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Done);
    let srt = s.stored_id("Show-14.srt").await;
    let chose = s
        .store
        .place
        .choose_stored(WORK, &srt, Chosen::Add, 2_500_000)
        .await
        .unwrap();
    assert!(matches!(chose, StoredChoice::Queued { .. }), "{chose:?}");
    run(&s).await;
    let srt_copy = |episode: u32| format!("Season 01/Show S01E{episode:02}.srt");
    assert_eq!(s.read(&srt_copy(2)), Some(fake::bytes_of("Show-14.srt")));

    remap(&s, -11).await;
    let job = s.relocation().await;
    assert_eq!(removals(&s, &job).await.len(), 2);
    queued(confirm(&s, &job).await);
    run(&s).await;

    // Both copies are taken off episode 2 and both are on episode 3.
    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(s.read(&copy(2)), None);
    assert_eq!(s.read(&srt_copy(2)), None);
    assert_eq!(s.read(&copy(3)), Some(fake::bytes_of("Show-14.ass")));
    assert_eq!(s.read(&srt_copy(3)), Some(fake::bytes_of("Show-14.srt")));
}

#[tokio::test]
async fn choosing_a_stored_subtitle_leaves_its_relocation_waiting() {
    let s = setup().await;
    let first = applied(&s, "c1", 14).await;
    remap(&s, -11).await;
    let job = s.relocation().await;
    let stored = s.stored_id("Show-14.ass").await;
    // Something else (a replacement, say) took the copy off episode 2.
    std::fs::remove_file(s.at(&copy(2))).unwrap();
    s.sql("UPDATE subtitle_applied SET removed_at = 2400000".to_owned())
        .await;

    let chose = s
        .store
        .place
        .choose_stored(WORK, &stored, Chosen::Apply, 2_500_000)
        .await
        .unwrap();

    // The job that received it takes the choice, not the relocation.
    assert!(
        matches!(&chose, StoredChoice::Queued { job_id, .. } if *job_id == first),
        "{chose:?}"
    );
    waiting_for_placement(&detail(&s, &job).await);
    // A later save is taken, and ends the relocation: no copy is left.
    remap(&s, -10).await;
    assert_eq!(s.relocation().await, job);
    let d = detail(&s, &job).await;
    assert_eq!(
        (d.row.state, d.row.note.as_deref()),
        (JobState::Done, Some(relocate::NOTHING_TO_MOVE))
    );
}

#[tokio::test]
async fn a_relocation_left_unconfirmed_moves_nothing() {
    let s = setup().await;
    applied(&s, "c1", 14).await;
    remap(&s, -11).await;
    let job = s.relocation().await;

    // The worker's next looks leave it waiting.
    run(&s).await;
    run(&s).await;

    waiting_for_placement(&detail(&s, &job).await);
    assert_eq!(s.read(&copy(2)), Some(fake::ass("Show-14")));
    assert_eq!(s.read(&copy(3)), None);
    assert_eq!(removals(&s, &job).await[0].state, RemovalState::Planned);
}

#[tokio::test]
async fn a_stored_only_subtitle_moves_without_asking() {
    let s = setup().await;
    // Episode 6 (Anissia's 18) has no video: its subtitle is stored and
    // waits for one.
    let id = make(&s, "c1", "18", "/ok/Show-18", true).await;
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Video))
    );
    assert_eq!(s.stored_episode("Show-18.ass").await, Some(6));

    remap(&s, -13).await;

    // The library's line `보관본 있음` is on the new episode, with no
    // question and no relocation.
    assert_eq!(s.stored_episode("Show-18.ass").await, Some(5));
    assert!(s.relocations().await.is_empty());
    // The job looks again: episode 5 has a video, so it is applied there.
    assert_eq!(detail(&s, &id).await.row.state, JobState::Pending);
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(s.read(&copy(5)), Some(fake::ass("Show-18")));
}

#[tokio::test]
async fn an_approval_waiting_for_the_old_target_is_not_used_for_the_new_one() {
    let s = setup().await;
    std::fs::write(s.at(&copy(2)), MINE).unwrap();
    let id = make(&s, "c1", "14", "/ok/Show-14", true).await;
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Approval))
    );
    let old = s
        .store
        .place
        .replacements(&id)
        .await
        .unwrap()
        .remove(0)
        .plan;

    remap(&s, -11).await;

    // The open plan goes stale; the job compares again for episode 3.
    let plans = s.store.place.replacements(&id).await.unwrap();
    assert_eq!(plans[0].plan.state, PlanState::Stale);
    assert_eq!(plans[0].plan.reason.as_deref(), Some(relocate::STALE));
    let decided = s
        .store
        .place
        .decide_replacement(&id, &old.id, old.version, true, 2_500_000)
        .await
        .unwrap();
    assert!(
        !matches!(
            decided,
            trss_jobs::place::replace::records::Decided::Done(_)
        ),
        "{decided:?}"
    );
    assert_eq!(detail(&s, &id).await.row.state, JobState::Pending);
    run(&s).await;

    // Episode 3 has no subtitle: applied there; episode 2's stays the person's.
    let d = detail(&s, &id).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(s.read(&copy(2)), Some(MINE.as_bytes().to_vec()));
    assert_eq!(s.read(&copy(3)), Some(fake::ass("Show-14")));
    assert!(s.relocations().await.is_empty());
}

#[tokio::test]
async fn a_link_the_user_made_does_not_follow_the_mapping() {
    let s = setup().await;
    // A candidate of no known source: its own number, `explicit`, as a
    // person's choice is; then named the source's.
    let id = make(&s, "c1", "2", "/ok/Show-2", false).await;
    run(&s).await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Done);
    s.sql(format!(
        "UPDATE subtitle_stored SET source_id = '{SOURCE}';
         UPDATE subtitle_jobs SET source_id = '{SOURCE}';"
    ))
    .await;

    remap(&s, -11).await;

    assert_eq!(s.stored_episode("Show-2.ass").await, Some(2));
    assert!(s.relocations().await.is_empty());
    assert_eq!(s.read(&copy(2)), Some(fake::ass("Show-2")));
}

/// What puts the stored subtitle received from `name` on its episode, and
/// that episode.
async fn link(s: &Setup, name: &'static str) -> (String, Option<String>, i64) {
    s.db.run(move |c| {
        c.query_row(
            "SELECT s.assignment, s.basis, s.episode FROM subtitle_stored s
               JOIN subtitle_job_plan p ON p.stored_id = s.id
              WHERE p.name = ?1 LIMIT 1",
            [name],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(DbError::from)
    })
    .await
    .unwrap()
}

// A candidate of a source with no mapping goes on its own number until a
// mapping is decided, then follows it: the copy moves once a person confirms
// the relocation.
#[tokio::test]
async fn a_link_made_by_the_same_number_follows_the_mapping_decided_later() {
    let s = unmapped().await;
    let id = applied(&s, "c1", 3).await;
    assert_eq!(
        link(&s, "Show-3.ass").await,
        ("same_number".to_owned(), Some("anissia".to_owned()), 3)
    );
    assert_eq!(s.read(&copy(3)), Some(fake::ass("Show-3")));

    remap(&s, -1).await;

    assert_eq!(
        link(&s, "Show-3.ass").await,
        ("mapped".to_owned(), Some("anissia".to_owned()), 2)
    );
    let job = s.relocation().await;
    let off: Vec<i64> = removals(&s, &job).await.iter().map(|r| r.episode).collect();
    assert_eq!(off, [3]);
    // Nothing moves before the person confirms.
    assert_eq!(s.read(&copy(3)), Some(fake::ass("Show-3")));
    queued(confirm(&s, &job).await);
    run(&s).await;

    assert_eq!(detail(&s, &job).await.row.state, JobState::Done);
    assert_eq!(s.read(&copy(2)), Some(fake::ass("Show-3")));
    assert_eq!(s.read(&copy(3)), None);
    // The candidate's row is a record of what put it there then.
    let plan = s.store.place.plan(&id).await.unwrap();
    assert_eq!(
        plan[0]
            .placed
            .as_ref()
            .map(|p| (p.episode, p.assignment.code())),
        Some((3, "same_number"))
    );
}

// A person who put a candidate's file on an episode in the table chose it: a
// mapping decided later does not move it.
#[tokio::test]
async fn an_episode_a_person_chose_for_a_source_with_no_mapping_stays() {
    let s = unmapped().await;
    // The name says 4, the candidate 3: asked.
    let id = make(&s, "c1", "3", "/pack/Show%20-%2004.ass", true).await;
    run(&s).await;
    let (asked, whole) = s.store.place.placeable(&id).await.unwrap().unwrap();
    assert!(!whole);
    let placings = asked
        .iter()
        .map(|r| RowPlacing {
            position: r.position,
            episode: Some(4),
            apply: true,
        })
        .collect();
    queued(
        s.store
            .place
            .confirm_placement(&id, placings, Vec::new(), None, 1_500_000)
            .await
            .unwrap(),
    );
    run(&s).await;
    assert_eq!(
        link(&s, "Show - 04.ass").await,
        ("explicit".to_owned(), None, 4)
    );

    remap(&s, -1).await;

    assert_eq!(
        link(&s, "Show - 04.ass").await,
        ("explicit".to_owned(), None, 4)
    );
    assert!(s.relocations().await.is_empty());
    assert!(s.read(&copy(4)).is_some());
}

// A mapping decided to keep a same-number link on its episode names the same
// target: the replacement a person approves is carried out.
#[tokio::test]
async fn an_approval_holds_when_the_mapping_keeps_the_same_number() {
    let s = unmapped().await;
    std::fs::write(s.at(&copy(3)), MINE).unwrap();
    let job = make(&s, "c1", "3", "/ok/Show-3", true).await;
    run(&s).await;
    assert_eq!(detail(&s, &job).await.row.wait, Some(Wait::Approval));

    remap(&s, 0).await;

    assert_eq!(
        link(&s, "Show-3.ass").await,
        ("mapped".to_owned(), Some("anissia".to_owned()), 3)
    );
    let row = s.store.place.plan(&job).await.unwrap().remove(0);
    assert_eq!(
        row.placed.map(|p| (p.episode, p.assignment.code())),
        Some((3, "mapped"))
    );
    assert_eq!(detail(&s, &job).await.row.wait, Some(Wait::Approval));
    let plan = s
        .store
        .place
        .replacements(&job)
        .await
        .unwrap()
        .remove(0)
        .plan;
    s.store
        .place
        .decide_replacement(&job, &plan.id, plan.version, true, 3_500_000)
        .await
        .unwrap();
    run(&s).await;

    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(s.read(&copy(3)), Some(fake::ass("Show-3")));
}

#[tokio::test]
async fn a_change_before_the_confirmation_plans_anew_and_refuses_the_old_table() {
    let s = setup().await;
    applied(&s, "c1", 14).await;
    remap(&s, -11).await;
    let job = s.relocation().await;
    let (asked, _) = s.store.place.placeable(&job).await.unwrap().unwrap();
    let shown: Vec<String> = removals(&s, &job).await.into_iter().map(|r| r.id).collect();

    remap(&s, -10).await;

    // The same job, planned for episode 4.
    assert_eq!(s.relocation().await, job);
    let (now_asked, _) = s.store.place.placeable(&job).await.unwrap().unwrap();
    assert_eq!(now_asked[0].placed.as_ref().map(|p| p.episode), Some(4));
    assert_ne!(now_asked[0].position, asked[0].position);
    // The table the person saw is not the plan any more.
    let stale = s
        .store
        .place
        .confirm_placement(
            &job,
            vec![RowPlacing {
                position: asked[0].position,
                episode: Some(3),
                apply: true,
            }],
            shown,
            None,
            3_000_000,
        )
        .await
        .unwrap();
    assert!(matches!(stale, Confirmed::Stale), "{stale:?}");
    // The rows as they are now, without the removals they come with.
    let rows_only = s
        .store
        .place
        .confirm_placement(
            &job,
            vec![RowPlacing {
                position: now_asked[0].position,
                episode: Some(4),
                apply: true,
            }],
            Vec::new(),
            None,
            3_000_000,
        )
        .await
        .unwrap();
    assert!(matches!(rows_only, Confirmed::Stale), "{rows_only:?}");
    // A row moved off its planned episode is refused.
    let planned: Vec<String> = removals(&s, &job).await.into_iter().map(|r| r.id).collect();
    let moved = s
        .store
        .place
        .confirm_placement(
            &job,
            vec![RowPlacing {
                position: now_asked[0].position,
                episode: Some(5),
                apply: true,
            }],
            planned,
            None,
            3_000_000,
        )
        .await
        .unwrap();
    assert_eq!(
        moved,
        Confirmed::Refused("재배치는 표에 적힌 회차 그대로 확인해요.".to_owned())
    );
    // The new table moves the copy to episode 4.
    queued(confirm(&s, &job).await);
    run(&s).await;
    assert_eq!(detail(&s, &job).await.row.state, JobState::Done);
    assert_eq!(s.read(&copy(2)), None);
    assert_eq!(s.read(&copy(3)), None);
    assert_eq!(s.read(&copy(4)), Some(fake::ass("Show-14")));
}

#[tokio::test]
async fn a_relocation_moved_back_before_its_confirmation_ends_with_nothing_moved() {
    let s = setup().await;
    applied(&s, "c1", 14).await;
    remap(&s, -11).await;
    let job = s.relocation().await;

    remap(&s, -12).await;

    let d = detail(&s, &job).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(d.row.note.as_deref(), Some(relocate::NOTHING_TO_MOVE));
    assert!(removals(&s, &job).await.is_empty());
    assert_eq!(s.read(&copy(2)), Some(fake::ass("Show-14")));
    assert_eq!(s.stored_episode("Show-14.ass").await, Some(2));
    // A person's confirmation of the table they saw is refused.
    assert!(!matches!(
        confirm_rows(&s, &job).await,
        Confirmed::Queued { .. }
    ));
}

/// A confirmation with no rows and no removals.
async fn confirm_rows(s: &Setup, job: &str) -> Confirmed {
    s.store
        .place
        .confirm_placement(job, Vec::new(), Vec::new(), None, 3_000_000)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_copy_a_person_changed_stays_where_it_is() {
    let s = setup().await;
    applied(&s, "c1", 14).await;
    remap(&s, -11).await;
    let job = s.relocation().await;
    queued(confirm(&s, &job).await);
    std::fs::write(s.at(&copy(2)), MINE).unwrap();

    run(&s).await;

    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(
        d.row.note.as_deref(),
        Some("옛 회차의 적용본 1개를 그대로 뒀어요")
    );
    let off = removals(&s, &job).await;
    assert_eq!(off[0].state, RemovalState::Kept);
    assert_eq!(
        off[0].reason.as_deref(),
        Some("적용한 뒤 바뀐 파일이라 그대로 뒀어요")
    );
    assert_eq!(s.read(&copy(2)), Some(MINE.as_bytes().to_vec()));
    assert_eq!(s.read(&copy(3)), Some(fake::ass("Show-14")));
    // Looked at where it is: it was not renamed aside.
    assert!(!s.at(".trss/aside").exists());
}

#[tokio::test]
async fn a_relocation_that_only_takes_copies_off_says_it_took_them_off() {
    let s = setup().await;
    applied(&s, "c1", 14).await;
    remap(&s, -11).await;
    let first = s.relocation().await;
    queued(confirm(&s, &first).await);
    // A person's change keeps episode 2's copy; episode 3 gets the subtitle.
    std::fs::write(s.at(&copy(2)), MINE).unwrap();
    run(&s).await;
    assert_eq!(removals(&s, &first).await[0].state, RemovalState::Kept);
    // The copy is the applied bytes again, and the mapping is saved again.
    std::fs::write(s.at(&copy(2)), fake::ass("Show-14")).unwrap();
    remap(&s, -11).await;

    // The table has the removal alone: episode 3 has the subtitle already.
    let jobs = s.relocations().await;
    assert_eq!(jobs.len(), 2, "{jobs:?}");
    let job = jobs[1].clone();
    let (asked, _) = s.store.place.placeable(&job).await.unwrap().unwrap();
    assert!(asked.is_empty(), "{asked:?}");
    let off = removals(&s, &job).await;
    assert_eq!(
        off.iter()
            .map(|r| (r.episode, r.position))
            .collect::<Vec<_>>(),
        [(2, None)]
    );
    queued(confirm(&s, &job).await);
    run(&s).await;

    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(
        d.row.note.as_deref(),
        Some("옛 회차의 적용본 1개를 지웠어요")
    );
    assert_eq!(s.read(&copy(2)), None);
    assert_eq!(s.read(&copy(3)), Some(fake::ass("Show-14")));
}

/// The episodes the relocation's table applies its rows on.
async fn planned_on(s: &Setup, job: &str) -> Vec<Option<i64>> {
    let (asked, _) = s.store.place.placeable(job).await.unwrap().unwrap();
    asked
        .iter()
        .map(|r| r.placed.as_ref().map(|p| p.episode))
        .collect()
}

#[tokio::test]
async fn a_copy_a_cut_run_moved_aside_is_applied_again_when_its_episode_came_back() {
    let s = setup().await;
    applied(&s, "c1", 14).await;
    remap(&s, -11).await;
    let first = s.relocation().await;
    queued(confirm(&s, &first).await);
    // A person's change keeps episode 2's copy; episode 3 gets the subtitle.
    std::fs::write(s.at(&copy(2)), MINE).unwrap();
    run(&s).await;
    std::fs::write(s.at(&copy(2)), fake::ass("Show-14")).unwrap();
    remap(&s, -11).await;
    let second = s.relocations().await[1].clone();
    queued(confirm(&s, &second).await);
    // A run renamed episode 2's copy aside and was cut short before it
    // recorded anything; the mapping moves back before it is restarted.
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'running', wait = NULL WHERE id = '{second}'"
    ))
    .await;
    let aside = s.at(".trss/aside/cut");
    std::fs::create_dir_all(aside.parent().unwrap()).unwrap();
    std::fs::rename(s.at(&copy(2)), &aside).unwrap();
    cut_and_restart(&s.store, &second).await;
    remap(&s, -12).await;

    // Episode 2's copy is on record there: the new plan only takes episode
    // 3's off.
    let jobs = s.relocations().await;
    assert_eq!(jobs.len(), 3, "{jobs:?}");
    let third = jobs[2].clone();
    assert_eq!(planned_on(&s, &third).await, []);

    // Its removal done again finds it gone, and the record that ends it plans
    // to apply the subtitle on episode 2 again.
    run(&s).await;
    let off = removals(&s, &second).await;
    assert_eq!(off[0].state, RemovalState::Done, "{off:?}");
    assert_eq!(planned_on(&s, &third).await, [Some(2)]);
    assert_eq!(
        removals(&s, &third)
            .await
            .iter()
            .map(|r| r.episode)
            .collect::<Vec<_>>(),
        [3]
    );
    queued(confirm(&s, &third).await);
    run(&s).await;

    let d = detail(&s, &third).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(s.read(&copy(2)), Some(fake::ass("Show-14")));
    assert_eq!(s.read(&copy(3)), None);
    // What the cut run moved aside stays there.
    assert_eq!(std::fs::read(&aside).unwrap(), fake::ass("Show-14"));
}

#[tokio::test]
async fn a_copy_whose_device_number_changed_since_it_was_applied_is_taken_off() {
    // A file system mounted again (after a restart of the machine) may give
    // the same file another device number; its bytes are what say it is the
    // applied copy.
    let s = setup().await;
    applied(&s, "c1", 14).await;
    s.sql("UPDATE subtitle_applied SET object = '999999:' || object".to_owned())
        .await;
    remap(&s, -11).await;
    let job = s.relocation().await;
    queued(confirm(&s, &job).await);

    run(&s).await;

    let off = removals(&s, &job).await;
    assert_eq!(off[0].state, RemovalState::Done, "{:?}", off[0].reason);
    assert_eq!(s.read(&copy(2)), None);
    assert_eq!(s.read(&copy(3)), Some(fake::ass("Show-14")));
}

#[tokio::test]
async fn a_shift_by_one_episode_moves_every_copy_with_no_approval() {
    let s = setup().await;
    applied(&s, "c1", 14).await;
    applied(&s, "c2", 15).await;
    remap(&s, -11).await;
    let job = s.relocation().await;
    assert_eq!(removals(&s, &job).await.len(), 2);

    queued(confirm(&s, &job).await);
    run(&s).await;

    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert!(s.store.place.replacements(&job).await.unwrap().is_empty());
    assert_eq!(s.read(&copy(2)), None);
    assert_eq!(s.read(&copy(3)), Some(fake::ass("Show-14")));
    assert_eq!(s.read(&copy(4)), Some(fake::ass("Show-15")));
}

#[tokio::test]
async fn a_mapping_saved_after_the_confirmation_keeps_what_is_back_in_place() {
    let s = setup().await;
    applied(&s, "c1", 14).await;
    remap(&s, -11).await;
    let job = s.relocation().await;
    queued(confirm(&s, &job).await);

    // The person moves the mapping back before the worker takes the job.
    remap(&s, -12).await;
    run(&s).await;

    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    let off = removals(&s, &job).await;
    assert_eq!(off[0].state, RemovalState::Kept, "{off:?}");
    let plan = s.store.place.plan(&job).await.unwrap();
    assert_eq!(plan[0].outcome, Some(Outcome::Applied));
    assert_eq!(plan[0].note.as_deref(), Some(relocate::ALREADY_APPLIED));
    // Episode 2 keeps its copy; nothing went on 3.
    assert_eq!(s.read(&copy(2)), Some(fake::ass("Show-14")));
    assert_eq!(s.read(&copy(3)), None);
    // No second relocation is planned.
    assert_eq!(s.relocations().await, [job]);
}

#[tokio::test]
async fn a_run_whose_rows_a_mapping_change_moved_goes_back_in_line() {
    let s = setup().await;
    std::fs::write(s.at(&copy(2)), MINE).unwrap();
    let id = make(&s, "c1", "14", "/ok/Show-14", true).await;
    run(&s).await;
    // A worker runs the job when the mapping changes.
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'running', wait = NULL WHERE id = '{id}'"
    ))
    .await;
    remap(&s, -11).await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Running);

    // Its run ends waiting for the approval it counted: back in line.
    let back = s
        .store
        .run
        .settle(
            &id,
            JobState::Waiting,
            Some(Wait::Approval),
            Some(AWAITING_APPROVAL.to_owned()),
            2_500_000,
        )
        .await
        .unwrap();
    assert_eq!(back, Some(relocate::REMAPPED));
    let d = detail(&s, &id).await;
    assert_eq!(
        (d.row.state, d.row.note.as_deref()),
        (JobState::Pending, Some(relocate::REMAPPED))
    );
    // Once: the next run's end stands.
    run(&s).await;
    let d = detail(&s, &id).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(s.read(&copy(3)), Some(fake::ass("Show-14")));
}

#[tokio::test]
async fn an_interrupted_job_whose_rows_a_mapping_change_moved_goes_back_in_line_after_its_restarted_run(
) {
    let s = setup().await;
    std::fs::write(s.at(&copy(2)), MINE).unwrap();
    let id = make(&s, "c1", "14", "/ok/Show-14", true).await;
    run(&s).await;
    // The process stopped while it ran the job, and the next start interrupted
    // it; the mapping changes before a person restarts it.
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'interrupted', wait = NULL WHERE id = '{id}'"
    ))
    .await;
    remap(&s, -11).await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Interrupted);

    // The run after the restart ends waiting for the approval it counted:
    // back in line, as a run the change came during.
    assert_eq!(
        s.store.requests.restart(&id, 2_400_000).await.unwrap(),
        trss_jobs::Restarted::Restarted
    );
    let back = s
        .store
        .run
        .settle(
            &id,
            JobState::Waiting,
            Some(Wait::Approval),
            Some(AWAITING_APPROVAL.to_owned()),
            2_500_000,
        )
        .await
        .unwrap();
    assert_eq!(back, Some(relocate::REMAPPED));
}

#[tokio::test]
async fn a_copy_recorded_after_a_mapping_change_moved_its_subtitle_is_planned_to_move() {
    let s = setup().await;
    let id = applied(&s, "c1", 14).await;
    // The run publishes the copy on episode 2 and the mapping moves the
    // stored subtitle to 3 before the copy is recorded: no copy is on record
    // for the change to move.
    s.sql(format!(
        "UPDATE subtitle_applied SET removed_at = 1;
         UPDATE subtitle_job_plan SET outcome = NULL, applied_id = NULL WHERE job_id = '{id}';"
    ))
    .await;
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'running', wait = NULL WHERE id = '{id}'"
    ))
    .await;
    remap(&s, -11).await;
    assert!(s.relocations().await.is_empty());
    let stored_id = s.stored_id("Show-14.ass").await;
    let bytes = fake::ass("Show-14");
    let published = records::PublishedCopy {
        job_id: id.clone(),
        position: 0,
        video: Some(video(2)),
        path: copy(2),
        size: bytes.len() as u64,
        sha256: {
            use sha2::{Digest, Sha256};
            trss_jobs::area::hex(&Sha256::digest(&bytes))
        },
        object: trss_jobs::area::object_of(&std::fs::metadata(s.at(&copy(2))).unwrap()),
    };
    let copy_id =
        s.db.run(move |c| -> Result<String, JobError> {
            let copy = NewApplied {
                work_id: WORK.to_owned(),
                stored_id,
                season: 1,
                episode: 2,
            };
            records::applied(c, &published, &copy, None, 2_100_000)
        })
        .await
        .unwrap();

    // Recording it plans its relocation, and the row says where it went.
    let plan = s.store.place.plan(&id).await.unwrap();
    assert_eq!(
        (plan[0].outcome, plan[0].placed.as_ref().map(|p| p.episode)),
        (Some(Outcome::Applied), Some(2))
    );
    let job = s.relocation().await;
    let off = removals(&s, &job).await;
    assert_eq!(
        off.iter()
            .map(|r| (r.applied_id.as_str(), r.episode))
            .collect::<Vec<_>>(),
        [(copy_id.as_str(), 2)]
    );
    queued(confirm(&s, &job).await);
    run(&s).await;
    assert_eq!(s.read(&copy(2)), None);
    assert_eq!(s.read(&copy(3)), Some(fake::ass("Show-14")));
}

// ---------------------------------------------------------------------------
// A run cut short while taking a copy off

/// A relocation confirmed, with its one removal.
async fn confirmed(s: &Setup) -> (String, Removal) {
    applied(s, "c1", 14).await;
    remap(s, -11).await;
    let job = s.relocation().await;
    queued(confirm(s, &job).await);
    let removal = removals(s, &job).await.remove(0);
    (job, removal)
}

/// The job's run cut short, as the process that ran it stopping leaves it,
/// and a person's `다시 시작`.
async fn cut(s: &Setup, job: &str) {
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'running', wait = NULL WHERE id = '{job}'"
    ))
    .await;
    cut_and_restart(&s.store, job).await;
}

/// The removal held, as one whose outcome the run could not confirm is.
async fn hold_removal(s: &Setup, removal: &Removal) {
    s.sql(format!(
        "UPDATE subtitle_relocations
            SET state = 'held', reason = '{}을 지우지 못했어요'
          WHERE id = '{}'",
        removal.path, removal.id
    ))
    .await;
}

/// The removal left as a release before 0153 left one cut short: `state`
/// (`intended` or `set_aside`) with the folder and the place aside it
/// recorded.
async fn leave_as_an_earlier_release(s: &Setup, removal: &Removal, state: &str) {
    let folder = s.work().to_string_lossy().into_owned();
    s.sql(format!(
        "UPDATE subtitle_relocations
            SET state = '{state}', folder = '{folder}', aside = '.trss/tmp/{}.aside'
          WHERE id = '{}'",
        removal.id, removal.id
    ))
    .await;
}

/// Where a release before 0153 renamed the removal's copy aside.
fn earlier_aside(s: &Setup, removal: &Removal) -> PathBuf {
    s.at(&format!(".trss/tmp/{}.aside", removal.id))
}

#[tokio::test]
async fn a_copy_whose_stored_file_is_missing_stays_where_it_is() {
    // The removal keeps no copy of the bytes: the stored file holds them.
    let s = setup().await;
    let (job, _) = confirmed(&s).await;
    let kept: String =
        s.db.run(|c| {
            c.query_row(
                "SELECT a.relative_path FROM subtitle_assets a
                   JOIN subtitle_stored s ON s.subtitle_asset_id = a.id",
                [],
                |r| r.get(0),
            )
            .map_err(DbError::from)
        })
        .await
        .unwrap();
    std::fs::remove_file(s.at(&kept)).unwrap();

    run(&s).await;

    let off = removals(&s, &job).await;
    assert_eq!(
        (off[0].state, off[0].reason.as_deref()),
        (RemovalState::Kept, Some(relocate::STORED_MISSING))
    );
    assert_eq!(s.read(&copy(2)), Some(fake::ass("Show-14")));
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_applied WHERE episode = 2 AND removed_at IS NULL")
            .await,
        1
    );
}

#[tokio::test]
async fn a_removal_cut_before_its_rename_takes_the_copy_off_when_it_is_done_again() {
    let s = setup().await;
    let (job, _) = confirmed(&s).await;
    cut(&s, &job).await;

    run(&s).await;

    assert_eq!(detail(&s, &job).await.row.state, JobState::Done);
    assert_eq!(removals(&s, &job).await[0].state, RemovalState::Done);
    assert_eq!(s.read(&copy(2)), None);
    assert_eq!(s.read(&copy(3)), Some(fake::ass("Show-14")));
    assert!(s.temps().is_empty(), "{:?}", s.temps());
}

#[tokio::test]
async fn a_removal_cut_after_its_rename_or_its_removal_ends_done_and_leaves_what_is_aside() {
    for left in ["renamed", "removed"] {
        let s = setup().await;
        let (job, _) = confirmed(&s).await;
        // The run renamed the copy into `.trss/aside/`, and maybe removed it
        // there, and was cut short before it recorded anything.
        let aside = s.at(".trss/aside/cut");
        std::fs::create_dir_all(aside.parent().unwrap()).unwrap();
        std::fs::rename(s.at(&copy(2)), &aside).unwrap();
        if left == "removed" {
            std::fs::remove_file(&aside).unwrap();
        }
        cut(&s, &job).await;

        run(&s).await;

        let d = detail(&s, &job).await;
        assert_eq!(
            d.row.state,
            JobState::Done,
            "{left}: {:?} {:?}",
            d.row.note,
            d.events
        );
        assert_eq!(removals(&s, &job).await[0].state, RemovalState::Done);
        assert!(
            d.events
                .iter()
                .any(|e| e.message == format!("2화 적용본 {}은 이미 없어요", copy(2))),
            "{left}: {:?}",
            d.events
        );
        assert_eq!(
            s.count(
                "SELECT count(*) FROM subtitle_applied WHERE episode = 2 AND removed_at IS NULL"
            )
            .await,
            0,
            "{left}"
        );
        assert_eq!(s.read(&copy(2)), None, "{left}");
        assert_eq!(s.read(&copy(3)), Some(fake::ass("Show-14")), "{left}");
        // The app never removes what a cut run left aside.
        let kept = match left {
            "renamed" => Some(fake::ass("Show-14")),
            _ => None,
        };
        assert_eq!(std::fs::read(&aside).ok(), kept, "{left}");
    }
}

#[tokio::test]
async fn a_removal_an_earlier_release_left_under_way_is_kept_and_not_planned_again() {
    for state in ["intended", "set_aside"] {
        let s = setup().await;
        let (job, removal) = confirmed(&s).await;
        // An earlier release recorded the removal and was cut short, before
        // the rename (`intended`) or after it (`set_aside`), and the job was
        // interrupted when it started again.
        leave_as_an_earlier_release(&s, &removal, state).await;
        if state == "set_aside" {
            std::fs::create_dir_all(s.at(".trss/tmp")).unwrap();
            std::fs::rename(s.at(&copy(2)), earlier_aside(&s, &removal)).unwrap();
        }
        cut(&s, &job).await;

        run(&s).await;

        let d = detail(&s, &job).await;
        assert_eq!(
            d.row.state,
            JobState::Done,
            "{state}: {:?} {:?}",
            d.row.note,
            d.events
        );
        assert_eq!(
            d.row.note.as_deref(),
            Some("옛 회차의 적용본 1개를 그대로 뒀어요"),
            "{state}"
        );
        let off = removals(&s, &job).await;
        assert_eq!(
            (off[0].state, off[0].reason.as_deref()),
            (
                RemovalState::Kept,
                Some(relocate::LEFT_BY_AN_EARLIER_RELEASE)
            ),
            "{state}"
        );
        // Its record and its files are as the earlier release left them.
        assert_eq!(
            s.count("SELECT count(*) FROM subtitle_relocations WHERE state IN ('intended', 'set_aside')")
                .await,
            1,
            "{state}"
        );
        let (on_path, aside) = match state {
            "intended" => (Some(fake::ass("Show-14")), None),
            _ => (None, Some(fake::ass("Show-14"))),
        };
        assert_eq!(s.read(&copy(2)), on_path, "{state}");
        assert_eq!(
            std::fs::read(earlier_aside(&s, &removal)).ok(),
            aside,
            "{state}"
        );
        assert_eq!(s.read(&copy(3)), Some(fake::ass("Show-14")), "{state}");

        // A later mapping change does not plan its copy again.
        remap(&s, -10).await;
        let later = s.relocations().await.pop().unwrap();
        assert_ne!(later, job);
        assert_eq!(
            removals(&s, &later)
                .await
                .iter()
                .map(|r| r.episode)
                .collect::<Vec<_>>(),
            [3],
            "{state}"
        );
    }
}

#[tokio::test]
async fn a_held_removal_holds_only_the_rows_it_touches() {
    let s = setup().await;
    applied(&s, "c1", 14).await;
    applied(&s, "c2", 15).await;
    applied(&s, "c3", 16).await;
    remap(&s, -11).await;
    let job = s.relocation().await;
    queued(confirm(&s, &job).await);
    // Episode 3's removal is held: what became of its copy is not known.
    let three = removals(&s, &job)
        .await
        .into_iter()
        .find(|r| r.episode == 3)
        .unwrap();
    hold_removal(&s, &three).await;

    run(&s).await;

    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Held,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    let off: Vec<(i64, RemovalState)> = removals(&s, &job)
        .await
        .iter()
        .map(|r| (r.episode, r.state))
        .collect();
    assert_eq!(
        off,
        [
            (2, RemovalState::Done),
            (3, RemovalState::Held),
            (4, RemovalState::Done)
        ]
    );
    // Episode 3 may still have its copy, and episode 4's subtitle is the one
    // whose copy is held: both wait. Episode 5's goes on.
    let rows: Vec<(Option<i64>, Option<Outcome>, Option<String>)> = s
        .store
        .place
        .plan(&job)
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.placed.map(|p| p.episode), r.outcome, r.note))
        .collect();
    assert_eq!(
        rows,
        [
            (
                Some(3),
                Some(Outcome::Held),
                Some(relocate::COPY_HERE_HELD.to_owned())
            ),
            (
                Some(4),
                Some(Outcome::Held),
                Some(relocate::OLD_COPY_HELD.to_owned())
            ),
            (Some(5), Some(Outcome::Applied), None),
        ]
    );
    assert_eq!(s.read(&copy(2)), None);
    assert_eq!(s.read(&copy(3)), Some(fake::ass("Show-15")));
    assert_eq!(s.read(&copy(4)), None);
    assert_eq!(s.read(&copy(5)), Some(fake::ass("Show-16")));
}

#[tokio::test]
async fn a_relocation_held_for_its_starts_keeps_the_copies_it_did_not_take_off() {
    let s = setup().await;
    applied(&s, "c1", 14).await;
    applied(&s, "c2", 15).await;
    remap(&s, -11).await;
    let job = s.relocation().await;
    queued(confirm(&s, &job).await);

    s.store
        .run
        .hold_stuck(&job, "거듭 중단됐어요".to_owned(), 4_000_000)
        .await
        .unwrap();

    assert_eq!(detail(&s, &job).await.row.state, JobState::Held);
    let off: Vec<(i64, RemovalState, Option<String>)> = removals(&s, &job)
        .await
        .into_iter()
        .map(|r| (r.episode, r.state, r.reason))
        .collect();
    let note = Some("거듭 중단됐어요".to_owned());
    assert_eq!(
        off,
        [
            (2, RemovalState::Kept, note.clone()),
            (3, RemovalState::Kept, note)
        ]
    );
    // A later relocation may move them.
    remap(&s, -10).await;
    let later = s.relocations().await.pop().unwrap();
    assert_eq!(
        removals(&s, &later)
            .await
            .iter()
            .map(|r| r.episode)
            .collect::<Vec<_>>(),
        [2, 3]
    );
}

#[tokio::test]
async fn a_held_removal_waits_for_the_approval_another_row_needs() {
    let s = setup().await;
    applied(&s, "c1", 14).await;
    applied(&s, "c2", 16).await;
    // Episode 3 has a subtitle a person put there.
    std::fs::write(s.at(&copy(3)), MINE).unwrap();
    remap(&s, -11).await;
    let job = s.relocation().await;
    queued(confirm(&s, &job).await);
    let four = removals(&s, &job)
        .await
        .into_iter()
        .find(|r| r.episode == 4)
        .unwrap();
    hold_removal(&s, &four).await;

    run(&s).await;

    // Episode 4's removal is held, but episode 3's replacement waits for a
    // person first: held, the job would never carry it out.
    let d = detail(&s, &job).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Approval)),
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    let plan = s
        .store
        .place
        .replacements(&job)
        .await
        .unwrap()
        .remove(0)
        .plan;
    s.store
        .place
        .decide_replacement(&job, &plan.id, plan.version, true, 3_500_000)
        .await
        .unwrap();
    assert_eq!(detail(&s, &job).await.row.state, JobState::Pending);
    run(&s).await;

    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Held,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(s.read(&copy(2)), None);
    assert_eq!(s.read(&copy(3)), Some(fake::ass("Show-14")));
    assert_eq!(s.read(&copy(5)), None);
}

#[tokio::test]
async fn a_held_removal_waits_for_the_work_folder_an_approved_row_needs() {
    let s = setup().await;
    applied(&s, "c1", 14).await;
    applied(&s, "c2", 16).await;
    std::fs::write(s.at(&copy(3)), MINE).unwrap();
    remap(&s, -11).await;
    let job = s.relocation().await;
    queued(confirm(&s, &job).await);
    let four = removals(&s, &job)
        .await
        .into_iter()
        .find(|r| r.episode == 4)
        .unwrap();
    hold_removal(&s, &four).await;
    run(&s).await;
    let plan = s
        .store
        .place
        .replacements(&job)
        .await
        .unwrap()
        .remove(0)
        .plan;
    s.store
        .place
        .decide_replacement(&job, &plan.id, plan.version, true, 3_500_000)
        .await
        .unwrap();
    // The share is not mounted when the approved replacement's turn comes.
    let away = s.dir.path().join("away");
    std::fs::rename(s.work(), &away).unwrap();

    run(&s).await;

    // Held now, the approved replacement would never be carried out.
    let d = detail(&s, &job).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Video)),
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    // Back, and put in line as `requeue_awaiting_video` does.
    std::fs::rename(&away, s.work()).unwrap();
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'pending', wait = NULL, note = NULL WHERE id = '{job}'"
    ))
    .await;
    run(&s).await;

    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Held,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(s.read(&copy(3)), Some(fake::ass("Show-14")));
}

#[tokio::test]
async fn a_copy_prepared_for_the_old_episode_is_not_published_there() {
    let s = setup().await;
    // Stored while episode 2 had no video, so its row waits to be applied.
    let ep2 = s.at(&video(2));
    std::fs::remove_file(&ep2).unwrap();
    let id = make(&s, "c1", "14", "/ok/Show-14", true).await;
    run(&s).await;
    assert_eq!(
        s.store.place.plan(&id).await.unwrap()[0].outcome,
        Some(Outcome::NoVideo)
    );
    std::fs::write(&ep2, b"video").unwrap();
    // A run cut short left the copy for episode 2 staged, and the row to
    // apply.
    let bytes = fake::ass("Show-14");
    std::fs::create_dir_all(s.at(".trss/staging")).unwrap();
    std::fs::write(s.at(".trss/staging/t1"), &bytes).unwrap();
    s.sql(format!(
        "UPDATE subtitle_job_plan SET outcome = NULL, note = NULL WHERE job_id = '{id}';
         UPDATE subtitle_jobs SET state = 'running', wait = NULL WHERE id = '{id}'"
    ))
    .await;
    cut_and_restart(&s.store, &id).await;

    // The mapping changes before the run is done again.
    remap(&s, -11).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert_eq!(s.read(&copy(2)), None);
    assert_eq!(s.read(&copy(3)), Some(bytes));
    assert!(s.temps().is_empty(), "{:?}", s.temps());
}
