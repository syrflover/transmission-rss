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
    /// The job's source is known.
    source: bool,
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
            source: true,
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
                source: self.source,
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
    // The candidate's file has no mark: the reason names the job's season.
    assert_eq!(
        rows[1].note.as_deref(),
        Some("회차에 붙이지 않고 보관만 해요: 파일 이름이 가리키는 시즌(3)이 이 작업의 시즌(2)과 달라 보여요")
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
fn another_season_is_told_against_the_mark_of_the_candidates_file() {
    // The candidate's file is marked with another season than the job's
    // (season 1); the other files are held against that mark.
    let case = Case::new(
        "1",
        vec!["Show S02E01.smi", "Show S02E02.smi", "Show S01E03.smi"],
    );
    let rows = case.plan().rows;
    assert_eq!(rows[0].action, PlanAction::Apply);
    assert_eq!(episode(&rows[1]), Some(2));
    assert_eq!(rows[2].placed, None);
    assert_eq!(
        rows[2].note.as_deref(),
        Some("회차에 붙이지 않고 보관만 해요: 파일 이름이 가리키는 시즌(1)이 후보의 1화 파일에 적힌 시즌(2)과 달라 보여요")
    );
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
    // A document whose bytes are a ZIP is no archive to unpack.
    assert_eq!(member("설명.docx", Format::Zip), Member::Attachment);
    assert_eq!(member("읽어주세요.TXT", Format::Other), Member::Attachment);
    assert_eq!(member("setup.exe", Format::Other), Member::Other);
}

impl Case<'_> {
    /// The plan of the files as a person's upload (no candidate).
    fn named(&self) -> Planned {
        let files: Vec<File<'_>> = self
            .names
            .iter()
            .map(|n| File {
                name: n,
                format: format_of(n),
            })
            .collect();
        let sha: Vec<&str> = self.sha.iter().map(String::as_str).collect();
        plan_named(
            1,
            &files,
            &Context {
                mapping: self.mapping.as_ref(),
                source: self.source,
                total: self.total,
                season: self.season,
                order: &ORDER,
                follow: false,
                sha256: &sha,
            },
        )
    }
}

/// Each row's episode and how it got there, action and question.
fn named_rows(planned: &Planned) -> Vec<(Option<i64>, Option<Assignment>, PlanAction, bool)> {
    planned
        .rows
        .iter()
        .map(|r| {
            (
                r.placed.as_ref().map(|p| p.episode),
                r.placed.as_ref().map(|p| p.assignment),
                r.action,
                r.question.is_some(),
            )
        })
        .collect()
}

#[test]
fn an_upload_of_an_unknown_creator_is_placed_under_the_same_numbers_and_one_past_the_season_asked()
{
    let names: Vec<String> = (1..=13).map(|n| format!("Show - {n:02}.ass")).collect();
    let mut case = Case::new("", names.iter().map(String::as_str).collect());
    case.source = false;
    case.total = Some(12);
    let planned = case.named();
    let rows = named_rows(&planned);
    for (i, row) in rows.iter().take(12).enumerate() {
        assert_eq!(
            *row,
            (
                Some(i as i64 + 1),
                Some(Assignment::Explicit),
                PlanAction::Apply,
                false
            )
        );
    }
    assert_eq!(rows[12], (None, None, PlanAction::Apply, true));
    assert_eq!(
        planned.rows[12].question.as_deref(),
        Some("13화가 시즌의 1–12화 밖이에요")
    );
    assert!(planned.missing.is_empty());
}

// A known creator without a mapping yet: the same numbers, which follow the
// mapping decided later.
#[test]
fn an_upload_of_a_creator_without_a_mapping_is_placed_until_one_is_decided() {
    let names: Vec<String> = (1..=3).map(|n| format!("Show - {n:02}.ass")).collect();
    let case = Case::new("", names.iter().map(String::as_str).collect());
    let planned = case.named();
    let placed: Vec<_> = planned
        .rows
        .iter()
        .map(|r| {
            r.placed
                .as_ref()
                .map(|p| (p.episode, p.assignment, p.basis))
        })
        .collect();
    assert_eq!(
        placed,
        (1..=3)
            .map(|n| Some((n, Assignment::SameNumber, Some(Basis::Attachment))))
            .collect::<Vec<_>>()
    );
}

#[test]
fn an_upload_of_a_mapped_creator_goes_through_the_mapping() {
    let names: Vec<String> = (13..=24).map(|n| format!("Show - {n:02}.ass")).collect();
    let mut case = Case::new("", names.iter().map(String::as_str).collect());
    case.total = Some(12);
    case.mapping = Some(offset(-12));
    let rows = named_rows(&case.named());
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(
            *row,
            (
                Some(i as i64 + 1),
                Some(Assignment::Mapped),
                PlanAction::Apply,
                false
            )
        );
    }
}

#[test]
fn an_upload_asks_about_an_undecided_mapping_a_decimal_another_season_and_no_number() {
    let mut case = Case::new(
        "",
        vec![
            "Show - 03.ass",
            "Show - 05.5.ass",
            "Show S03E01.ass",
            "Show.ass",
            "Show - 01-02.ass",
        ],
    );
    case.season = 2;
    let questions: Vec<Option<String>> =
        case.named().rows.into_iter().map(|r| r.question).collect();
    assert_eq!(
        questions,
        [
            None,
            Some("5.5화는 정수 회차가 아니라 시즌의 회차로 정하지 못했어요".to_owned()),
            Some("파일 이름이 가리키는 시즌(3)이 이 작업의 시즌(2)과 달라 보여요".to_owned()),
            Some("파일 이름에 회차 번호가 없어요".to_owned()),
            Some("파일 이름이 회차 여럿을 가리켜요".to_owned()),
        ]
    );
    case.mapping = Some(Mapping {
        offset: None,
        ..offset(0)
    });
    assert_eq!(
        case.named().rows[0].question.as_deref(),
        Some("이 제작자의 회차 대응이 아직 미정이에요")
    );
}

#[test]
fn an_upload_applies_the_first_format_and_asks_about_alternatives() {
    let mut case = Case::new(
        "",
        vec![
            "a/Show - 01.ass",
            "a/Show - 01.srt",
            "b/Show - 02.ass",
            "c/Show - 02.ass",
            "Show - 03.vtt",
            "font.ttf",
            "setup.exe",
        ],
    );
    case.total = Some(12);
    let planned = case.named();
    let rows = named_rows(&planned);
    assert_eq!(
        rows[0],
        (
            Some(1),
            Some(Assignment::SameNumber),
            PlanAction::Apply,
            false
        )
    );
    assert_eq!(
        rows[1],
        (
            Some(1),
            Some(Assignment::SameNumber),
            PlanAction::Store,
            false
        )
    );
    assert_eq!(
        rows[2],
        (
            Some(2),
            Some(Assignment::SameNumber),
            PlanAction::Apply,
            true
        )
    );
    assert_eq!(
        rows[3],
        (
            Some(2),
            Some(Assignment::SameNumber),
            PlanAction::Apply,
            true
        )
    );
    assert_eq!(
        rows[4],
        (
            Some(3),
            Some(Assignment::SameNumber),
            PlanAction::Store,
            false
        )
    );
    assert_eq!(planned.rows[5].kind, AssetKind::Font);
    assert_eq!(planned.rows[6].action, PlanAction::Drop);
}

fn placed(episode: Option<i64>, apply: bool, format: SubtitleFormat, sha256: &str) -> Placing<'_> {
    Placing {
        episode,
        apply,
        format,
        sha256,
    }
}

#[test]
fn a_persons_placing_applies_one_file_per_episode_in_the_format_order() {
    let a = "a".repeat(64);
    let b = "b".repeat(64);
    let decided = placing(
        &[
            placed(Some(1), true, SubtitleFormat::Srt, &a),
            placed(Some(1), true, SubtitleFormat::Ass, &b),
            placed(Some(2), true, SubtitleFormat::Ass, &a),
            placed(Some(2), true, SubtitleFormat::Ass, &a),
            placed(Some(3), false, SubtitleFormat::Ass, &a),
            placed(None, false, SubtitleFormat::Ass, &b),
            placed(Some(4), true, SubtitleFormat::Other, &b),
        ],
        &ORDER,
    )
    .unwrap();
    let actions: Vec<PlanAction> = decided.iter().map(|(a, _)| *a).collect();
    assert_eq!(
        actions,
        [
            PlanAction::Store,
            PlanAction::Apply,
            PlanAction::Apply,
            PlanAction::Store,
            PlanAction::Store,
            PlanAction::Store,
            PlanAction::Store,
        ]
    );
    assert_eq!(
        decided[0].1.as_deref(),
        Some("형식 순서에 따라 ASS를 적용하고 이 형식은 보관만 해요")
    );
    assert_eq!(
        decided[3].1.as_deref(),
        Some("같은 내용의 파일을 적용하므로 이 파일은 보관만 해요")
    );
    assert_eq!(
        decided[4].1.as_deref(),
        Some("배치 확인에서 적용하지 않기로 해 보관만 해요")
    );
    assert_eq!(
        decided[5].1.as_deref(),
        Some("회차에 붙이지 않고 보관만 해요")
    );
    assert_eq!(
        decided[6].1.as_deref(),
        Some("자동으로 적용하지 않는 형식이라 보관만 해요")
    );
}

#[test]
fn two_different_files_of_one_format_on_an_episode_are_refused() {
    let (a, b) = ("a".repeat(64), "b".repeat(64));
    let refused = placing(
        &[
            placed(Some(2), true, SubtitleFormat::Ass, &a),
            placed(Some(2), true, SubtitleFormat::Ass, &b),
        ],
        &ORDER,
    );
    assert_eq!(
        refused,
        Err(
            "2화에 적용할 ASS 자막이 2개예요. 하나만 남기고 나머지는 적용하지 않음으로 둬 주세요."
                .to_owned()
        )
    );
}
