//! The to-dos that need a person (`trss_jobs::todo::list`, `docs/specs/jobs.md`
//! 할 일), gathered from the records the jobs and the history leave: a site's
//! check, a failed receipt, and the order and the count they come in.
//! (`todo/tests.rs` of the crate has the sums and the order of made-up
//! cards; the cards of a table to confirm are in `placement.rs` and
//! `relocate.rs`, where their fixtures are.) The card of replacements is here
//! too, over the fake source's subtitles.

use std::{future::Future, pin::Pin};

use crate::world::{Base, Shows};

use tokio_util::sync::CancellationToken;
use trss_collect::store::history::{HistoryResult, HistoryStore, Observation};
use trss_jobs::{
    model::PlanState,
    place::replace::records::{Decided, Plan},
    todo::{Changes, Todo},
    Created, ItemState, JobState, NewItem, NewJob, Runner, Wait,
};
use trss_subtitles::{
    fake::{self, FakeSource},
    Sources,
};

struct Setup {
    base: Base,
    runner: Runner,
}

/// A library with the work `Show` (season 1, videos of episodes 2 and 3) linked
/// to Anissia's anime 3424, titled `작품`.
async fn setup() -> Setup {
    let base = Base::new().await;
    base.library(&Shows {
        episodes: vec![2, 3],
        ..Shows::default()
    })
    .await;
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

// --- the replacement cards ----------------------------------------------------------

type Prepare = for<'a> fn(&'a Setup) -> Pin<Box<dyn Future<Output = ()> + 'a>>;

/// A time a card shows: none, one the run made (any), or exactly `At`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Time {
    /// Not checked.
    Any,
    None,
    Some,
    At(i64),
}

impl Time {
    fn holds(self, shown: Option<i64>) -> bool {
        match self {
            Time::Any => true,
            Time::None => shown.is_none(),
            Time::Some => shown.is_some(),
            Time::At(at) => shown == Some(at),
        }
    }
}

/// What the work's one `교체 승인` card says, as far as a case checks it.
struct Card {
    episodes: &'static [i64],
    jobs: usize,
    /// What its open plans change, summed; `None`: not checked.
    changes: Option<Changes>,
    /// When the newest current subtitle the app manages, a current file it
    /// did not manage, and the newest new subtitle were received or changed.
    times: [Time; 3],
}

struct Case {
    name: &'static str,
    prepare: Prepare,
    card: Card,
}

const TARGET: &str = "Season 01/Show S01E02.ass";

impl Setup {
    fn at(&self, path: &str) -> std::path::PathBuf {
        self.base.dir.path().join("shows/Show").join(path)
    }

    /// A job of the fake posts (episode, path), made and run.
    async fn job(&self, command: &str, posts: &[(&str, &str)]) -> String {
        let id = self.make(command, posts).await;
        self.run().await;
        id
    }

    async fn sql(&self, sql: String) {
        self.base
            .db
            .run(move |c| c.execute_batch(&sql).map_err(trss_core::DbError::from))
            .await
            .unwrap();
    }

    /// The job's one plan.
    async fn plan(&self, job: &str) -> Plan {
        let mut views = self.base.store.place.replacements(job).await.unwrap();
        assert_eq!(views.len(), 1, "{views:?}");
        views.remove(0).plan
    }

    /// The plan's comparison as an earlier build left it (none) or as the
    /// worker could not make it (`why`).
    async fn recompared(&self, plan: &Plan, why: Option<&str>) {
        let insert = match why {
            Some(why) => format!(
                "INSERT INTO subtitle_replacement_diffs (plan_id, path, unreadable)
                 VALUES ('{}', '{TARGET}', '{why}');",
                plan.id
            ),
            None => String::new(),
        };
        self.sql(format!(
            "DELETE FROM subtitle_replacement_diffs WHERE plan_id = '{}'; {insert}",
            plan.id
        ))
        .await;
    }

    /// The first job applies the episode 2's subtitle (`/ok/Show-02`) and a
    /// second job's plan to replace it with its revision waits: the second
    /// job and its plan.
    async fn revision_waiting(&self) -> (String, Plan) {
        self.job("c1", &[("2", "/ok/Show-02")]).await;
        let job = self.job("c2", &[("2", "/ok/Show-02v2")]).await;
        let plan = self.plan(&job).await;
        (job, plan)
    }
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            // Two plans compared (24 dialogue lines changed each), one that
            // could not be, and one made before contents were compared.
            name: "four open plans of one episode are summed",
            prepare: |s| {
                Box::pin(async move {
                    s.job("c1", &[("2", "/ok/Show-02")]).await;
                    s.job("c2", &[("2", "/ok/Show-02v2")]).await;
                    s.job("c3", &[("2", "/ok/Show-02v3")]).await;
                    let unreadable = s.job("c4", &[("2", "/ok/Show-02v4")]).await;
                    let earlier = s.job("c5", &[("2", "/ok/Show-02v5")]).await;
                    let (unreadable, earlier) = (s.plan(&unreadable).await, s.plan(&earlier).await);
                    s.recompared(&unreadable, Some("현재 자막: 인코딩을 알 수 없어요"))
                        .await;
                    s.recompared(&earlier, None).await;
                })
            },
            card: Card {
                episodes: &[2],
                jobs: 4,
                changes: Some(Changes {
                    changed: 48,
                    uncompared: 2,
                    plans: 4,
                    ..Changes::default()
                }),
                times: [Time::Any; 3],
            },
        },
        Case {
            // A file the app did not manage: the new copy's font and one
            // line's start differ.
            name: "timing, styles and fonts are summed too",
            prepare: |s| {
                Box::pin(async move {
                    let current = String::from_utf8(fake::ass("Show-02v2"))
                        .unwrap()
                        .replace("Style: Default,Arial,", "Style: Default,Noto Sans CJK KR,")
                        .replacen("Dialogue: 0,0:00:00.00,", "Dialogue: 0,0:00:00.50,", 1);
                    std::fs::write(s.at(TARGET), current).unwrap();
                    s.job("c1", &[("2", "/ok/Show-02v2")]).await;
                })
            },
            card: Card {
                episodes: &[2],
                jobs: 1,
                changes: Some(Changes {
                    timing: 1,
                    styles: 1,
                    fonts: 2,
                    plans: 1,
                    ..Changes::default()
                }),
                times: [Time::Any; 3],
            },
        },
        Case {
            // An SRT beside the video with the new ASS's very dialogue:
            // nothing differs in what was compared, but the ASS's styles and
            // fonts were not.
            name: "a plan compared only in part is counted",
            prepare: |s| {
                Box::pin(async move {
                    let mut srt = String::new();
                    for i in 0..24 {
                        srt += &format!(
                            "{}\n00:00:{:02},000 --> 00:00:{:02},500\n가짜 자막 Show-02v2 {}\n\n",
                            i + 1,
                            i * 2,
                            i * 2 + 1,
                            i + 1
                        );
                    }
                    std::fs::write(s.at("Season 01/Show S01E02.srt"), srt).unwrap();
                    s.job("c1", &[("2", "/ok/Show-02v2")]).await;
                })
            },
            card: Card {
                episodes: &[2],
                jobs: 1,
                changes: Some(Changes {
                    partial: 1,
                    plans: 1,
                    ..Changes::default()
                }),
                times: [Time::Any; 3],
            },
        },
        Case {
            // Each plan's current and new subtitles were received at other
            // times.
            name: "the newest current and new subtitles' receipt times are shown",
            prepare: |s| {
                Box::pin(async move {
                    let first = s
                        .job("c1", &[("2", "/ok/Show-02"), ("3", "/ok/Show-03")])
                        .await;
                    let second = s
                        .job("c2", &[("2", "/ok/Show-02v2"), ("3", "/ok/Show-03v2")])
                        .await;
                    let received = |job: &str, episode: i64, at: i64| {
                        format!(
                            "UPDATE subtitle_packages SET received_at = {at}
                              WHERE id IN (SELECT package_id FROM subtitle_stored
                                            WHERE job_id = '{job}' AND episode = {episode});"
                        )
                    };
                    s.sql(
                        [
                            received(&first, 2, 1_000),
                            received(&first, 3, 3_000),
                            received(&second, 2, 6_000),
                            received(&second, 3, 5_000),
                        ]
                        .concat(),
                    )
                    .await;
                })
            },
            card: Card {
                episodes: &[2, 3],
                jobs: 1,
                changes: None,
                times: [Time::At(3_000), Time::None, Time::At(6_000)],
            },
        },
        Case {
            name: "a current file the app did not manage gives its change time",
            prepare: |s| {
                Box::pin(async move {
                    std::fs::write(s.at(TARGET), fake::ass("Show-02")).unwrap();
                    let changed =
                        std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_700_000_000_000);
                    std::fs::File::options()
                        .write(true)
                        .open(s.at(TARGET))
                        .unwrap()
                        .set_modified(changed)
                        .unwrap();
                    s.job("c1", &[("2", "/ok/Show-02v2")]).await;
                })
            },
            card: Card {
                episodes: &[2],
                jobs: 1,
                changes: None,
                times: [Time::None, Time::At(1_700_000_000_000), Time::Some],
            },
        },
        Case {
            // Episode 3's current file is not managed, but episode 2's is:
            // the receipt time is shown, and no change time.
            name:
                "a current subtitle the app manages gives its receipt time over another plan's file",
            prepare: |s| {
                Box::pin(async move {
                    s.job("c1", &[("2", "/ok/Show-02")]).await;
                    std::fs::write(s.at("Season 01/Show S01E03.ass"), fake::ass("Show-03"))
                        .unwrap();
                    s.job("c2", &[("2", "/ok/Show-02v2"), ("3", "/ok/Show-03v2")])
                        .await;
                })
            },
            card: Card {
                episodes: &[2, 3],
                jobs: 1,
                changes: None,
                times: [Time::Some, Time::None, Time::Any],
            },
        },
    ]
}

#[tokio::test]
async fn the_replacement_card_sums_what_its_open_plans_change_and_says_when_their_subtitles_came() {
    for case in cases() {
        let s = setup().await;
        (case.prepare)(&s).await;

        let list = s.base.todo().await;

        let [Todo::Replacement {
            episodes,
            jobs,
            changes,
            current_received_at,
            current_changed_at,
            new_received_at,
            ..
        }] = &list.needs[..]
        else {
            panic!("{}: {:?}", case.name, list.needs)
        };
        assert_eq!(episodes, case.card.episodes, "{}", case.name);
        assert_eq!(*jobs, case.card.jobs, "{}", case.name);
        if let Some(expected) = &case.card.changes {
            assert_eq!(changes, expected, "{}", case.name);
        }
        let shown = [*current_received_at, *current_changed_at, *new_received_at];
        for (time, shown) in case.card.times.into_iter().zip(shown) {
            assert!(time.holds(shown), "{}: {time:?} {shown:?}", case.name);
        }
    }
}

#[tokio::test]
async fn a_revision_waiting_for_approval_is_one_card_of_its_work_opening_its_job() {
    let s = setup().await;
    let (job, _) = s.revision_waiting().await;

    let list = s.base.todo().await;

    assert_eq!(list.count, 1);
    let Todo::Replacement {
        key,
        work,
        title,
        season,
        episodes,
        creator,
        job_id,
        jobs,
        changes,
        ..
    } = &list.needs[0]
    else {
        panic!("{:?}", list.needs)
    };
    assert_eq!(key, "replacement:w1");
    assert_eq!(work.as_ref().map(|w| w.id.as_str()), Some("w1"));
    assert_eq!(
        (title.as_str(), *season, creator.as_deref()),
        ("작품", Some(1), Some("제작자"))
    );
    assert_eq!(
        (episodes, job_id.as_str(), *jobs),
        (&vec![2], job.as_str(), 1)
    );
    assert_eq!(
        changes,
        &Changes {
            changed: 24,
            plans: 1,
            ..Changes::default()
        }
    );
    assert_eq!(list.badges.get("w1"), Some(&vec!["replacement"]));
}

#[tokio::test]
async fn a_decided_replacement_is_no_card_and_no_badge() {
    for replace in [true, false] {
        let s = setup().await;
        let (job, plan) = s.revision_waiting().await;
        assert_eq!(
            s.base.todo().await.badges.get("w1"),
            Some(&vec!["replacement"])
        );

        let decided = s
            .base
            .store
            .place
            .decide_replacement(&job, &plan.id, plan.version, replace, 5_000_000)
            .await
            .unwrap();

        let expected = match replace {
            true => PlanState::Approved,
            false => PlanState::Kept,
        };
        assert_eq!(decided, Decided::Done(expected), "{replace}");
        let list = s.base.todo().await;
        assert_eq!((list.count, list.badges.len()), (0, 0), "{replace}");
    }
}

#[tokio::test]
async fn the_receive_failures_have_the_adds_a_rule_failed_and_no_other_history_item() {
    let base = Base::new().await;
    let observation = |key: &str, result, rule_id: Option<&str>| Observation {
        channel_id: "c1".into(),
        channel_label: "https://feed.test/".into(),
        identity_key: key.into(),
        title: format!("[Group] Show - {key}"),
        link: format!("https://feed.test/{key}"),
        result,
        rule_id: rule_id.map(str::to_owned),
        torrent_hash: None,
        reason: Some("Transmission에 연결하지 못했어요".into()),
    };
    let history = HistoryStore::new(base.db.clone());
    history
        .record(
            500_000,
            vec![
                observation("03", HistoryResult::AddFailed, Some("r1")),
                // A failure of no rule (a hand-made add), and one that was added.
                observation("04", HistoryResult::AddFailed, None),
                observation("05", HistoryResult::Received, Some("r1")),
            ],
        )
        .await
        .unwrap();

    let failures = trss_jobs::todo::receive_failures(
        &trss_collect::store::revisions::RevisionStore::new(base.db.clone()),
        &history,
    )
    .await
    .unwrap();

    assert!(failures.revisions.is_empty());
    let added: Vec<(&str, &str)> = failures
        .adds
        .iter()
        .map(|a| (a.rule_id.as_str(), a.item.title.as_str()))
        .collect();
    assert_eq!(added, [("r1", "[Group] Show - 03")]);
}
