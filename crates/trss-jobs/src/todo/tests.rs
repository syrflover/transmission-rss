use super::*;
use crate::{
    model::{PathAction, PlanState, SubtitleFormat},
    place::{
        episode::Assignment,
        replace::records::{
            Compared, Comparison, Diff, Encoding, FileSeen, Format, Plan, PlanPath, PlanView, Side,
            StoredFacts, VideoSeen,
        },
    },
};
use trss_subtitles::compare::{Dialogue, FieldChange, Fonts, StyleChange, Styles, Timing};

// ---- the badges

/// A to-do of `kind` about `work`, the rest of it left plain.
fn todo_of(kind: &str, work: Option<&str>) -> Todo {
    at_time(kind, work, 1)
}

/// As [`todo_of`], at `at`.
fn at_time(kind: &str, work: Option<&str>, at: i64) -> Todo {
    let key = format!("{kind}:{}:{at}", work.unwrap_or("-"));
    let work = work.map(|id| WorkRef {
        id: id.to_owned(),
        name: id.to_owned(),
        cover_url: None,
    });
    let title = "작품".to_owned();
    match kind {
        "auth" => Todo::Auth {
            key,
            at,
            work,
            title,
            season: None,
            episodes: vec![],
            creator: None,
            reason: String::new(),
            job_id: "j".into(),
            jobs: 1,
        },
        "receive_failed" => Todo::ReceiveFailed {
            key,
            at,
            context: "revision",
            work,
            title,
            season: None,
            episodes: vec![],
            count: 1,
            reason: None,
            channel_id: None,
        },
        "episode_check" => Todo::EpisodeCheck {
            key,
            at,
            work,
            title,
            season: 1,
            creator: String::new(),
            source_id: String::new(),
            episodes: vec![],
            reason: None,
            sources: 1,
        },
        "placement_check" => Todo::PlacementCheck {
            key,
            at,
            work,
            title,
            season: None,
            creator: None,
            origin: "upload".into(),
            source: None,
            files: vec![],
            reason: None,
            job_id: "j".into(),
        },
        "video_check" => Todo::VideoCheck {
            key,
            at,
            work,
            title,
            season: 1,
            path: "Season 01/extra.mkv".into(),
            reason: String::new(),
            seen: "1:1".into(),
        },
        "replacement" => Todo::Replacement {
            key,
            at,
            work,
            title,
            season: None,
            episodes: vec![],
            creator: None,
            job_id: "j".into(),
            jobs: 1,
            changes: Changes::default(),
            current_received_at: None,
            current_changed_at: None,
            new_received_at: None,
        },
        _ => unreachable!("{kind}"),
    }
}

#[test]
fn a_works_badges_are_its_kinds_once_each_in_the_lists_order() {
    // As `list` orders them: the red kinds first.
    let todos = [
        todo_of("auth", Some("w2")),
        todo_of("receive_failed", Some("w1")),
        todo_of("receive_failed", Some("w1")),
        todo_of("replacement", Some("w1")),
        todo_of("replacement", None),
        todo_of("episode_check", Some("w3")),
        todo_of("placement_check", Some("w3")),
        todo_of("placement_check", Some("w1")),
        todo_of("video_check", Some("w4")),
        todo_of("video_check", Some("w3")),
    ];

    let badges = badges_by_work(&todos);

    assert_eq!(
        badges,
        std::collections::HashMap::from([
            (
                "w1".to_owned(),
                vec!["receive_failed", "replacement", "episode_check"]
            ),
            ("w2".to_owned(), vec!["auth"]),
            // A job's 배치 확인 and a video's episode are the same badge as
            // a mapping's.
            ("w3".to_owned(), vec!["episode_check"]),
            ("w4".to_owned(), vec!["episode_check"]),
        ])
    );
}

#[test]
fn a_todo_of_no_work_has_no_badge_and_a_list_of_none_has_no_badges() {
    assert!(badges_by_work(&[todo_of("auth", None), todo_of("video_check", None)]).is_empty());
    assert!(badges_by_work(&[]).is_empty());
}

// ---- the order

fn keys(list: &TodoList) -> Vec<String> {
    list.needs
        .iter()
        .map(|todo| match todo {
            Todo::Auth { key, .. }
            | Todo::ReceiveFailed { key, .. }
            | Todo::EpisodeCheck { key, .. }
            | Todo::PlacementCheck { key, .. }
            | Todo::VideoCheck { key, .. }
            | Todo::Replacement { key, .. } => key.clone(),
        })
        .collect()
}

#[test]
fn the_list_has_auth_then_receive_failed_then_replacement_then_the_checks_each_newest_first() {
    let list = ordered(
        vec![at_time("auth", Some("a"), 5), at_time("auth", Some("b"), 9)],
        vec![
            at_time("receive_failed", Some("a"), 7),
            at_time("receive_failed", Some("b"), 100),
        ],
        vec![
            at_time("replacement", Some("a"), 50),
            at_time("replacement", Some("b"), 2),
        ],
        vec![
            at_time("video_check", Some("a"), 3),
            at_time("episode_check", Some("a"), 8),
            at_time("placement_check", Some("a"), 4),
        ],
    );

    assert_eq!(list.count, 9);
    assert_eq!(
        keys(&list),
        [
            "auth:b:9",
            "auth:a:5",
            "receive_failed:b:100",
            "receive_failed:a:7",
            "replacement:a:50",
            "replacement:b:2",
            "episode_check:a:8",
            "placement_check:a:4",
            "video_check:a:3",
        ]
    );
}

#[test]
fn to_dos_of_the_same_time_keep_the_order_they_were_gathered_in() {
    let list = ordered(
        vec![
            at_time("auth", Some("x"), 1),
            at_time("auth", Some("y"), 1),
            at_time("auth", Some("z"), 1),
        ],
        vec![],
        vec![],
        vec![],
    );

    let works: Vec<_> = list.needs.iter().filter_map(Todo::work_id).collect();
    assert_eq!(works, ["x", "y", "z"]);
}

// ---- the change totals

fn side(format: Format) -> Side {
    Side {
        format,
        encoding: Encoding::Utf8,
        cues: 40,
    }
}

/// A comparison of two files of `old` and `new` formats, with every item
/// compared: 2 lines added, 12 changed, 3 timings, 2 styles, 2 fonts.
fn compared(old: Format, new: Format) -> Comparison {
    Comparison {
        path: "Season 01/Show S01E01.ass".into(),
        result: Compared::Diff(Box::new(Diff {
            old: side(old),
            new: side(new),
            dialogue: Dialogue {
                added: 2,
                changed: 12,
                removed: 0,
                lines: vec![],
            },
            timing: Timing {
                count: 3,
                lines: vec![],
            },
            styles: Some(Styles {
                added: vec!["Sign".into()],
                removed: vec![],
                changed: vec![StyleChange {
                    name: "Default".into(),
                    fields: vec![FieldChange {
                        field: "Fontname".into(),
                        old: "Arial".into(),
                        new: "Noto Sans CJK KR".into(),
                    }],
                }],
            }),
            fonts: Some(Fonts {
                added: vec!["Noto Sans CJK KR".into()],
                removed: vec!["Arial".into()],
            }),
            not_compared: vec![],
        })),
    }
}

fn unchanged(old: Format, new: Format) -> Comparison {
    let mut comparison = compared(old, new);
    if let Compared::Diff(diff) = &mut comparison.result {
        diff.dialogue = Dialogue::default();
        diff.timing = Timing::default();
        diff.styles = Some(Styles::default());
        diff.fonts = Some(Fonts::default());
    }
    comparison
}

#[test]
fn the_open_plans_are_summed_into_one_total() {
    let mut total = Changes::default();

    total.add(Some(&compared(Format::Ass, Format::Ass)));
    total.add(Some(&compared(Format::Ass, Format::Ass)));
    total.add(None);

    assert_eq!(
        total,
        Changes {
            added: 4,
            changed: 24,
            removed: 0,
            timing: 6,
            styles: 4,
            fonts: 4,
            uncompared: 1,
            partial: 0,
            plans: 3,
        }
    );
}

#[test]
fn a_compared_plan_with_no_difference_adds_only_itself_to_the_plans() {
    let mut total = Changes::default();

    total.add(Some(&unchanged(Format::Ass, Format::Ass)));

    assert_eq!(
        total,
        Changes {
            plans: 1,
            ..Changes::default()
        }
    );
}

#[test]
fn the_total_is_serialized_with_the_names_the_screen_reads() {
    let mut total = Changes::default();
    total.add(Some(&compared(Format::Ass, Format::Ass)));

    assert_eq!(
        serde_json::to_value(&total).unwrap(),
        serde_json::json!({
            "added": 2, "changed": 12, "removed": 0, "timing": 3, "styles": 2,
            "fonts": 2, "uncompared": 0, "partial": 0, "plans": 1,
        })
    );
}

// ---- when the subtitles were received

fn facts(received_at: i64) -> StoredFacts {
    StoredFacts {
        creator: None,
        format: SubtitleFormat::Ass,
        source_kind: "post".into(),
        post: None,
        received_at,
        encoding: None,
        asset_path: "a.ass".into(),
    }
}

/// An open plan whose current file `Show.ass` changed at `mtime_ns`; `applied`
/// says the file is the app's copy of a subtitle received at that time.
fn view(mtime_ns: i64, applied: Option<i64>, new: Option<i64>) -> PlanView {
    PlanView {
        plan: Plan {
            id: "p".into(),
            job_id: "j".into(),
            position: 0,
            version: 1,
            state: PlanState::Open,
            reason: None,
            work_id: "w".into(),
            season: 1,
            episode: 1,
            assignment: Assignment::Mapped,
            basis: None,
            folder: "Season 01".into(),
            video: VideoSeen {
                path: "Season 01/Show.mkv".into(),
                object: "1:1".into(),
                size: 1,
                mtime: 1,
            },
            stored_id: "s".into(),
            asset_id: "a".into(),
            asset_path: "a.ass".into(),
            asset_size: 1,
            asset_sha256: "0".into(),
            asset_lines: None,
            target: "Season 01/Show.ass".into(),
            created_at: 0,
            decided_at: None,
            paths: vec![PlanPath {
                path: "Season 01/Show.ass".into(),
                action: PathAction::Replace,
                file: Some(FileSeen {
                    size: 1,
                    sha256: "0".into(),
                    object: "1:1".into(),
                    mtime: mtime_ns,
                    lines: None,
                }),
                applied_id: applied.map(|_| "applied".into()),
            }],
        },
        previous: None,
        new: new.map(facts),
        applied: applied
            .map(|at| vec![("Season 01/Show.ass".to_owned(), facts(at))])
            .unwrap_or_default(),
        comparison: None,
    }
}

#[test]
fn a_current_file_the_app_applied_has_the_time_its_subtitle_was_received() {
    let mut received = Received::default();

    received.add(&view(9_000_000_000, Some(1_000), Some(2_000)));

    assert_eq!(received.current, Some(1_000));
    assert_eq!(received.current_changed, None);
    assert_eq!(received.new, Some(2_000));
}

#[test]
fn a_current_file_the_app_did_not_manage_has_its_change_time_in_milliseconds() {
    let mut received = Received::default();

    received.add(&view(5_500_000_000, None, None));

    assert_eq!(received.current, None);
    assert_eq!(received.current_changed, Some(5_500));
    assert_eq!(received.new, None);
}

#[test]
fn of_several_plans_the_newest_of_each_time_is_kept() {
    let mut received = Received::default();

    received.add(&view(0, Some(3_000), Some(7_000)));
    received.add(&view(0, Some(1_000), Some(9_000)));
    received.add(&view(0, Some(2_000), None));

    assert_eq!(received.current, Some(3_000));
    assert_eq!(received.new, Some(9_000));
}
