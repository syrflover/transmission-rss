//! The to-dos that need a person (`trss_jobs::todo::list`, `docs/specs/jobs.md`
//! 할 일), gathered from the records the jobs and the history leave: a site's
//! check, a failed receipt, and the order and the count they come in.
//! (`todo/tests.rs` of the crate has the sums and the order of made-up
//! cards; the cards of replacements and of a table to confirm are in
//! `replace.rs`, `placement.rs` and `relocate.rs`, where their fixtures are.)

use crate::world::{Base, Shows};

use tokio_util::sync::CancellationToken;
use trss_collect::store::history::{HistoryResult, HistoryStore, Observation};
use trss_jobs::{todo::Todo, Created, ItemState, JobState, NewItem, NewJob, Runner, Wait};
use trss_subtitles::{
    fake::{self, FakeSource},
    Sources,
};

struct Setup {
    base: Base,
    runner: Runner,
}

/// A library with the work `Show` (season 1) linked to Anissia's anime 3424,
/// titled `작품`.
async fn setup() -> Setup {
    let base = Base::new().await;
    base.library(&Shows::default()).await;
    base.db
        .run(|c| {
            c.execute_batch(
                "INSERT INTO anissia_anime (anime_no, subject, week, status, fetched_at)
                     VALUES (3424, '작품', 2, 'ON', 77);",
            )
            .map_err(trss_core::DbError::from)
        })
        .await
        .unwrap();
    let runner = base.runner(Sources::none().with_fake(FakeSource));
    Setup { base, runner }
}

impl Setup {
    /// A job of the work for the fake posts (episode, path), made.
    async fn make(&self, command: &str, posts: &[(&str, &str)]) -> String {
        let job = NewJob {
            command_id: command.to_owned(),
            request: format!("{{\"c\":\"{command}\"}}"),
            origin: "pick".to_owned(),
            work_id: Some("w1".to_owned()),
            season: Some(1),
            anime_no: Some(3424),
            source_id: None,
            creator: Some("제작자".to_owned()),
            revision_of: None,
            revises_attributed: false,
            items: posts
                .iter()
                .map(|(episode, path)| NewItem {
                    observation_id: None,
                    episode: (*episode).to_owned(),
                    post_url: format!("https://{}{path}", fake::HOST),
                    found_at: 500,
                })
                .collect(),
        };
        match self.base.store.requests.create(job, 900).await.unwrap() {
            Created::Created(id) => id,
            other => panic!("created: {other:?}"),
        }
    }

    async fn run(&self) {
        self.runner
            .run_ready(&CancellationToken::new())
            .await
            .unwrap();
    }

    async fn state(&self, job: &str) -> (JobState, Option<Wait>) {
        let d = self.base.store.views.detail(job).await.unwrap().unwrap();
        (d.row.state, d.row.wait)
    }
}

/// The kinds of the cards, in order.
fn kinds(list: &trss_jobs::todo::TodoList) -> Vec<&'static str> {
    list.needs
        .iter()
        .map(|t| match t {
            Todo::Auth { .. } => "auth",
            Todo::ReceiveFailed { .. } => "receive_failed",
            Todo::EpisodeCheck { .. } => "episode_check",
            Todo::PlacementCheck { .. } => "placement_check",
            Todo::VideoCheck { .. } => "video_check",
            Todo::Replacement { .. } => "replacement",
        })
        .collect()
}

#[tokio::test]
async fn a_site_check_comes_before_a_receive_failure_and_the_count_has_both() {
    let s = setup().await;
    let job = s.make("c1", &[("1", "/auth/1"), ("2", "/ok/2")]).await;
    s.run().await;
    assert_eq!(s.state(&job).await, (JobState::Waiting, Some(Wait::Auth)));
    // A newer receive failure still comes after the check.
    HistoryStore::new(s.base.db.clone())
        .record(
            500_000,
            vec![Observation {
                channel_id: "c1".into(),
                channel_label: "https://feed.test/".into(),
                identity_key: "k".into(),
                title: "[Group] Show - 03".into(),
                link: "https://feed.test/x".into(),
                result: HistoryResult::AddFailed,
                rule_id: Some("r1".into()),
                torrent_hash: None,
                reason: Some("Transmission에 연결하지 못했어요".into()),
            }],
        )
        .await
        .unwrap();

    let list = s.base.todo().await;

    assert_eq!(kinds(&list), ["auth", "receive_failed"]);
    assert_eq!(list.count, 2);
    let Todo::Auth {
        key,
        work,
        title,
        season,
        episodes,
        creator,
        reason,
        job_id,
        jobs,
        ..
    } = &list.needs[0]
    else {
        panic!()
    };
    assert_eq!(key, "auth:w1");
    assert_eq!(work.as_ref().map(|w| w.id.as_str()), Some("w1"));
    assert_eq!(
        (title.as_str(), *season, creator.as_deref()),
        ("작품", Some(1), Some("제작자"))
    );
    assert_eq!(episodes, &["1"]);
    assert_eq!(reason, "CAPTCHA");
    assert_eq!((job_id.as_str(), *jobs), (job.as_str(), 1));
    let Todo::ReceiveFailed {
        context,
        channel_id,
        count,
        reason,
        ..
    } = &list.needs[1]
    else {
        panic!()
    };
    assert_eq!(*context, "add_failed");
    assert_eq!(channel_id.as_deref(), Some("c1"));
    assert_eq!(*count, 1);
    assert_eq!(reason.as_deref(), Some("Transmission에 연결하지 못했어요"));
    // The work shows the check as a badge; the failure has no work.
    assert_eq!(list.badges.get("w1"), Some(&vec!["auth"]));
    assert_eq!(list.badges.len(), 1);
}

#[tokio::test]
async fn the_checks_of_a_work_are_one_card_at_its_oldest_job_and_no_other_job_is_one() {
    let s = setup().await;
    let failed = s.make("c1", &[("1", "/missing/1")]).await;
    let partial = s.make("c2", &[("1", "/ok/1"), ("2", "/missing/2")]).await;
    let held = s.make("c3", &[("1", "/ok/3")]).await;
    let first = s.make("c4", &[("1", "/auth/1")]).await;
    let second = s.make("c5", &[("2", "/auth/2")]).await;
    // One is held, and one made after the run stays pending.
    s.base
        .db
        .run(move |c| {
            c.execute(
                "UPDATE subtitle_jobs SET state = 'held' WHERE id = ?1",
                [held],
            )
            .map_err(trss_core::DbError::from)
        })
        .await
        .unwrap();
    s.run().await;
    let pending = s.make("c6", &[("1", "/ok/6")]).await;
    assert_eq!(s.state(&pending).await.0, JobState::Pending);
    assert_eq!(s.state(&failed).await.0, JobState::Failed);
    assert_eq!(s.state(&partial).await.0, JobState::Partial);
    for job in [&first, &second] {
        assert_eq!(s.state(job).await, (JobState::Waiting, Some(Wait::Auth)));
    }
    let items = s.base.store.views.items(&second).await.unwrap();
    assert_eq!(items[0].state, ItemState::Waiting);

    let list = s.base.todo().await;

    // Failed and held jobs are no to-do; the checks a person has to pass are
    // one per work, at the oldest of its jobs.
    assert_eq!(kinds(&list), ["auth"]);
    assert_eq!(list.count, 1);
    let Todo::Auth {
        job_id,
        jobs,
        reason,
        episodes,
        ..
    } = &list.needs[0]
    else {
        panic!()
    };
    assert_eq!((job_id.as_str(), *jobs), (first.as_str(), 2));
    assert_eq!(reason, "CAPTCHA");
    assert_eq!(episodes, &["1", "2"]);
}
