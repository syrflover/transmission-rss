use super::*;
use crate::store::history::HistoryResult;

fn parsed(title: &str) -> (String, Option<String>) {
    let release = parse_release(title).unwrap_or_else(|| panic!("no work in {title:?}"));
    (release.work, release.episode)
}

fn item(id: i64, title: &str, seen: Millis) -> HistoryItem {
    HistoryItem {
        id,
        channel_id: "c".into(),
        channel_label: "feed".into(),
        identity_key: format!("guid:{id}"),
        title: title.into(),
        link: String::new(),
        first_seen_at: seen,
        last_seen_at: seen,
        result: HistoryResult::NoMatch,
        result_at: seen,
        rule_id: None,
        reason: None,
        torrent_hash: None,
    }
}

#[test]
fn the_work_is_the_title_between_the_group_and_the_episode() {
    for (title, work, episode) in [
        (
            "[SubsPlease] Work - 01 (1080p) [ABCD1234].mkv",
            "Work",
            "01",
        ),
        (
            "[SubsPlease] Re:Zero kara - Hajimeru - 12v2 (720p)",
            "Re:Zero kara - Hajimeru",
            "12v2",
        ),
        (
            "[Erai-raws] Work Name - 07 [1080p][Multiple Subtitle][ABCD]",
            "Work Name",
            "07",
        ),
        ("[Moozzi2] Work - 05.5 (BD 1920x1080 x265)", "Work", "05.5"),
        ("[Group] [Extra] Work - 03", "Work", "03"),
        ("Work Name S01E03 1080p WEB", "Work Name", "03"),
        (
            "[Beatrice-Raws] Kono Subarashii 04 [BDRip 1920x1080 HEVC FLAC]",
            "Kono Subarashii",
            "04",
        ),
        ("[Batch] Work - 01-12 (BD 1080p)", "Work", "01-12"),
        ("【Group】 작품 이름 - 02", "작품 이름", "02"),
        (
            "[SubsPlease] 86 - Eighty Six - 01 (1080p)",
            "86 - Eighty Six",
            "01",
        ),
    ] {
        assert_eq!(
            parsed(title),
            (work.to_owned(), Some(episode.to_owned())),
            "{title}"
        );
    }
    let release = parse_release("[SubsPlease] Work - 01 (1080p)").unwrap();
    assert_eq!(release.groups, ["SubsPlease"]);
}

#[test]
fn a_title_without_an_episode_keeps_its_work_without_the_trailing_details() {
    assert_eq!(
        parsed("[Group] Work Movie (BD 1080p) [ABCD]"),
        ("Work Movie".to_owned(), None)
    );
    assert_eq!(parsed("Plain Title"), ("Plain Title".to_owned(), None));
    assert!(parse_release("[Group]").is_none());
    assert!(parse_release("   ").is_none());
    assert!(parse_release("[Group] (1080p)").is_none());
}

#[test]
fn a_number_inside_the_title_is_not_taken_for_the_episode() {
    assert_eq!(
        parsed("[SubsPlease] 2.5 Dimensional Seduction - 03 (1080p)"),
        (
            "2.5 Dimensional Seduction".to_owned(),
            Some("03".to_owned())
        )
    );
    assert_eq!(
        parsed("[SubsPlease] Mob Psycho 100 - 03 (1080p)"),
        ("Mob Psycho 100".to_owned(), Some("03".to_owned()))
    );
}

#[test]
fn works_are_grouped_by_case_and_spacing_and_the_newest_comes_first() {
    let items = [
        item(1, "[SubsPlease] Work A - 01 (1080p)", 100),
        item(2, "[SubsPlease] Work B - 01 (1080p)", 150),
        item(3, "[Erai-raws] work  a - 02 [1080p]", 300),
        item(4, "[SubsPlease] Work A - 03 (1080p)", 200),
        item(5, "[Group]", 400),
    ];
    let groups = title_groups(&items);
    assert_eq!(groups.len(), 2);
    assert_eq!(
        groups[0],
        TitleGroup {
            work: "work  a".into(),
            latest_title: "[Erai-raws] work  a - 02 [1080p]".into(),
            items: 3,
            latest_seen_at: 300,
        }
    );
    assert_eq!(groups[1].work, "Work B");
    assert_eq!(groups[1].items, 1);
}

#[test]
fn the_folder_suggestion_drops_what_a_folder_name_cannot_hold() {
    assert_eq!(folder_suggestion("Work").as_deref(), Some("Work/Season 01"));
    assert_eq!(
        folder_suggestion("Re:Zero / Season: 2?").as_deref(),
        Some("Re Zero Season 2/Season 01")
    );
    assert_eq!(
        folder_suggestion("..hidden..").as_deref(),
        Some("hidden/Season 01")
    );
    assert_eq!(
        folder_suggestion("../../etc").as_deref(),
        Some("etc/Season 01")
    );
    assert_eq!(folder_suggestion(" : / ").as_deref(), None);
    let long = "가".repeat(100);
    let suggestion = folder_suggestion(&long).unwrap();
    let name = suggestion.strip_suffix("/Season 01").unwrap();
    assert!(name.len() <= MAX_FOLDER_BYTES && name.chars().all(|c| c == '가'));
}

#[test]
fn a_quarter_is_read_in_seoul_time_and_from_the_dates_anissia_gives() {
    // 2026-09-30 15:00 UTC is 2026-10-01 00:00 in Seoul: 4분기 already.
    let sept_30_utc_15 = 1_790_780_400_000;
    assert_eq!(
        Quarter::at(sept_30_utc_15 - 1),
        Quarter {
            year: 2026,
            number: 3
        }
    );
    assert_eq!(
        Quarter::at(sept_30_utc_15),
        Quarter {
            year: 2026,
            number: 4
        }
    );
    assert_eq!(
        Quarter::at(0),
        Quarter {
            year: 1970,
            number: 1
        }
    );
    assert_eq!(
        Quarter::of_date("2026-10-07"),
        Some(Quarter {
            year: 2026,
            number: 4
        })
    );
    assert_eq!(
        Quarter::of_date("2027-01"),
        Some(Quarter {
            year: 2027,
            number: 1
        })
    );
    assert_eq!(Quarter::of_date("2027-13-01"), None);
    assert_eq!(Quarter::of_date("soon"), None);
    assert!(
        Quarter {
            year: 2026,
            number: 4
        } < Quarter {
            year: 2027,
            number: 1
        }
    );
}
