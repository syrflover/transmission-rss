use super::*;
use crate::mapping::MappingKind;

const ORDER: [SubtitleFormat; 3] = [
    SubtitleFormat::Ass,
    SubtitleFormat::Srt,
    SubtitleFormat::Smi,
];

fn offset(offset: i64) -> Mapping {
    Mapping {
        kind: MappingKind::User,
        offset: Some(offset),
        evidence: "시험".to_owned(),
        decided_at: 0,
        version: 1,
        exceptions: Vec::new(),
    }
}

fn format_of(name: &str) -> Format {
    match extension(name).as_deref() {
        Some("ass" | "ssa") => Format::Ass,
        Some("srt") => Format::Srt,
        Some("smi") => Format::Smi,
        Some("zip") => Format::Zip,
        _ => Format::Other,
    }
}

struct Case<'a> {
    candidates: Vec<Candidate<'a>>,
    names: Vec<&'a str>,
    sha: Vec<String>,
    mapping: Option<Mapping>,
    total: Option<u32>,
    season: u32,
    follow: bool,
}

impl<'a> Case<'a> {
    fn new(episode: &'a str, names: Vec<&'a str>) -> Case<'a> {
        let sha = (0..names.len()).map(|i| format!("{i:064}")).collect();
        Case {
            candidates: vec![Candidate {
                item_id: 1,
                episode,
            }],
            names,
            sha,
            mapping: None,
            total: None,
            season: 1,
            follow: false,
        }
    }

    fn plan(&self) -> Planned {
        let files: Vec<File<'_>> = self
            .names
            .iter()
            .map(|n| File {
                name: n,
                format: format_of(n),
            })
            .collect();
        let sha: Vec<&str> = self.sha.iter().map(String::as_str).collect();
        plan(
            &self.candidates,
            &files,
            &Context {
                mapping: self.mapping.as_ref(),
                total: self.total,
                season: self.season,
                order: &ORDER,
                follow: self.follow,
                sha256: &sha,
            },
        )
    }
}

fn episode(row: &Row) -> Option<i64> {
    row.placed.as_ref().map(|p| p.episode)
}

// The ticket's first case: a Tistory post whose attachments are every
// episode, received for one.
#[test]
fn the_candidates_file_is_applied_and_the_other_episodes_are_stored() {
    let names: Vec<String> = (1..=25).map(|n| format!("Show - {n:02}.ass")).collect();
    let mut case = Case::new("5", names.iter().map(String::as_str).collect());
    case.total = Some(25);
    let planned = case.plan();
    assert!(planned.missing.is_empty());
    for (i, row) in planned.rows.iter().enumerate() {
        assert_eq!(episode(row), Some(i as i64 + 1), "{row:?}");
        assert_eq!(row.question, None);
        match i {
            4 => {
                assert_eq!(row.action, PlanAction::Apply);
                assert_eq!(row.anissia_episode.as_deref(), Some("5"));
            }
            _ => {
                assert_eq!(row.action, PlanAction::Store, "{row:?}");
                assert_eq!(row.anissia_episode, None);
                assert_eq!(row.note.as_deref(), Some("고르지 않은 회차라 보관만 해요"));
            }
        }
    }
}

#[test]
fn the_subscribed_creators_job_applies_the_other_episodes_too() {
    let mut case = Case::new("5", vec!["Show - 05.ass", "Show - 06.ass"]);
    case.follow = true;
    let planned = case.plan();
    assert!(planned.rows.iter().all(|r| r.action == PlanAction::Apply));
    assert_eq!(episode(&planned.rows[1]), Some(6));
}

// The Naver post of `auto:14`: the SMI and its fonts.
#[test]
fn fonts_are_kept_beside_the_subtitle() {
    let case = Case::new(
        "1",
        vec![
            "Reincarnated.as.a.Sword.S02E01.The.Island.1080p.smi",
            "H2MPRB.TTF",
            "H2MKPB.TTF",
            "a옛날목욕탕L.ttf",
        ],
    );
    let planned = case.plan();
    let smi = &planned.rows[0];
    assert_eq!(smi.action, PlanAction::Apply);
    assert_eq!(episode(smi), Some(1));
    for font in &planned.rows[1..] {
        assert_eq!(font.kind, AssetKind::Font);
        assert_eq!(font.action, PlanAction::Store);
        assert_eq!(font.placed, None);
        assert_eq!(font.attachment_episode, None);
    }
}

#[test]
fn the_first_format_of_the_order_is_applied() {
    let case = Case::new("3", vec!["Show - 03.srt", "Show - 03.ass", "Show - 03.smi"]);
    let rows = case.plan().rows;
    assert_eq!(rows[1].action, PlanAction::Apply);
    for i in [0, 2] {
        assert_eq!(rows[i].action, PlanAction::Store);
        assert_eq!(episode(&rows[i]), Some(3));
        assert!(rows[i].note.as_deref().unwrap().contains("ASS를 적용"));
    }
}

// Two files of one name (the post of 하느: an ASS and an SMI, no number).
#[test]
fn an_unnumbered_package_of_one_episode_is_the_candidates() {
    let case = Case::new(
        "1",
        vec![
            "[Doomdos] - Paris ni Saku Etoile - [1080p U-NEXT WEB-DL].ass",
            "[Doomdos] - Paris ni Saku Etoile - [1080p U-NEXT WEB-DL].smi",
        ],
    );
    let rows = case.plan().rows;
    assert_eq!(rows[0].action, PlanAction::Apply);
    assert_eq!(rows[1].action, PlanAction::Store);
    assert_eq!(episode(&rows[1]), Some(1));
}

#[test]
fn alternatives_of_one_format_are_asked_and_none_is_taken() {
    let case = Case::new("3", vec!["Show - 03 [TV].ass", "Show - 03 [sign].ass"]);
    let rows = case.plan().rows;
    for row in &rows {
        assert_eq!(row.action, PlanAction::Apply);
        assert_eq!(episode(row), Some(3));
        assert!(
            row.question.as_deref().unwrap().contains("ASS 자막이 2개"),
            "{row:?}"
        );
    }
    // The same bytes twice are one file.
    let mut same = Case::new("3", vec!["Show - 03.ass", "Show - 03 (copy).ass"]);
    same.sha = vec!["a".repeat(64), "a".repeat(64)];
    let rows = same.plan().rows;
    assert_eq!(rows[0].action, PlanAction::Apply);
    assert_eq!(rows[1].action, PlanAction::Store);
    assert!(rows.iter().all(|r| r.question.is_none()));
}

#[test]
fn another_seasons_file_is_on_no_episode() {
    let mut case = Case::new("1", vec!["Show - 01.ass", "Show S03E01.ass"]);
    case.season = 2;
    let rows = case.plan().rows;
    assert_eq!(rows[0].action, PlanAction::Apply);
    assert_eq!(rows[1].placed, None);
    assert_eq!(rows[1].action, PlanAction::Store);
    assert!(
        rows[1].note.as_deref().unwrap().contains("시즌 3"),
        "{:?}",
        rows[1]
    );
    // Of the candidate's files under two marks, the job season's is its.
    let mut case = Case::new(
        "1",
        vec!["Show S02E01.ass", "Show S02E02.ass", "Show S03E01.ass"],
    );
    case.season = 2;
    let rows = case.plan().rows;
    assert_eq!(rows[0].action, PlanAction::Apply);
    assert_eq!(rows[0].question, None);
    assert_eq!(episode(&rows[1]), Some(2));
    assert_eq!(rows[2].placed, None);
    // Neither mark is the job's season: which is the candidate's is asked,
    // whatever their formats.
    case.season = 1;
    let rows = case.plan().rows;
    assert!(rows[0].question.is_some() && rows[2].question.is_some());
    let case = Case::new("1", vec!["Show S03E01.ass", "Show S04E01.srt"]);
    let rows = case.plan().rows;
    for row in &rows {
        assert_eq!(row.action, PlanAction::Apply);
        assert_eq!(
            row.question.as_deref(),
            Some("이 회차의 파일 이름에 시즌 표시가 여럿(시즌 3, 시즌 4) 있어 적용할 것을 골라야 해요")
        );
    }
    // Each episode under one mark of its own asks nothing.
    let mut case = Case::new("1", vec!["Show S03E01.ass", "Show S04E02.ass"]);
    case.season = 2;
    case.candidates.push(Candidate {
        item_id: 2,
        episode: "2",
    });
    let rows = case.plan().rows;
    assert!(rows.iter().all(|r| r.question.is_none()), "{rows:?}");
    assert_eq!((episode(&rows[0]), episode(&rows[1])), (Some(1), Some(2)));
    // A mark that is not the job's season, on the candidate's only file,
    // is the season the creator counts (a work kept as its own folder).
    let case = Case::new("1", vec!["Show S02E01.smi", "Show S02E02.smi"]);
    let rows = case.plan().rows;
    assert_eq!(rows[0].action, PlanAction::Apply);
    assert_eq!(episode(&rows[1]), Some(2));
}

#[test]
fn files_none_of_the_kept_kinds_are_dropped_with_the_reason() {
    let case = Case::new(
        "1",
        vec!["Show - 01.ass", "setup.exe", "readme.txt", "a.idx", "a.sub"],
    );
    let rows = case.plan().rows;
    assert_eq!(rows[1].action, PlanAction::Drop);
    assert_eq!(rows[1].outcome, Some(Outcome::Dropped));
    assert_eq!(rows[1].note.as_deref(), Some(NOT_KEPT));
    assert_eq!(rows[2].kind, AssetKind::Attachment);
    assert_eq!(rows[3].format, Some(SubtitleFormat::Other));
    assert_eq!(rows[4].kind, AssetKind::Companion);
    // An IDX alone says its SUB is missing.
    let rows = Case::new("1", vec!["Show - 01.ass", "a.idx"]).plan().rows;
    assert_eq!(
        rows[1].note.as_deref(),
        Some("짝인 SUB 파일이 묶음에 없어요")
    );
}

#[test]
fn the_package_numbers_as_its_candidates_file_does() {
    // Anissia's numbers: 14 is the season's 2, so 13 is its 1 and 24 its 12.
    let names: Vec<String> = (13..=25).map(|n| format!("Show - {n}.ass")).collect();
    let mut case = Case::new("14", names.iter().map(String::as_str).collect());
    case.mapping = Some(offset(-12));
    case.total = Some(12);
    case.season = 2;
    let rows = case.plan().rows;
    assert_eq!(rows[1].action, PlanAction::Apply);
    assert_eq!(episode(&rows[0]), Some(1));
    assert_eq!(
        rows[0].placed.as_ref().unwrap().basis,
        Some(Basis::Attachment)
    );
    assert_eq!(episode(&rows[11]), Some(12));
    // 25 is the season's 13, past its 12.
    assert_eq!(rows[12].placed, None);
    assert!(rows[12].note.as_deref().unwrap().contains("1–12화 밖"));

    // The season's numbers: 02 is the candidate's 14.
    let mut case = Case::new("14", vec!["Show S2 - 02.ass", "Show S2 - 05.ass"]);
    case.mapping = Some(offset(-12));
    case.total = Some(12);
    case.season = 2;
    let rows = case.plan().rows;
    assert_eq!(rows[0].action, PlanAction::Apply);
    assert_eq!(episode(&rows[1]), Some(5));
    assert_eq!(
        rows[1].placed.as_ref().unwrap().assignment,
        Assignment::Explicit
    );
}

#[test]
fn a_package_without_the_candidates_file_says_so() {
    // Nothing tells which numbers the names use, and the mapping moves them.
    let mut case = Case::new("14", vec!["a - 05.ass", "a - 06.ass"]);
    case.mapping = Some(offset(-12));
    case.total = Some(12);
    let planned = case.plan();
    assert_eq!(planned.missing.len(), 1);
    assert!(planned.missing[0].1.contains("14화"));
    assert!(planned.rows.iter().all(|r| r.placed.is_none()));
    assert!(planned.rows.iter().all(|r| r.question.is_none()));
    // With no mapping the numbers are the season's.
    let planned = Case::new("14", vec!["a - 05.ass", "a - 06.ass"]).plan();
    assert_eq!(episode(&planned.rows[0]), Some(5));
    assert_eq!(planned.rows[0].action, PlanAction::Store);
}

// `docs/specs/subtitles.md`, 파일의 회차: a candidate whose own episode cannot
// be told has its file asked about, not called missing.
#[test]
fn the_file_of_a_candidate_whose_episode_cannot_be_told_is_asked() {
    let names: Vec<String> = (13..=25).map(|n| format!("Show - {n}.ass")).collect();
    let undecided = Mapping {
        kind: MappingKind::Undecided,
        offset: None,
        ..offset(0)
    };
    let mut case = Case::new("14", names.iter().map(String::as_str).collect());
    case.mapping = Some(undecided);
    case.total = Some(12);
    let planned = case.plan();
    assert!(planned.missing.is_empty(), "{:?}", planned.missing);
    let row = &planned.rows[1];
    assert_eq!((row.placed.as_ref(), row.action), (None, PlanAction::Apply));
    assert_eq!(row.anissia_episode.as_deref(), Some("14"));
    assert!(row.question.as_deref().unwrap().contains("미정"), "{row:?}");
    // The others are on no episode, told why.
    assert!(planned
        .rows
        .iter()
        .enumerate()
        .all(|(i, r)| i == 1 || (r.placed.is_none() && r.question.is_none() && r.note.is_some())));

    // Past the season: candidate 26 moved by −12 is the 14th of 12.
    let names: Vec<String> = (25..=27).map(|n| format!("Show - {n}.ass")).collect();
    let mut case = Case::new("26", names.iter().map(String::as_str).collect());
    case.mapping = Some(offset(-12));
    case.total = Some(12);
    let planned = case.plan();
    assert!(planned.missing.is_empty());
    assert!(planned.rows[1].question.as_deref().unwrap().contains("밖"));
    assert!(planned.rows[0].question.is_none() && planned.rows[2].question.is_none());
}

// `docs/specs/subtitles.md`, 파일의 회차: a single file of another number.
#[test]
fn a_single_file_of_another_number_is_asked() {
    let mut case = Case::new("14", vec!["Show - 13.ass"]);
    case.mapping = Some(offset(-12));
    case.total = Some(12);
    let planned = case.plan();
    assert!(planned.missing.is_empty());
    let row = &planned.rows[0];
    assert_eq!(row.placed, None);
    assert_eq!(row.action, PlanAction::Apply);
    assert!(row.question.as_deref().unwrap().contains("13화"));
    // An unsupported format asks nothing: it is not applied anyway.
    let row = &Case::new("14", vec!["Show - 13.vtt"]).plan().rows[0];
    assert_eq!(
        (row.action, row.question.as_deref()),
        (PlanAction::Store, None)
    );
}

#[test]
fn season_marks_stand_on_their_own() {
    assert_eq!(season_mark("Show.S02E01.1080p.smi"), Some(2));
    assert_eq!(season_mark("Show S3 - 01.ass"), Some(3));
    assert_eq!(season_mark("Show s03e01.ass"), Some(3));
    assert_eq!(season_mark("Shows 01.ass"), None);
    assert_eq!(season_mark("[SubsPlease] Show - 01 (1080p).ass"), None);
    assert_eq!(season_mark("Show S100.ass"), None);
}

#[test]
fn members_are_told_by_format_and_name() {
    assert_eq!(
        member("a.ass", Format::Ass),
        Member::Subtitle(SubtitleFormat::Ass)
    );
    assert_eq!(
        member("a.ssa", Format::Ass),
        Member::Subtitle(SubtitleFormat::Other)
    );
    assert_eq!(
        member("a.vtt", Format::Other),
        Member::Subtitle(SubtitleFormat::Other)
    );
    assert_eq!(member("H2MPRB.TTF", Format::Other), Member::Font);
    assert_eq!(member("pack.rar", Format::Other), Member::Archive);
    assert_eq!(member("a.zip", Format::Zip), Member::Archive);
    assert_eq!(member("읽어주세요.TXT", Format::Other), Member::Attachment);
    assert_eq!(member("setup.exe", Format::Other), Member::Other);
}
