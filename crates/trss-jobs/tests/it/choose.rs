//! Choosing a stored subtitle to apply (`docs/specs/subtitles.md`, 보관본과
//! 적용본; `docs/tickets/0072-choose-stored-subtitle.md`,
//! `trss_jobs::place::records::choose_stored`): as the episode's subtitle, which
//! is a replacement to compare when the episode has one and a past revision
//! restored when a newer one is stored, or added beside the creator's applied
//! copies; and the work's own order of the formats.

use crate::{
    world::{Base, Shows},
    Handles,
};
use std::path::PathBuf;

use tokio_util::sync::CancellationToken;
use trss_core::{
    settings::{policy::FormatOrder, SettingsStore},
    Db,
};
use trss_jobs::{
    model::{Chosen, Outcome, PathAction, PlanState, StepKind, StepState, SubtitleFormat},
    place::{
        records::{PlanRow, StoredChoice, ADD_REFUSED},
        replace::{
            records::{Decided, Plan, PlanView},
            ADOPTED,
        },
    },
    store::JobDetail,
    Created, JobState, NewItem, NewJob, Runner, Wait,
};
use trss_subtitles::{
    fake::{self, FakeSource},
    Sources,
};

const WORK: &str = "w1";
const CREATOR: &str = "제작자";
const OTHER: &str = "다른 제작자";
const TARGET: &str = "Season 01/Show S01E02.ass";
const SRT: &str = "Season 01/Show S01E02.srt";
const SMI: &str = "Season 01/Show S01E02.smi";
/// A subtitle the app did not apply, at `SRT`.
const MINE: &str = "1\n00:00:01,000 --> 00:00:02,000\n내 자막\n";
/// A subtitle the app did not apply, at `SMI`.
const MINE_SMI: &str = "<SAMI><BODY><SYNC Start=1000><P>내 자막</BODY></SAMI>\n";

struct Setup {
    dir: tempfile::TempDir,
    db: Db,
    store: Handles,
    runner: Runner,
}

impl Setup {
    /// The work's folder.
    fn work(&self) -> PathBuf {
        self.dir.path().join("shows/Show")
    }

    fn at(&self, path: &str) -> PathBuf {
        self.work().join(path)
    }

    fn read(&self, path: &str) -> Vec<u8> {
        std::fs::read(self.at(path)).unwrap()
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

    /// The paths of the copies the app applied and has not removed, with the
    /// stored subtitle each is a copy of.
    async fn applied(&self) -> Vec<(String, String)> {
        self.db
            .run(|c| {
                let mut stmt = c.prepare(
                    "SELECT path, stored_id FROM subtitle_applied
                      WHERE removed_at IS NULL ORDER BY path",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(trss_core::DbError::from)
            })
            .await
            .unwrap()
    }

    /// The stored subtitle of the stored file `name`.
    async fn stored(&self, name: &str) -> String {
        let name = name.to_owned();
        self.db
            .run(move |c| {
                c.query_row(
                    "SELECT s.id FROM subtitle_stored s JOIN subtitle_assets a
                         ON a.id = s.subtitle_asset_id
                      WHERE a.relative_path LIKE '%/' || ?1",
                    [name],
                    |r| r.get::<_, String>(0),
                )
                .map_err(trss_core::DbError::from)
            })
            .await
            .unwrap()
    }

    /// Records the subtitle at `path` beside episode 2's video, as the
    /// library's watcher does.
    async fn record(&self, path: &str) {
        self.sql(format!(
            "INSERT INTO media_files (work_id, path, season, episode, kind)
                 VALUES ('{WORK}', '{path}', 1, '02', 'subtitle')"
        ))
        .await;
    }

    async fn choose(&self, stored: &str, mode: Chosen) -> StoredChoice {
        self.store
            .place
            .choose_stored(WORK, stored, mode, 5_000_000)
            .await
            .unwrap()
    }
}

/// A library with the work `Show` (season 1) whose episodes 2 and 3 have a
/// video, and the subtitle source `src`.
async fn setup() -> Setup {
    let base = Base::new().await;
    base.library(&Shows {
        episodes: vec![2, 3],
        source: true,
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

/// A pick's job of the work for the fake post `path` (on [`fake::HOST`]) by
/// `creator`, as Anissia's episode 2, of the source `src` when `source`.
async fn make(s: &Setup, command: &str, creator: &str, path: &str, source: bool) -> String {
    make_at(s, command, creator, path, source, "2").await
}

async fn make_at(
    s: &Setup,
    command: &str,
    creator: &str,
    path: &str,
    source: bool,
    episode: &str,
) -> String {
    let job = NewJob {
        command_id: command.to_owned(),
        request: format!("{{\"c\":\"{command}\"}}"),
        origin: "pick".to_owned(),
        work_id: Some(WORK.to_owned()),
        season: Some(1),
        anime_no: Some(7),
        source_id: source.then(|| "src".to_owned()),
        creator: Some(creator.to_owned()),
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

async fn run(s: &Setup) {
    s.runner.run_ready(&CancellationToken::new()).await.unwrap();
}

async fn detail(s: &Setup, id: &str) -> JobDetail {
    s.store.views.detail(id).await.unwrap().unwrap()
}

fn done(d: &JobDetail) {
    assert_eq!(
        d.row.state,
        JobState::Done,
        "{:?} {:?}",
        d.row.note,
        d.events
    );
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

/// The job's one latest plan.
async fn view(s: &Setup, job: &str) -> PlanView {
    let mut views = s.store.place.replacements(job).await.unwrap();
    assert_eq!(views.len(), 1, "{views:?}");
    views.remove(0)
}

async fn decide(s: &Setup, job: &str, plan: &Plan, replace: bool) {
    let decided = s
        .store
        .place
        .decide_replacement(job, &plan.id, plan.version, replace, 5_000_000)
        .await
        .unwrap();
    assert_eq!(
        decided,
        Decided::Done(match replace {
            true => PlanState::Approved,
            false => PlanState::Kept,
        })
    );
}

fn actions(plan: &Plan) -> Vec<(&str, PathAction)> {
    plan.paths
        .iter()
        .map(|p| (p.path.as_str(), p.action))
        .collect()
}

/// What a row of the job's plan was chosen as.
async fn chosen_of(s: &Setup, job: &str, stored: &str) -> Option<Chosen> {
    let rows = s.store.place.plan(job).await.unwrap();
    rows.iter()
        .find(|r| r.stored_id.as_deref() == Some(stored))
        .expect("the row")
        .chosen
}

/// The creator's `/ok/Show-02` applied beside the video by a first job, then
/// `other`'s `/ok/Show-02b` brought by a second job, which the person keeps
/// the current subtitle against: the second job, done, and its stored copy.
async fn other_kept(s: &Setup, other: &str) -> (String, String) {
    let first = make(s, "c1", CREATOR, "/ok/Show-02", false).await;
    run(s).await;
    done(&detail(s, &first).await);
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));
    let job = make(s, "c2", other, "/ok/Show-02b", false).await;
    run(s).await;
    waiting_for_approval(&detail(s, &job).await);
    let plan = view(s, &job).await.plan;
    decide(s, &job, &plan, false).await;
    run(s).await;
    done(&detail(s, &job).await);
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));
    (job, s.stored("Show-02b.ass").await)
}

/// A package with an ASS and an SRT of episode 2 by the creator: the ASS is
/// applied (the common order) and the SRT stored only: the job and the SRT's
/// stored subtitle.
async fn ass_applied_srt_stored(s: &Setup) -> (String, String) {
    let job = make(s, "c1", CREATOR, "/pack/Show - 02.ass/Show - 02.srt", false).await;
    run(s).await;
    done(&detail(s, &job).await);
    assert_eq!(s.read(TARGET), fake::bytes_of("Show - 02.ass"));
    assert!(!s.at(SRT).exists());
    (job, s.stored("Show - 02.srt").await)
}

#[tokio::test]
async fn another_creators_copy_on_an_episode_with_a_subtitle_is_compared_then_replaces_it() {
    let s = setup().await;
    let (job, other) = other_kept(&s, OTHER).await;
    let kept = s.stored("Show-02.ass").await;

    // The episode has a subtitle: choosing the copy asks for a comparison.
    assert_eq!(
        s.choose(&other, Chosen::Apply).await,
        StoredChoice::Queued {
            job_id: job.clone(),
            compare: true
        }
    );
    assert_eq!(chosen_of(&s, &job, &other).await, Some(Chosen::Apply));
    run(&s).await;
    waiting_for_approval(&detail(&s, &job).await);
    let v = view(&s, &job).await;
    assert_eq!(
        (v.plan.state, v.plan.stored_id.as_str()),
        (PlanState::Open, other.as_str())
    );
    assert_eq!(actions(&v.plan), [(TARGET, PathAction::Replace)]);
    assert!(v.comparison.is_some(), "the plan compares contents");
    // Nothing beside the video changed before the approval.
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));

    decide(&s, &job, &v.plan, true).await;
    run(&s).await;
    done(&detail(&s, &job).await);
    assert_eq!(s.read(TARGET), fake::ass("Show-02b"));
    // The replaced subtitle's stored copy is still there, no longer applied.
    assert_eq!(
        s.read(".trss/subtitles/제작자/Show-02.ass"),
        fake::ass("Show-02")
    );
    assert_eq!(s.applied().await, [(TARGET.to_owned(), other)]);
    assert_eq!(
        s.count(format!(
            "SELECT count(*) FROM subtitle_stored WHERE id = '{kept}' AND cleaned_at IS NULL"
        ))
        .await,
        1
    );
}

#[tokio::test]
async fn a_copy_on_an_episode_with_no_subtitle_is_applied_at_once_and_marked_chosen() {
    let s = setup().await;
    // Episode 3 is no one's pick: its subtitle is stored only.
    let job = make(
        &s,
        "c1",
        CREATOR,
        "/pack/Show - 02.ass/Show - 03.ass",
        false,
    )
    .await;
    run(&s).await;
    done(&detail(&s, &job).await);
    let three = s.stored("Show - 03.ass").await;
    assert_eq!(chosen_of(&s, &job, &three).await, None);
    assert!(!s.at("Season 01/Show S01E03.ass").exists());

    assert_eq!(
        s.choose(&three, Chosen::Apply).await,
        StoredChoice::Queued {
            job_id: job.clone(),
            compare: false
        }
    );
    assert_eq!(chosen_of(&s, &job, &three).await, Some(Chosen::Apply));
    run(&s).await;
    done(&detail(&s, &job).await);
    assert_eq!(
        s.read("Season 01/Show S01E03.ass"),
        fake::bytes_of("Show - 03.ass")
    );
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_replacements".into())
            .await,
        0
    );
}

#[tokio::test]
async fn a_past_revision_is_restored_over_the_newer_one_which_stays_stored() {
    let s = setup().await;
    // The creator's v1 applied, then v2 of the same source replaced it.
    let first = make(&s, "c1", CREATOR, "/ok/Show-02", true).await;
    run(&s).await;
    done(&detail(&s, &first).await);
    let second = make(&s, "c2", CREATOR, "/ok/Show-02v2", true).await;
    run(&s).await;
    waiting_for_approval(&detail(&s, &second).await);
    let plan = view(&s, &second).await.plan;
    decide(&s, &second, &plan, true).await;
    run(&s).await;
    done(&detail(&s, &second).await);
    assert_eq!(s.read(TARGET), fake::ass("Show-02v2"));
    let (v1, v2) = (
        s.stored("Show-02.ass").await,
        s.stored("Show-02v2.ass").await,
    );

    // Choosing v1 makes a plan despite the newer revision, which would have
    // kept the copy stored only, and the plan's check does not close it.
    assert_eq!(
        s.choose(&v1, Chosen::Apply).await,
        StoredChoice::Queued {
            job_id: first.clone(),
            compare: true
        }
    );
    run(&s).await;
    waiting_for_approval(&detail(&s, &first).await);
    let v = view(&s, &first).await;
    assert_eq!(
        (v.plan.state, v.plan.stored_id.as_str()),
        (PlanState::Open, v1.as_str())
    );
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'pending', wait = NULL WHERE id = '{first}'"
    ))
    .await;
    run(&s).await;
    waiting_for_approval(&detail(&s, &first).await);
    let again = view(&s, &first).await.plan;
    assert_eq!(
        (again.id.as_str(), again.state),
        (v.plan.id.as_str(), PlanState::Open)
    );

    decide(&s, &first, &again, true).await;
    run(&s).await;
    done(&detail(&s, &first).await);
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));
    assert_eq!(
        s.read(".trss/subtitles/제작자/Show-02v2.ass"),
        fake::ass("Show-02v2")
    );
    assert_eq!(s.applied().await, [(TARGET.to_owned(), v1)]);
    assert_eq!(
        s.count(format!(
            "SELECT count(*) FROM subtitle_stored WHERE id = '{v2}' AND cleaned_at IS NULL"
        ))
        .await,
        1
    );
}

#[tokio::test]
async fn a_newer_revision_still_keeps_a_row_nobody_chose_stored_only() {
    let s = setup().await;
    // v1 of the source is stored only (the person kept the current subtitle
    // of another creator), then v2 of the same source arrives: v1's job
    // compares nothing, the newer one is the job to compare.
    let mine = make(&s, "c0", OTHER, "/ok/Show-02", false).await;
    run(&s).await;
    done(&detail(&s, &mine).await);
    let first = make(&s, "c1", CREATOR, "/ok/Show-02b", true).await;
    run(&s).await;
    waiting_for_approval(&detail(&s, &first).await);
    let second = make(&s, "c2", CREATOR, "/ok/Show-02bv2", true).await;
    run(&s).await;
    waiting_for_approval(&detail(&s, &second).await);
    // The first job's plan closes for the newer revision.
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'pending', wait = NULL WHERE id = '{first}'"
    ))
    .await;
    run(&s).await;
    let rows = s.store.place.plan(&first).await.unwrap();
    assert!(rows.iter().all(|r| r.chosen.is_none()));
    assert_eq!(
        s.count(format!(
            "SELECT count(*) FROM subtitle_replacements
              WHERE job_id = '{first}' AND state = 'stale'"
        ))
        .await,
        1
    );
}

#[tokio::test]
async fn the_creators_other_format_is_added_beside_the_applied_copy_with_no_approval() {
    let s = setup().await;
    let (job, srt) = ass_applied_srt_stored(&s).await;
    let ass_before = s.read(TARGET);

    assert_eq!(
        s.choose(&srt, Chosen::Add).await,
        StoredChoice::Queued {
            job_id: job.clone(),
            compare: false
        }
    );
    assert_eq!(chosen_of(&s, &job, &srt).await, Some(Chosen::Add));
    run(&s).await;
    done(&detail(&s, &job).await);
    assert_eq!(s.read(SRT), fake::bytes_of("Show - 02.srt"));
    assert_eq!(s.read(TARGET), ass_before);
    let applied = s.applied().await;
    assert_eq!(applied.len(), 2, "{applied:?}");
    assert_eq!(applied[0].0, TARGET);
    assert_eq!(
        (applied[1].0.as_str(), applied[1].1.as_str()),
        (SRT, srt.as_str())
    );
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_replacements".into())
            .await,
        0
    );
}

#[tokio::test]
async fn an_added_format_whose_path_holds_a_file_is_planned_with_the_applied_copy_kept() {
    let s = setup().await;
    let (job, srt) = ass_applied_srt_stored(&s).await;
    let ass_before = s.read(TARGET);
    // A subtitle of the person's own is at the new copy's path, and the
    // library recorded it: choosing opens the comparison.
    std::fs::write(s.at(SRT), MINE).unwrap();
    s.record(SRT).await;

    assert_eq!(
        s.choose(&srt, Chosen::Add).await,
        StoredChoice::Queued {
            job_id: job.clone(),
            compare: true
        }
    );
    run(&s).await;
    waiting_for_approval(&detail(&s, &job).await);
    let v = view(&s, &job).await;
    assert_eq!(
        actions(&v.plan),
        [(SRT, PathAction::Replace), (TARGET, PathAction::Keep)]
    );
    // Nothing is taken off the applied copy, which the plan shows as the
    // episode's other subtitle.
    assert!(v.plan.taken_off().all(|p| p.path == SRT));
    assert_eq!(s.read(SRT), MINE.as_bytes());

    decide(&s, &job, &v.plan, true).await;
    run(&s).await;
    done(&detail(&s, &job).await);
    assert_eq!(s.read(SRT), fake::bytes_of("Show - 02.srt"));
    assert_eq!(s.read(TARGET), ass_before);
    assert!(s
        .applied()
        .await
        .iter()
        .any(|(p, id)| p == TARGET && *id != srt));
    assert!(s
        .applied()
        .await
        .iter()
        .any(|(p, id)| p == SRT && *id == srt));
}

#[tokio::test]
async fn an_added_format_counts_only_its_own_name_among_the_subtitles_the_library_recorded() {
    let s = setup().await;
    let (job, srt) = ass_applied_srt_stored(&s).await;
    let ass_before = s.read(TARGET);
    // The watcher recorded the applied ASS and a subtitle of the person's own
    // at another name.
    std::fs::write(s.at(SMI), MINE_SMI).unwrap();
    s.record(TARGET).await;
    s.record(SMI).await;

    assert_eq!(
        s.choose(&srt, Chosen::Add).await,
        StoredChoice::Queued {
            job_id: job.clone(),
            compare: false
        }
    );
    run(&s).await;
    done(&detail(&s, &job).await);
    assert_eq!(s.read(SRT), fake::bytes_of("Show - 02.srt"));
    assert_eq!(s.read(TARGET), ass_before);
    assert_eq!(s.read(SMI), MINE_SMI.as_bytes());
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_replacements".into())
            .await,
        0
    );
}

#[tokio::test]
async fn an_added_formats_name_is_held_in_any_case() {
    let s = setup().await;
    let (job, srt) = ass_applied_srt_stored(&s).await;
    // The person's subtitle has the new copy's name in other case.
    let upper = "Season 01/Show S01E02.SRT";
    std::fs::write(s.at(upper), MINE).unwrap();
    s.record(upper).await;

    assert_eq!(
        s.choose(&srt, Chosen::Add).await,
        StoredChoice::Queued {
            job_id: job.clone(),
            compare: true
        }
    );
    run(&s).await;
    waiting_for_approval(&detail(&s, &job).await);
    let v = view(&s, &job).await;
    assert_eq!(
        actions(&v.plan),
        [(upper, PathAction::Replace), (TARGET, PathAction::Keep)]
    );
    assert_eq!(s.read(upper), MINE.as_bytes());
}

#[tokio::test]
async fn an_add_is_for_the_creators_other_format_and_the_other_refusals_stand() {
    let s = setup().await;
    // Another creator's copy is not added: the episode has no applied copy
    // of that creator, whatever its format.
    let (job, other) = other_kept(&s, OTHER).await;
    let their_srt = make(&s, "c4", OTHER, "/pack/Show - 02.srt", false).await;
    run(&s).await;
    waiting_for_approval(&detail(&s, &their_srt).await);
    let plan = view(&s, &their_srt).await.plan;
    decide(&s, &their_srt, &plan, false).await;
    run(&s).await;
    done(&detail(&s, &their_srt).await);
    let srt = s.stored("Show - 02.srt").await;
    assert_eq!(
        s.choose(&srt, Chosen::Add).await,
        StoredChoice::Refused(ADD_REFUSED)
    );
    assert_eq!(
        s.choose(&other, Chosen::Add).await,
        StoredChoice::Refused(ADD_REFUSED)
    );
    // Nor is a copy of the format the episode has applied already, the
    // creator's second ASS.
    let second = make(&s, "c3", CREATOR, "/ok/Show-02v2", false).await;
    run(&s).await;
    waiting_for_approval(&detail(&s, &second).await);
    let plan = view(&s, &second).await.plan;
    decide(&s, &second, &plan, false).await;
    run(&s).await;
    done(&detail(&s, &second).await);
    let v2 = s.stored("Show-02v2.ass").await;
    assert_eq!(
        s.choose(&v2, Chosen::Add).await,
        StoredChoice::Refused(ADD_REFUSED)
    );
    // A refusal changes nothing: the row is not marked.
    assert_eq!(chosen_of(&s, &second, &v2).await, None);
    assert_eq!(detail(&s, &second).await.row.state, JobState::Done);

    // A copy applied already, an unknown one, one of a format the app does
    // not apply, and a job that cannot take it are refused as before, for
    // either mode.
    let applied = s.stored("Show-02.ass").await;
    for mode in [Chosen::Apply, Chosen::Add] {
        assert_eq!(
            s.choose(&applied, mode).await,
            StoredChoice::Refused("이 보관본은 이미 영상 옆에 적용했어요.")
        );
        assert_eq!(s.choose("nope", mode).await, StoredChoice::NotFound);
    }
    s.sql(format!(
        "UPDATE subtitle_stored SET format = 'other' WHERE id = '{other}'"
    ))
    .await;
    for mode in [Chosen::Apply, Chosen::Add] {
        assert_eq!(
            s.choose(&other, mode).await,
            StoredChoice::Refused("자동으로 적용하지 않는 형식이라 적용할 수 없어요.")
        );
    }
    s.sql(format!(
        "UPDATE subtitle_stored SET format = 'ass' WHERE id = '{other}'"
    ))
    .await;
    s.sql(format!(
        "UPDATE subtitle_jobs SET state = 'held' WHERE id = '{job}'"
    ))
    .await;
    assert_eq!(
        s.choose(&other, Chosen::Apply).await,
        StoredChoice::Refused("이 보관본을 받은 작업이 보류 중이라 적용할 수 없어요.")
    );
}

#[tokio::test]
async fn a_copy_with_no_creator_is_never_added() {
    let s = setup().await;
    let (_, srt) = ass_applied_srt_stored(&s).await;
    // Neither the applied ASS nor the SRT names a creator: no match.
    s.sql("UPDATE subtitle_stored SET creator = NULL".into())
        .await;
    assert_eq!(
        s.choose(&srt, Chosen::Add).await,
        StoredChoice::Refused(ADD_REFUSED)
    );
    // The creator's name makes it one.
    s.sql(format!("UPDATE subtitle_stored SET creator = '{CREATOR}'"))
        .await;
    assert!(matches!(
        s.choose(&srt, Chosen::Add).await,
        StoredChoice::Queued { .. }
    ));
}

#[tokio::test]
async fn a_works_own_order_decides_its_first_apply_and_going_back_restores_the_common_one() {
    let s = setup().await;
    let settings = SettingsStore::new(s.db.clone());
    let srt_first = FormatOrder::from_codes(&["srt", "ass", "smi"]).unwrap();
    assert!(settings
        .put_work_format_order(WORK, srt_first, 10)
        .await
        .unwrap());
    // Another work follows the common order.
    let formats = |order: Vec<SubtitleFormat>| order.iter().map(|f| f.code()).collect::<Vec<_>>();
    assert_eq!(
        formats(s.store.place.format_order(WORK).await.unwrap()),
        ["srt", "ass", "smi"]
    );
    assert_eq!(
        formats(s.store.place.format_order("other").await.unwrap()),
        ["ass", "srt", "smi"]
    );

    // A package with both formats: the work's first apply takes the SRT.
    let job = make(
        &s,
        "c1",
        CREATOR,
        "/pack/Show - 02.ass/Show - 02.srt",
        false,
    )
    .await;
    run(&s).await;
    done(&detail(&s, &job).await);
    assert_eq!(s.read(SRT), fake::bytes_of("Show - 02.srt"));
    assert!(!s.at(TARGET).exists());

    // Back on the common order, the next episode's first apply takes the ASS.
    assert!(settings.delete_work_format_order(WORK).await.unwrap());
    assert!(settings.work_format_order(WORK).await.unwrap().is_none());
    assert_eq!(
        formats(s.store.place.format_order(WORK).await.unwrap()),
        ["ass", "srt", "smi"]
    );
    let job = make_at(
        &s,
        "c2",
        CREATOR,
        "/pack/Show - 03.ass/Show - 03.srt",
        false,
        "3",
    )
    .await;
    run(&s).await;
    done(&detail(&s, &job).await);
    assert_eq!(
        s.read("Season 01/Show S01E03.ass"),
        fake::bytes_of("Show - 03.ass")
    );
    assert!(!s.at("Season 01/Show S01E03.srt").exists());
}

/// The row of the job's plan that keeps `stored`.
async fn row_of(s: &Setup, job: &str, stored: &str) -> PlanRow {
    s.store
        .place
        .plan(job)
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.stored_id.as_deref() == Some(stored))
        .expect("the row")
}

/// [`ass_applied_srt_stored`], then `names` written beside the video with
/// the SRT's bytes, as a person put them there: the job and the SRT's
/// stored subtitle.
async fn srt_stored_and_beside(s: &Setup, names: &[&str]) -> (String, String) {
    let (job, srt) = ass_applied_srt_stored(s).await;
    for name in names {
        std::fs::write(s.at(name), fake::bytes_of("Show - 02.srt")).unwrap();
    }
    (job, srt)
}

#[tokio::test]
async fn a_chosen_copy_whose_bytes_are_beside_the_video_becomes_its_applied_copy() {
    let s = setup().await;
    let (job, srt) = srt_stored_and_beside(&s, &[SRT]).await;
    let ass = s.stored("Show - 02.ass").await;

    // The episode has a subtitle, so the person is to compare; the worker
    // finds the same bytes beside the video and compares nothing.
    assert_eq!(
        s.choose(&srt, Chosen::Apply).await,
        StoredChoice::Queued {
            job_id: job.clone(),
            compare: true
        }
    );
    run(&s).await;
    done(&detail(&s, &job).await);
    assert_eq!(
        s.read(SRT),
        fake::bytes_of("Show - 02.srt"),
        "left as it is"
    );
    assert_eq!(
        s.applied().await,
        [(TARGET.to_owned(), ass), (SRT.to_owned(), srt.clone())]
    );
    assert_eq!(
        s.count("SELECT count(*) FROM subtitle_replacements".into())
            .await,
        0
    );
    let row = row_of(&s, &job, &srt).await;
    assert_eq!(row.outcome, Some(Outcome::Applied));
    assert_eq!(row.note.as_deref(), Some(ADOPTED));
    assert!(row.applied_id.is_some());
    // It is the applied copy now: the card offers nothing for it.
    assert_eq!(
        s.choose(&srt, Chosen::Apply).await,
        StoredChoice::Refused("이 보관본은 이미 영상 옆에 적용했어요.")
    );
}

#[tokio::test]
async fn the_identical_file_at_another_name_is_the_applied_copy_unless_the_targets_own_is_too() {
    const KO: &str = "Season 01/Show S01E02.ko.srt";
    // Only `.ko.srt` has the bytes: it is the applied copy, and nothing is
    // written at the target's name.
    let s = setup().await;
    let (job, srt) = srt_stored_and_beside(&s, &[KO]).await;
    s.choose(&srt, Chosen::Apply).await;
    run(&s).await;
    done(&detail(&s, &job).await);
    assert!(!s.at(SRT).exists());
    assert!(s.applied().await.contains(&(KO.to_owned(), srt)));

    // Both have them: the target's own name wins.
    let s = setup().await;
    let (job, srt) = srt_stored_and_beside(&s, &[KO, SRT]).await;
    s.choose(&srt, Chosen::Apply).await;
    run(&s).await;
    done(&detail(&s, &job).await);
    let applied = s.applied().await;
    assert!(applied.contains(&(SRT.to_owned(), srt)), "{applied:?}");
    assert_eq!(applied.len(), 2, "{applied:?}");
}

#[tokio::test]
async fn a_job_leaves_a_file_applied_for_another_copy_but_a_chosen_copy_takes_it_over() {
    let s = setup().await;
    // The same bytes from two sources are two stored copies of one file.
    let first = make(&s, "c1", CREATOR, "/ok/Show-02", true).await;
    run(&s).await;
    done(&detail(&s, &first).await);
    let kept = row_of_first(&s, &first).await;
    let twin = make(&s, "c2", CREATOR, "/ok/Show-02", false).await;
    run(&s).await;
    done(&detail(&s, &twin).await);
    let twin_stored = row_of_first(&s, &twin).await;
    assert_ne!(kept, twin_stored);
    // The job's own apply found the file applied for the first: stored only.
    assert_eq!(s.applied().await, [(TARGET.to_owned(), kept.clone())]);
    assert_eq!(
        row_of(&s, &twin, &twin_stored).await.outcome,
        Some(Outcome::Existing)
    );

    // A person chose the twin: the file is its copy.
    assert!(matches!(
        s.choose(&twin_stored, Chosen::Apply).await,
        StoredChoice::Queued { .. }
    ));
    run(&s).await;
    done(&detail(&s, &twin).await);
    assert_eq!(
        s.applied().await,
        [(TARGET.to_owned(), twin_stored.clone())]
    );
    assert_eq!(
        row_of(&s, &twin, &twin_stored).await.outcome,
        Some(Outcome::Applied)
    );
    assert_eq!(s.read(TARGET), fake::ass("Show-02"));
}

#[tokio::test]
async fn an_identical_file_is_not_recorded_as_applied_when_the_stored_file_is_gone_or_changed() {
    const STORED: &str = ".trss/subtitles/제작자/Show - 02.srt";
    for altered in [false, true] {
        let s = setup().await;
        let (job, srt) = srt_stored_and_beside(&s, &[SRT]).await;
        assert_eq!(s.read(STORED), fake::bytes_of("Show - 02.srt"));
        match altered {
            false => std::fs::remove_file(s.at(STORED)).unwrap(),
            true => std::fs::write(s.at(STORED), b"changed").unwrap(),
        }
        let applied_before = s.applied().await;

        assert!(matches!(
            s.choose(&srt, Chosen::Apply).await,
            StoredChoice::Queued { .. }
        ));
        run(&s).await;

        // The person's file stays what it is, unrecorded: the stored copy
        // was the only other holder of its bytes.
        assert_eq!(s.read(SRT), fake::bytes_of("Show - 02.srt"));
        assert_eq!(s.applied().await, applied_before, "altered: {altered}");
        let row = row_of(&s, &job, &srt).await;
        assert_eq!(row.outcome, Some(Outcome::Held), "altered: {altered}");
        assert!(
            row.note
                .as_deref()
                .is_some_and(|n| n.contains("보관본이 기록과 달라")),
            "{:?}",
            row.note
        );
        assert_eq!(row.applied_id, None);
        assert_eq!(
            s.count("SELECT count(*) FROM subtitle_replacements".into())
                .await,
            0
        );
    }
}

/// The stored subtitle of the job's first plan row.
async fn row_of_first(s: &Setup, job: &str) -> String {
    s.store.place.plan(job).await.unwrap()[0]
        .stored_id
        .clone()
        .expect("stored")
}

#[tokio::test]
async fn an_applied_rows_note_is_not_shown_for_another_episodes_apply_step() {
    let s = setup().await;
    // Episode 2 has a subtitle the app did not apply, which the job's
    // package replaces once approved; episode 3 is stored only.
    std::fs::write(s.at(TARGET), b"mine").unwrap();
    let job = make(
        &s,
        "c1",
        CREATOR,
        "/pack/Show - 02.ass/Show - 03.ass",
        false,
    )
    .await;
    run(&s).await;
    waiting_for_approval(&detail(&s, &job).await);
    let plan = view(&s, &job).await.plan;
    decide(&s, &job, &plan, true).await;
    run(&s).await;
    done(&detail(&s, &job).await);
    let two = row_of(&s, &job, &s.stored("Show - 02.ass").await).await;
    assert_eq!(
        two.note.as_deref(),
        Some("기존 자막을 새 자막으로 교체했어요")
    );

    // A person chooses episode 3's copy, whose bytes are beside its video.
    let three = s.stored("Show - 03.ass").await;
    let beside = "Season 01/Show S01E03.ass";
    std::fs::write(s.at(beside), fake::bytes_of("Show - 03.ass")).unwrap();
    assert!(matches!(
        s.choose(&three, Chosen::Apply).await,
        StoredChoice::Queued { .. }
    ));
    run(&s).await;
    let d = detail(&s, &job).await;
    done(&d);
    let row = row_of(&s, &job, &three).await;
    assert_eq!(row.outcome, Some(Outcome::Applied));
    assert_eq!(row.note.as_deref(), Some(ADOPTED));
    assert_eq!(s.read(beside), fake::bytes_of("Show - 03.ass"));
    let apply = d.steps.iter().find(|s| s.step == StepKind::Apply).unwrap();
    assert_eq!(
        (apply.state, apply.note.as_deref()),
        (StepState::Done, None)
    );
}

#[tokio::test]
async fn migration_58_marks_a_chosen_row_apply_or_add_and_reads_older_rows_as_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.db");
    let conn = trss_core::db::database_at(&path, 57);
    let columns = |c: &rusqlite::Connection| -> Vec<String> {
        let mut stmt = c.prepare("PRAGMA table_info(subtitle_job_plan)").unwrap();
        let names = stmt.query_map([], |r| r.get::<_, String>(1)).unwrap();
        names.map(Result::unwrap).collect()
    };
    assert!(!columns(&conn).contains(&"chosen".to_owned()));
    // A row of an earlier build, its parents not written (foreign keys off
    // for it only).
    conn.pragma_update(None, "foreign_keys", false).unwrap();
    conn.execute_batch(
        "INSERT INTO subtitle_job_plan
             (job_id, position, file_id, name, kind, size, sha256, action, updated_at)
             VALUES ('j1', 0, 'f1', 'a.ass', 'subtitle', 1, printf('%064d', 1), 'apply', 0);",
    )
    .unwrap();
    drop(conn);
    let db = Db::open(&path).await.unwrap();

    db.run(move |c| {
        assert!(columns(c).contains(&"chosen".to_owned()));
        let chosen = |c: &rusqlite::Connection| -> Option<String> {
            c.query_row("SELECT chosen FROM subtitle_job_plan", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(chosen(c), None);
        let set = |value: &str| {
            c.execute(
                &format!("UPDATE subtitle_job_plan SET chosen = {value}"),
                [],
            )
        };
        // Only the two ways a person chooses, or none.
        assert!(set("'other'").is_err());
        assert!(set("''").is_err());
        assert!(set("'APPLY'").is_err());
        assert_eq!(chosen(c), None);
        set("'add'").unwrap();
        assert_eq!(chosen(c).as_deref(), Some("add"));
        set("'apply'").unwrap();
        assert_eq!(chosen(c).as_deref(), Some("apply"));
        set("NULL").unwrap();
        assert_eq!(chosen(c), None);
        Ok::<_, trss_core::DbError>(())
    })
    .await
    .unwrap();
}
