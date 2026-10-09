use crate::{store::history::HistoryResult, subscriptions::*};

fn item(id: i64, title: &str, seen: Millis) -> HistoryItem {
    HistoryItem {
        first_read: false,
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

#[test]
fn the_quarter_after_one_rolls_over_the_year() {
    let q = |year, number| Quarter { year, number };
    assert_eq!(q(2026, 1).next(), q(2026, 2));
    assert_eq!(q(2026, 3).next(), q(2026, 4));
    assert_eq!(q(2026, 4).next(), q(2027, 1));
}

#[test]
fn an_anime_belongs_to_the_quarter_it_started_in_else_to_the_one_the_subscription_began_in() {
    let anime = |start_date: Option<&str>| Anime {
        anime_no: 1,
        subject: "Show".into(),
        original_subject: None,
        week: 1,
        air_time: None,
        start_date: start_date.map(str::to_owned),
        end_date: None,
        status: "ON".into(),
        fetched_at: 0,
    };
    // 2026-09-30 15:00 UTC is already 4분기 in Seoul.
    let subscribed_at = 1_790_780_400_000;
    let q = |year, number| Quarter { year, number };

    // The start date decides, whenever the subscription began.
    assert_eq!(
        Quarter::of_anime(Some(&anime(Some("2027-01-05"))), subscribed_at),
        q(2027, 1)
    );
    assert_eq!(
        Quarter::of_anime(Some(&anime(Some("2026-07"))), subscribed_at),
        q(2026, 3)
    );
    // Without a readable start date, or without the anime, the subscription's.
    assert_eq!(
        Quarter::of_anime(Some(&anime(None)), subscribed_at),
        q(2026, 4)
    );
    assert_eq!(
        Quarter::of_anime(Some(&anime(Some("soon"))), subscribed_at),
        q(2026, 4)
    );
    assert_eq!(Quarter::of_anime(None, subscribed_at), q(2026, 4));
}
