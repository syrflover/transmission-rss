//! Deciding the replacements of several episodes of one job at once
//! (`docs/specs/subtitles.md`, 교체 비교와 승인; `docs/tickets/0070`).
//!
//! The work `Show` (season 1) has a video for each of episodes 1 to 12. A
//! first job applies a subtitle to all of them; a second job of the same
//! creator brings a revision of each (`/ok/Show-NNv2`) and waits for the
//! person with twelve open plans.

use crate::{
    world::{Base, Shows},
    Handles,
};
use std::{os::unix::fs::PermissionsExt, path::PathBuf};

use tokio_util::sync::CancellationToken;
use trss_core::Db;
use trss_jobs::{
    model::{Outcome, PlanState},
    place::replace::{
        records::{Decided, Decision, Plan},
        NEW_REVISION,
    },
    store::{JobDetail, DECIDED},
    Created, JobState, NewItem, NewJob, Runner, Wait,
};
use trss_subtitles::{
    fake::{self, FakeSource},
    Sources,
};

const WORK: &str = "w1";
const CREATOR: &str = "제작자";
const EPISODES: std::ops::RangeInclusive<u32> = 1..=12;
/// The episode whose video is in a folder of its own, so its folder alone
/// can be made unwritable.
const APART: u32 = 12;
const DECIDED_AT: i64 = 5_000_000;
/// A subtitle of a person's own, put where episode 3's is.
const EDITED: &str =
    "[Script Info]\n[Events]\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,바뀜\n";

struct Setup {
    dir: tempfile::TempDir,
    db: Db,
    store: Handles,
    runner: Runner,
}

impl Setup {
    fn work(&self) -> PathBuf {
        self.dir.path().join("shows/Show")
    }

    fn read(&self, path: &str) -> Vec<u8> {
        std::fs::read(self.work().join(path)).unwrap()
    }

    /// Whether the file is exactly `bytes`.
    fn is(&self, path: &str, bytes: &[u8]) -> bool {
        self.read(path) == bytes
    }

    async fn sql(&self, sql: String) {
        self.db
            .run(move |c| c.execute_batch(&sql).map_err(trss_core::DbError::from))
            .await
            .unwrap();
    }

    async fn count(&self, sql: String) -> i64 {
        self.db
            .run(move |c| {
                c.query_row(&sql, [], |r| r.get(0))
                    .map_err(trss_core::DbError::from)
            })
            .await
            .unwrap()
    }
}

/// The video of the episode, beside which its subtitle goes.
fn video(episode: u32) -> String {
    match episode == APART {
        true => format!("Season 01/Apart/Show S01E{episode:02}.mkv"),
        false => format!("Season 01/Show S01E{episode:02}.mkv"),
    }
}

/// The subtitle the app puts beside the episode's video.
fn target(episode: u32) -> String {
    format!("{}.ass", video(episode).trim_end_matches(".mkv"))
}

/// A library with the work `Show` (season 1) whose episodes 1 to 12 have a
/// video.
async fn setup() -> Setup {
    let base = Base::new().await;
    base.library(&Shows {
        episodes: EPISODES.collect(),
        source: true,
        apart: Some(APART),
        ..Shows::default()
    })
    .await;
    let runner = base.runner(Sources::none().with_fake(FakeSource));
    Setup {
        dir: base.dir,
        db: base.db,
        store: base.store,
        runner,
    }
}

/// A pick's job of the work for the fake posts `/ok/Show-NN<suffix>` of the
/// episodes, by [`CREATOR`].
async fn make(s: &Setup, command: &str, episodes: &[u32], suffix: &str) -> String {
    let job = NewJob {
        command_id: command.to_owned(),
        request: format!("{{\"c\":\"{command}\"}}"),
        origin: "pick".to_owned(),
        work_id: Some(WORK.to_owned()),
        season: Some(1),
        anime_no: Some(7),
        source_id: Some("src".to_owned()),
        creator: Some(CREATOR.to_owned()),
        revision_of: None,
        revises_attributed: false,
        items: episodes
            .iter()
            .map(|e| NewItem {
                observation_id: None,
                episode: e.to_string(),
                post_url: format!("https://{}/ok/Show-{e:02}{suffix}", fake::HOST),
                found_at: 500,
            })
            .collect(),
    };
    match s.store.requests.create(job, 900).await.unwrap() {
        Created::Created(id) => id,
        other => panic!("created: {other:?}"),
    }
}

async fn run(s: &Setup) {
    s.runner.run_ready(&CancellationToken::new()).await.unwrap();
}

async fn detail(s: &Setup, id: &str) -> JobDetail {
    s.store.views.detail(id).await.unwrap().unwrap()
}

fn waiting_for_approval(d: &JobDetail) {
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Approval)),
        "{:?} {:?}",
        d.row.note,
        d.events
    );
}

/// The job's latest plans, by episode.
async fn latest_plans(s: &Setup, job: &str) -> Vec<Plan> {
    let mut plans: Vec<Plan> = s
        .store
        .place
        .replacements(job)
        .await
        .unwrap()
        .into_iter()
        .map(|v| v.plan)
        .collect();
    plans.sort_by_key(|p| p.episode);
    plans
}

fn decisions(plans: &[Plan], replace: bool) -> Vec<Decision> {
    plans
        .iter()
        .map(|p| Decision {
            plan_id: p.id.clone(),
            version: p.version,
            replace,
        })
        .collect()
}

async fn decide_all(s: &Setup, job: &str, decisions: Vec<Decision>) -> Vec<Decided> {
    s.store
        .place
        .decide_replacements(job, decisions, DECIDED_AT)
        .await
        .unwrap()
}

/// The plan rows of the job by episode and version: state, reason and when
/// it was decided.
async fn rows(s: &Setup, job: &str) -> Vec<(i64, i64, PlanState, Option<String>, Option<i64>)> {
    let job = job.to_owned();
    s.db.run(move |c| {
        let mut stmt = c.prepare(
            "SELECT episode, version, state, reason, decided_at FROM subtitle_replacements
              WHERE job_id = ?1 ORDER BY episode, version",
        )?;
        let rows = stmt.query_map([job], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap()
}

/// Each placement row's outcome and note, in plan order.
async fn outcomes(s: &Setup, job: &str) -> Vec<(Option<Outcome>, Option<String>)> {
    let rows = s.store.place.plan(job).await.unwrap();
    rows.into_iter().map(|r| (r.outcome, r.note)).collect()
}

/// The first job's subtitles of all episodes (`/ok/Show-NN`) are applied,
/// then a second job's revisions (`/ok/Show-NNv2`) wait: the second job and
/// its twelve open plans.
async fn revisions_waiting(s: &Setup) -> (String, Vec<Plan>) {
    let all: Vec<u32> = EPISODES.collect();
    let first = make(s, "c1", &all, "").await;
    run(s).await;
    let d = detail(s, &first).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    for episode in EPISODES {
        assert!(s.is(&target(episode), &v1(episode)), "episode {episode}");
    }
    let second = make(s, "c2", &all, "v2").await;
    run(s).await;
    waiting_for_approval(&detail(s, &second).await);
    let plans = latest_plans(s, &second).await;
    (second, plans)
}

fn v1(episode: u32) -> Vec<u8> {
    fake::ass(&format!("Show-{episode:02}"))
}

fn v2(episode: u32) -> Vec<u8> {
    fake::ass(&format!("Show-{episode:02}v2"))
}

#[tokio::test]
async fn twelve_episodes_waiting_give_twelve_open_plans_in_one_job() {
    let s = setup().await;
    let (job, plans) = revisions_waiting(&s).await;
    assert_eq!(plans.len(), 12);
    assert_eq!(
        plans.iter().map(|p| p.episode).collect::<Vec<_>>(),
        (1..=12).collect::<Vec<_>>()
    );
    assert!(plans
        .iter()
        .all(|p| (p.version, p.state) == (1, PlanState::Open)));
    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.note.as_deref(),
        Some("교체를 기다리는 회차가 12개 있어요")
    );
    // Nothing beside the videos changed.
    for episode in EPISODES {
        assert!(s.is(&target(episode), &v1(episode)), "episode {episode}");
    }
}

#[tokio::test]
async fn approving_all_at_once_replaces_every_episode_with_one_requeue_and_a_record_each() {
    let s = setup().await;
    let (job, plans) = revisions_waiting(&s).await;
    let decided = decide_all(&s, &job, decisions(&plans, true)).await;
    assert_eq!(decided, vec![Decided::Done(PlanState::Approved); 12]);
    let d = detail(&s, &job).await;
    assert_eq!(
        (d.row.state, d.row.note.as_deref()),
        (JobState::Pending, Some(DECIDED))
    );
    // One log line for each episode.
    assert_eq!(
        s.count(format!(
            "SELECT count(*) FROM subtitle_job_events
              WHERE job_id = '{job}' AND message = '새 자막으로 교체하기로 했어요'"
        ))
        .await,
        12
    );
    for (episode, _, state, _, decided_at) in rows(&s, &job).await {
        assert_eq!(
            (state, decided_at),
            (PlanState::Approved, Some(DECIDED_AT)),
            "episode {episode}"
        );
    }
    run(&s).await;
    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    for episode in EPISODES {
        assert!(s.is(&target(episode), &v2(episode)), "episode {episode}");
    }
    for (episode, _, state, _, _) in rows(&s, &job).await {
        assert_eq!(state, PlanState::Done, "episode {episode}");
    }
}

#[tokio::test]
async fn a_new_revision_of_one_episode_leaves_it_to_compare_again_and_the_others_are_replaced() {
    let s = setup().await;
    let (job, plans) = revisions_waiting(&s).await;
    // The creator's third revision of episode 3 arrives, as a job of its
    // own, before the person chooses `모두 교체`.
    let third = make(&s, "c3", &[3], "v3").await;
    run(&s).await;
    waiting_for_approval(&detail(&s, &third).await);
    // The second job was not run again, so the plans it was shown are still
    // the ones to decide.
    let decided = decide_all(&s, &job, decisions(&plans, true)).await;
    assert_eq!(decided, vec![Decided::Done(PlanState::Approved); 12]);
    run(&s).await;

    // Episode 3 keeps what it had, and its plan is closed for the revision.
    assert!(s.is(&target(3), &v1(3)), "episode 3");
    let rows = rows(&s, &job).await;
    let three: Vec<_> = rows.iter().filter(|r| r.0 == 3).collect();
    assert_eq!(three[0].2, PlanState::Stale, "{three:?}");
    assert_eq!(three[0].3.as_deref(), Some(NEW_REVISION));
    // The other eleven are the new bytes, each with its own decision.
    for episode in EPISODES.filter(|e| *e != 3) {
        assert!(s.is(&target(episode), &v2(episode)), "episode {episode}");
        let row: Vec<_> = rows.iter().filter(|r| r.0 == i64::from(episode)).collect();
        assert_eq!(row.len(), 1, "{row:?}");
        assert_eq!(
            (row[0].2, row[0].4),
            (PlanState::Done, Some(DECIDED_AT)),
            "episode {episode}"
        );
    }
    // The second job is done: its row for episode 3 is stored only, as the
    // third job compares the newer revision.
    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    let outcomes: Vec<_> = s
        .store
        .place
        .plan(&job)
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.placed.map(|p| p.episode), r.outcome))
        .collect();
    assert!(outcomes.contains(&(Some(3), Some(Outcome::Existing))));
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| o.1 == Some(Outcome::Applied))
            .count(),
        11
    );
    // The third job still waits for the person to compare its own plan.
    waiting_for_approval(&detail(&s, &third).await);
}

#[tokio::test]
async fn an_episode_planned_again_before_the_batch_is_stale_and_the_others_are_approved() {
    let s = setup().await;
    let (job, plans) = revisions_waiting(&s).await;
    // Episode 3's subtitle beside the video changes, and the job runs again:
    // its plan is made again as version 2.
    std::fs::write(s.work().join(target(3)), EDITED).unwrap();
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'pending' WHERE id = '{job}'"
    ))
    .await;
    run(&s).await;
    waiting_for_approval(&detail(&s, &job).await);
    assert_eq!(
        s.count(format!(
            "SELECT count(*) FROM subtitle_replacements WHERE job_id = '{job}' AND version = 2"
        ))
        .await,
        1
    );

    // The batch carries the versions the person saw.
    let decided = decide_all(&s, &job, decisions(&plans, true)).await;
    for (plan, decided) in plans.iter().zip(&decided) {
        match plan.episode {
            3 => assert_eq!(decided, &Decided::Stale),
            _ => assert_eq!(decided, &Decided::Done(PlanState::Approved)),
        }
    }
    run(&s).await;
    for episode in EPISODES.filter(|e| *e != 3) {
        assert!(s.is(&target(episode), &v2(episode)), "episode {episode}");
    }
    // Episode 3 is untouched and waits for its new plan.
    assert!(s.is(&target(3), EDITED.as_bytes()));
    waiting_for_approval(&detail(&s, &job).await);
    let again = latest_plans(&s, &job).await;
    let open: Vec<_> = again
        .iter()
        .filter(|p| p.state == PlanState::Open)
        .collect();
    assert_eq!(open.len(), 1);
    assert_eq!((open[0].episode, open[0].version), (3, 2));
}

#[tokio::test]
async fn keeping_one_episode_and_approving_the_rest_in_a_batch() {
    let s = setup().await;
    let (job, plans) = revisions_waiting(&s).await;
    let two = plans.iter().find(|p| p.episode == 2).unwrap();
    // Episode 2 alone, as before.
    let single = s
        .store
        .place
        .decide_replacement(&job, &two.id, two.version, false, DECIDED_AT)
        .await
        .unwrap();
    assert_eq!(single, Decided::Done(PlanState::Kept));
    // The job went back in line for the one decision; the rest, in a batch,
    // are written all the same.
    assert_eq!(detail(&s, &job).await.row.state, JobState::Pending);
    let rest: Vec<Plan> = plans.iter().filter(|p| p.episode != 2).cloned().collect();
    let decided = decide_all(&s, &job, decisions(&rest, true)).await;
    assert_eq!(decided, vec![Decided::Done(PlanState::Approved); 11]);
    run(&s).await;
    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    assert!(s.is(&target(2), &v1(2)), "episode 2");
    for episode in EPISODES.filter(|e| *e != 2) {
        assert!(s.is(&target(episode), &v2(episode)), "episode {episode}");
    }
    let row = s
        .store
        .place
        .plan(&job)
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.placed.as_ref().is_some_and(|p| p.episode == 2))
        .unwrap();
    assert_eq!(row.outcome, Some(Outcome::Existing));
}

#[tokio::test]
async fn a_mixed_batch_settles_the_kept_rows_at_once_and_the_run_replaces_the_rest() {
    let s = setup().await;
    let (job, plans) = revisions_waiting(&s).await;
    let kept = [2, 5];
    let batch: Vec<Decision> = plans
        .iter()
        .map(|p| Decision {
            plan_id: p.id.clone(),
            version: p.version,
            replace: !kept.contains(&p.episode),
        })
        .collect();
    let decided = decide_all(&s, &job, batch).await;
    let expected: Vec<_> = plans
        .iter()
        .map(|p| match kept.contains(&p.episode) {
            true => Decided::Done(PlanState::Kept),
            false => Decided::Done(PlanState::Approved),
        })
        .collect();
    assert_eq!(decided, expected);
    // Before any run, the kept episodes' rows are stored only and the
    // approved ones are left to the run.
    for row in s.store.place.plan(&job).await.unwrap() {
        let episode = row.placed.as_ref().map(|p| p.episode).unwrap();
        match kept.contains(&episode) {
            true => assert_eq!(
                (row.outcome, row.note.as_deref()),
                (
                    Some(Outcome::Existing),
                    Some("현재 자막을 그대로 두고 보관만 했어요")
                ),
                "episode {episode}"
            ),
            false => assert_ne!(row.outcome, Some(Outcome::Existing), "episode {episode}"),
        }
    }
    run(&s).await;
    for episode in EPISODES {
        let expected = match kept.contains(&i64::from(episode)) {
            true => v1(episode),
            false => v2(episode),
        };
        assert!(s.is(&target(episode), &expected), "episode {episode}");
    }
}

#[tokio::test]
async fn keeping_every_episode_in_a_batch_leaves_the_files_and_settles_the_rows() {
    let s = setup().await;
    let (job, plans) = revisions_waiting(&s).await;
    let decided = decide_all(&s, &job, decisions(&plans, false)).await;
    assert_eq!(decided, vec![Decided::Done(PlanState::Kept); 12]);
    run(&s).await;
    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    for episode in EPISODES {
        assert!(s.is(&target(episode), &v1(episode)), "episode {episode}");
    }
    let outcomes: Vec<_> = s
        .store
        .place
        .plan(&job)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.outcome)
        .collect();
    assert_eq!(outcomes, vec![Some(Outcome::Existing); 12]);
}

#[tokio::test]
async fn one_episode_that_cannot_be_carried_out_fails_alone_and_the_job_is_partial() {
    let s = setup().await;
    let (job, plans) = revisions_waiting(&s).await;
    let decided = decide_all(&s, &job, decisions(&plans, true)).await;
    assert_eq!(decided, vec![Decided::Done(PlanState::Approved); 12]);
    // The folder of episode 12's video no longer takes a file out: its
    // subtitle can be read, but not moved aside.
    let apart = s.work().join("Season 01/Apart");
    std::fs::set_permissions(&apart, std::fs::Permissions::from_mode(0o555)).unwrap();
    run(&s).await;
    std::fs::set_permissions(&apart, std::fs::Permissions::from_mode(0o755)).unwrap();

    let d = detail(&s, &job).await;
    assert_eq!(
        d.row.state,
        JobState::Partial,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
    let failing = i64::from(APART);
    for (episode, _, state, reason, _) in rows(&s, &job).await {
        match episode == failing {
            true => {
                assert_eq!(state, PlanState::Failed);
                assert!(reason.is_some_and(|r| r.contains("기존 자막을 옮기지 못해")));
            }
            false => assert_eq!(
                (state, reason),
                (PlanState::Done, None),
                "episode {episode}"
            ),
        }
    }
    let plan_rows = s.store.place.plan(&job).await.unwrap();
    for row in plan_rows {
        let episode = row.placed.as_ref().map(|p| p.episode).unwrap();
        match episode == failing {
            true => {
                assert_eq!(row.outcome, Some(Outcome::Failed));
                assert!(row
                    .note
                    .as_deref()
                    .is_some_and(|n| n.contains("옮기지 못해")));
            }
            false => assert_ne!(row.outcome, Some(Outcome::Failed), "episode {episode}"),
        }
    }
    assert!(s.is(&target(APART), &v1(APART)), "episode {APART}");
    for episode in EPISODES.filter(|e| *e != APART) {
        assert!(s.is(&target(episode), &v2(episode)), "episode {episode}");
    }
}

#[tokio::test]
async fn a_batch_with_a_plan_of_another_job_writes_nothing() {
    let s = setup().await;
    let (job, plans) = revisions_waiting(&s).await;
    let third = make(&s, "c3", &[3], "v3").await;
    run(&s).await;
    waiting_for_approval(&detail(&s, &third).await);
    let foreign = latest_plans(&s, &third).await.remove(0);

    let before_rows = rows(&s, &job).await;
    let before_outcomes = outcomes(&s, &job).await;
    let before = detail(&s, &job).await;
    // Keeping some of them too, whose rows a decision would settle at once.
    let mut batch: Vec<Decision> = decisions(&plans, true)
        .into_iter()
        .enumerate()
        .map(|(i, d)| Decision {
            replace: i % 2 == 0,
            ..d
        })
        .collect();
    batch.insert(5, decisions(std::slice::from_ref(&foreign), true).remove(0));
    let decided = decide_all(&s, &job, batch).await;
    assert_eq!(decided.len(), 13);
    assert_eq!(decided[5], Decided::NotFound);
    assert!(decided
        .iter()
        .enumerate()
        .all(|(i, d)| i == 5 || d == &Decided::Stale));

    // Nothing of the twelve was decided, the job still waits and its log is
    // as it was; the other job's plan is open too.
    assert_eq!(rows(&s, &job).await, before_rows);
    assert_eq!(outcomes(&s, &job).await, before_outcomes);
    let after = detail(&s, &job).await;
    waiting_for_approval(&after);
    assert_eq!(after.events.len(), before.events.len());
    assert_eq!(after.row.note, before.row.note);
    assert_eq!(latest_plans(&s, &third).await[0].state, PlanState::Open);
    assert_eq!(
        s.count(format!(
            "SELECT count(*) FROM subtitle_replacements
              WHERE state <> 'open' AND job_id IN ('{job}', '{third}')"
        ))
        .await,
        0
    );
    // A plan nobody has is not found either.
    let nobody = vec![Decision {
        plan_id: "nope".to_owned(),
        version: 1,
        replace: true,
    }];
    assert_eq!(decide_all(&s, &job, nobody).await, vec![Decided::NotFound]);
}

#[tokio::test]
async fn a_batch_of_stale_decisions_writes_nothing_and_does_not_put_the_job_back_in_line() {
    let s = setup().await;
    let (job, plans) = revisions_waiting(&s).await;
    let before = detail(&s, &job).await;
    let stale: Vec<Decision> = plans
        .iter()
        .map(|p| Decision {
            plan_id: p.id.clone(),
            version: p.version + 1,
            replace: true,
        })
        .collect();
    assert_eq!(decide_all(&s, &job, stale).await, vec![Decided::Stale; 12]);
    let after = detail(&s, &job).await;
    waiting_for_approval(&after);
    assert_eq!(after.row.note, before.row.note);
    assert_eq!(after.events.len(), before.events.len());
    assert!(rows(&s, &job)
        .await
        .iter()
        .all(|r| (r.2, r.4) == (PlanState::Open, None)));

    // Decided plans are stale for a second batch, one at a time or not.
    assert_eq!(
        decide_all(&s, &job, decisions(&plans, true)).await,
        vec![Decided::Done(PlanState::Approved); 12]
    );
    assert_eq!(
        decide_all(&s, &job, decisions(&plans, false)).await,
        vec![Decided::Stale; 12]
    );
}
