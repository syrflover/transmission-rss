//! The save folder suggested for a release's work or a library work
//! (`super`): the season marker, the library work it names, and the season
//! AniList's split gives.

use std::collections::{BTreeSet, HashMap};

use trss_anilist::FuzzyDate;
use trss_anissia::{calendar::PartialDate, Anime};
use trss_core::{calendar::days_from_civil, Db};
use trss_library::{
    discovery::{Scan, ScannedWork, WorkRead},
    store::{library::LibraryStore, seasons::SeasonStore},
};

use super::*;
use crate::{release_name::ReleaseName, store::channels::ChannelStore, test_world::entry};

const COLLECT: &str = "/media/current";
const ARCHIVE: &str = "/media/shows";

fn date(year: i32, month: u32, day: u32) -> FuzzyDate {
    FuzzyDate {
        year: Some(year),
        month: Some(month),
        day: Some(day),
    }
}

/// An airing that starts on this day, as Anissia gives it.
fn starts(year: i32, month: u32, day: u32) -> Airing {
    Airing::Start(PartialDate::Day(days_from_civil(year, month, day)))
}

/// The collect and archive folders of a library, with works of the seasons
/// given, and AniList entries and Anissia anime linked to them.
struct Library {
    library: LibraryStore,
    seasons: SeasonStore,
    channels: ChannelStore,
    /// Work IDs by `<folder>/<name>`.
    ids: HashMap<String, String>,
    next_entry: std::cell::Cell<i64>,
}

impl Library {
    async fn new(collect: &[(&str, &[u32])], archive: &[(&str, &[u32])]) -> Library {
        let db = Db::open_blocking(":memory:").unwrap();
        let library = LibraryStore::new(db.clone());
        let mut ids = HashMap::new();
        for (folder, works) in [(COLLECT, collect), (ARCHIVE, archive)] {
            let scan = Scan {
                works: works
                    .iter()
                    .map(|(name, seasons)| {
                        WorkRead::Read(ScannedWork {
                            dir_name: (*name).into(),
                            seasons: seasons.iter().copied().collect::<BTreeSet<_>>(),
                            files: Vec::new(),
                            unrecognized: Vec::new(),
                        })
                    })
                    .collect(),
            };
            let registered = library.folders().await.unwrap();
            let (added, _) = library
                .add_folder(folder.into(), scan, 100, &registered)
                .await
                .unwrap();
            for work in library.works(&added.id).await.unwrap() {
                ids.insert(format!("{folder}/{}", work.dir_name), work.id);
            }
        }
        Library {
            library,
            seasons: SeasonStore::new(db.clone()),
            channels: ChannelStore::new(db),
            ids,
            next_entry: std::cell::Cell::new(1),
        }
    }

    fn id(&self, folder: &str, name: &str) -> &str {
        &self.ids[&format!("{folder}/{name}")]
    }

    /// Links AniList entries airing over these spans to a season, in order.
    async fn link(&self, folder: &str, name: &str, season: u32, spans: &[(FuzzyDate, FuzzyDate)]) {
        let work = self.id(folder, name).to_owned();
        let mut ids = Vec::new();
        for (start, end) in spans {
            let id = self.next_entry.get();
            self.next_entry.set(id + 1);
            let mut e = entry(id, Some(12));
            e.start = *start;
            e.end = *end;
            self.seasons.put_entry(e).await.unwrap();
            ids.push(id);
        }
        let link = self.seasons.link(&work, season).await.unwrap();
        self.seasons
            .set_links(&work, season, link.version, ids)
            .await
            .unwrap();
    }

    /// Links Anissia anime `anime_no` to a season.
    async fn link_anime(&self, folder: &str, name: &str, season: u32, anime_no: i64) {
        let anime = Anime {
            anime_no,
            subject: format!("작품 {anime_no}"),
            original_subject: None,
            week: 3,
            air_time: None,
            start_date: None,
            end_date: None,
            status: "ON".into(),
            fetched_at: 1,
        };
        self.channels
            .set_season_anime(self.id(folder, name), season, 0, Some(anime))
            .await
            .unwrap();
    }

    async fn ask(&self, asked: Asked<'_>, anime_no: Option<i64>, airing: Airing) -> Suggestion {
        self.try_ask(asked, anime_no, airing).await.unwrap()
    }

    async fn try_ask(
        &self,
        asked: Asked<'_>,
        anime_no: Option<i64>,
        airing: Airing,
    ) -> Option<Suggestion> {
        let folders = [COLLECT.to_owned(), ARCHIVE.to_owned()];
        suggest(
            &self.library,
            &self.seasons,
            &folders,
            asked,
            anime_no,
            airing,
        )
        .await
        .unwrap()
    }
}

#[test]
fn a_marker_names_the_season_and_leaves_the_name_before_it() {
    let read = |work: &str| {
        read_release_work(work).map(|r| (r.name, r.marker.map(|m| (m.text, m.season))))
    };
    let named = |name: &str, marker: &str, season: u32| {
        Some((name.to_owned(), Some((marker.to_owned(), season))))
    };

    assert_eq!(
        read("Nige Jouzu no Wakagimi S2"),
        named("Nige Jouzu no Wakagimi", "S2", 2)
    );
    assert_eq!(
        read("Mushoku Tensei s03"),
        named("Mushoku Tensei", "s03", 3)
    );
    assert_eq!(read("Show Season 2"), named("Show", "Season 2", 2));
    assert_eq!(read("Show Season2"), named("Show", "Season2", 2));
    assert_eq!(read("Show 2nd Season"), named("Show", "2nd Season", 2));
    assert_eq!(read("Show 3RD SEASON"), named("Show", "3RD SEASON", 3));
    assert_eq!(read("작품 2기"), named("작품", "2기", 2));
    assert_eq!(read("作品 第2期"), named("作品", "第2期", 2));
    // The words after a marker are the season's subtitle, not the work's name.
    assert_eq!(
        read("Ore dake Level Up na Ken Season 2 -Arise from the Shadow-"),
        named("Ore dake Level Up na Ken", "Season 2", 2)
    );
    // A colon is no folder character: `Season: 2` is `Season 2`.
    assert_eq!(
        read("Re:Zero / Season: 2?"),
        named("Re Zero", "Season 2", 2)
    );
    // Nothing before the marker: the words after it.
    assert_eq!(read("S2 Show"), named("Show", "S2", 2));

    // A cour is no season, and neither is season 0 or a word that only looks like one.
    let unmarked = |name: &str| Some((name.to_owned(), None));
    assert_eq!(read("Show Part 2"), unmarked("Show Part 2"));
    assert_eq!(read("작품 2쿨"), unmarked("작품 2쿨"));
    assert_eq!(read("Show S0"), unmarked("Show S0"));
    assert_eq!(read("Show Season 100"), unmarked("Show Season 100"));
    assert_eq!(read("Seasons Greetings"), unmarked("Seasons Greetings"));
    assert_eq!(
        read("Shingeki no Kyojin The Final Season"),
        unmarked("Shingeki no Kyojin The Final Season")
    );
    assert_eq!(read("S2"), None);
}

#[test]
fn a_folder_name_drops_what_a_folder_name_cannot_hold() {
    assert_eq!(folder_name("Work").as_deref(), Some("Work"));
    assert_eq!(
        folder_name("Re:Zero / Part: 2?").as_deref(),
        Some("Re Zero Part 2")
    );
    assert_eq!(folder_name("..hidden..").as_deref(), Some("hidden"));
    assert_eq!(folder_name("../../etc").as_deref(), Some("etc"));
    assert_eq!(folder_name(" : / "), None);
    let long = folder_name(&"가".repeat(100)).unwrap();
    assert!(long.len() <= MAX_FOLDER_BYTES && long.chars().all(|c| c == '가'));
    assert_eq!(directory("Work", 2), "Work/Season 02");
}

#[tokio::test]
async fn a_marked_release_names_the_library_work_and_the_marked_season() {
    let lib = Library::new(&[("Nige Jouzu no Wakagimi", &[0, 1, 2])], &[]).await;
    lib.link(
        COLLECT,
        "Nige Jouzu no Wakagimi",
        1,
        &[(date(2024, 7, 6), date(2024, 9, 28))],
    )
    .await;

    // The feedback of 0.7.0: `Nige Jouzu no Wakagimi S2/Season 01` was offered.
    let found = lib
        .ask(
            Asked::Release("Nige Jouzu no Wakagimi S2"),
            Some(7),
            starts(2026, 1, 10),
        )
        .await;
    assert_eq!(found.directory(), "Nige Jouzu no Wakagimi/Season 02");
    assert_eq!(found.why, Why::Marker("S2".into()));
    assert_eq!(
        found.library.as_ref().map(|w| w.folder.as_str()),
        Some(COLLECT)
    );
    assert_eq!(
        found.message(),
        "릴리스 제목의 ‘S2’ 표시를 시즌 번호로 읽었어요."
    );

    // `2nd Season` and `2기` read the same; case and spacing do not count.
    for release in [
        "Nige Jouzu no Wakagimi 2nd Season",
        "nige jouzu no  wakagimi 2기",
    ] {
        let found = lib
            .ask(Asked::Release(release), Some(7), starts(2026, 1, 10))
            .await;
        assert_eq!(
            found.directory(),
            "Nige Jouzu no Wakagimi/Season 02",
            "{release}"
        );
    }

    // `Part 2` is no marker: the name keeps it, and no library work has it.
    let found = lib
        .ask(
            Asked::Release("Nige Jouzu no Wakagimi Part 2"),
            Some(7),
            starts(2026, 1, 10),
        )
        .await;
    assert_eq!(found.directory(), "Nige Jouzu no Wakagimi Part 2/Season 01");
    assert_eq!(found.why, Why::NewWork);
    assert_eq!(found.library, None);
}

#[tokio::test]
async fn a_work_the_library_lacks_is_new_at_season_one_or_at_its_marker() {
    let lib = Library::new(&[("Other", &[1])], &[]).await;
    let found = lib
        .ask(Asked::Release("New Show"), Some(7), starts(2026, 1, 10))
        .await;
    assert_eq!(found.directory(), "New Show/Season 01");
    assert_eq!(found.why, Why::NewWork);
    assert_eq!(
        found.message(),
        "수집 폴더와 보관 폴더에 없는 작품이라 시즌 1로 제안해요."
    );

    let found = lib
        .ask(Asked::Release("New Show S3"), Some(7), starts(2026, 1, 10))
        .await;
    assert_eq!(found.directory(), "New Show/Season 03");
    assert_eq!(found.library, None);

    assert_eq!(
        lib.try_ask(Asked::Release(" : / "), None, starts(2026, 1, 10))
            .await,
        None
    );
}

/// Without a marker the season follows AniList's split: the last season's last
/// entry holds the airing (a second cour kept in one season) or not (a
/// sequel), and the release's numbering is never looked at.
#[tokio::test]
async fn without_a_marker_the_last_seasons_last_entry_decides_same_or_next() {
    let lib = Library::new(
        &[
            ("Sequel Show", &[1]),
            ("Two Cours", &[1]),
            ("Unlinked", &[1, 2]),
        ],
        &[],
    )
    .await;
    // Season 1 ended on episode 23; the sequel's release goes on at 24.
    lib.link(
        COLLECT,
        "Sequel Show",
        1,
        &[(date(2025, 4, 5), date(2025, 9, 20))],
    )
    .await;
    let release = ReleaseName::read("[SubsPlease] Sequel Show - 24 (1080p) [ABCD1234].mkv")
        .work
        .unwrap();
    let found = lib
        .ask(Asked::Release(&release), Some(7), starts(2026, 1, 10))
        .await;
    assert_eq!(found.directory(), "Sequel Show/Season 02");
    assert_eq!(
        found.message(),
        "마지막 시즌의 AniList 항목(2025-04-05 ~ 2025-09-20)이 방영 시작일(2026-01-10)을 담지 않아서 다음 시즌으로 제안해요."
    );

    // Two entries on season 1 (two cours AniList split): the last one decides,
    // and it is still airing, its end not known yet.
    lib.link(
        COLLECT,
        "Two Cours",
        1,
        &[
            (date(2025, 7, 1), date(2025, 9, 23)),
            (date(2026, 1, 8), FuzzyDate::default()),
        ],
    )
    .await;
    let found = lib
        .ask(Asked::Release("Two Cours"), Some(7), starts(2026, 1, 8))
        .await;
    assert_eq!(found.directory(), "Two Cours/Season 01");
    assert_eq!(
        found.message(),
        "마지막 시즌의 AniList 항목(2026-01-08 ~)이 방영 시작일(2026-01-08)을 담아서 같은 시즌으로 제안해요."
    );
    // A month Anissia gives alone is enough when the entry started in it.
    let found = lib
        .ask(
            Asked::Release("Two Cours"),
            Some(7),
            Airing::Start(PartialDate::Month(2026, 1)),
        )
        .await;
    assert_eq!(found.season, 1);

    // The last season has no entry linked: the next one.
    lib.link(
        COLLECT,
        "Unlinked",
        1,
        &[(date(2024, 1, 1), date(2024, 3, 30))],
    )
    .await;
    let found = lib
        .ask(Asked::Release("Unlinked"), Some(7), starts(2026, 1, 10))
        .await;
    assert_eq!(found.directory(), "Unlinked/Season 03");
    assert_eq!(found.why, Why::NoEntry);
}

#[tokio::test]
async fn a_last_season_linked_to_the_anime_already_is_kept() {
    let lib = Library::new(&[("Show", &[1, 2])], &[]).await;
    // Season 2 holds anime 7 (a subscription on another channel received it)
    // and has no AniList entry: without the link it would be season 3.
    lib.link_anime(COLLECT, "Show", 2, 7).await;
    let found = lib
        .ask(Asked::Release("Show"), Some(7), starts(2026, 1, 10))
        .await;
    assert_eq!(found.directory(), "Show/Season 02");
    assert_eq!(found.why, Why::Linked);
    let other = lib
        .ask(Asked::Release("Show"), Some(8), starts(2026, 1, 10))
        .await;
    assert_eq!(other.directory(), "Show/Season 03");
}

#[tokio::test]
async fn works_of_the_archive_folder_are_found_and_the_collect_folder_comes_first() {
    let lib = Library::new(
        &[("Both", &[1])],
        &[("Archived", &[1, 2]), ("Both", &[1, 2, 3])],
    )
    .await;
    let found = lib
        .ask(Asked::Release("Archived"), None, starts(2026, 1, 10))
        .await;
    assert_eq!(found.directory(), "Archived/Season 03");
    assert_eq!(
        found.library.as_ref().map(|w| w.folder.as_str()),
        Some(ARCHIVE)
    );

    let found = lib
        .ask(Asked::Release("Both"), None, starts(2026, 1, 10))
        .await;
    assert_eq!(
        found.library.as_ref().map(|w| w.folder.as_str()),
        Some(COLLECT)
    );
    assert_eq!(found.directory(), "Both/Season 02");

    let folders = [COLLECT.to_owned(), ARCHIVE.to_owned()];
    let listed: Vec<(String, String)> = folder_works(&lib.library, &folders)
        .await
        .unwrap()
        .into_iter()
        .map(|w| (w.folder, w.dir_name))
        .collect();
    assert_eq!(
        listed,
        [
            (COLLECT.to_owned(), "Both".to_owned()),
            (ARCHIVE.to_owned(), "Archived".to_owned()),
        ]
    );
}

/// A rule is no subscription: today stands for the airing. A work picked by
/// its folder name is taken as it is, markers and all.
#[tokio::test]
async fn a_picked_work_is_taken_as_named_and_a_rule_places_today() {
    let lib = Library::new(&[("Mushoku Tensei S2", &[1]), ("Airing", &[1])], &[]).await;
    lib.link(
        COLLECT,
        "Airing",
        1,
        &[(date(2026, 10, 2), FuzzyDate::default())],
    )
    .await;
    let today = Airing::Today(days_from_civil(2026, 10, 13));

    let found = lib.ask(Asked::Work("Airing"), None, today).await;
    assert_eq!(found.directory(), "Airing/Season 01");
    assert_eq!(
        found.message(),
        "마지막 시즌의 AniList 항목(2026-10-02 ~)이 오늘(2026-10-13)을 담아서 같은 시즌으로 제안해요."
    );

    let found = lib.ask(Asked::Work("Mushoku Tensei S2"), None, today).await;
    assert_eq!(found.directory(), "Mushoku Tensei S2/Season 02");
    assert_eq!(found.why, Why::NoEntry);

    let found = lib.ask(Asked::Work(" Typed Name "), None, today).await;
    assert_eq!(found.directory(), "Typed Name/Season 01");
    assert_eq!(found.why, Why::NewWork);
}

#[tokio::test]
async fn a_work_with_no_season_from_one_on_starts_at_one() {
    let lib = Library::new(&[("Movies", &[0])], &[]).await;
    let found = lib
        .ask(Asked::Release("Movies"), None, starts(2026, 1, 10))
        .await;
    assert_eq!(found.directory(), "Movies/Season 01");
    assert_eq!(found.why, Why::NoSeason);
}

#[test]
fn a_span_holds_an_airing_as_far_as_both_dates_are_known() {
    let span = |start: FuzzyDate, end: FuzzyDate| Span { start, end };
    let month = |year, month| FuzzyDate {
        year: Some(year),
        month: Some(month),
        day: None,
    };
    let day = |y, m, d| Airing::Today(days_from_civil(y, m, d));

    let finished = span(date(2024, 7, 6), date(2024, 9, 28));
    assert!(finished.holds(day(2024, 7, 6)));
    assert!(finished.holds(day(2024, 9, 28)));
    assert!(!finished.holds(day(2024, 7, 5)));
    assert!(!finished.holds(day(2024, 9, 29)));
    // A month alone on either side does not tell days of that month apart.
    assert!(span(month(2024, 7), month(2024, 9)).holds(day(2024, 9, 30)));
    assert!(finished.holds(Airing::Start(PartialDate::Month(2024, 9))));
    assert!(!finished.holds(Airing::Start(PartialDate::Month(2024, 10))));
    // An end not known yet is open; a start not known holds nothing.
    assert!(span(date(2024, 7, 6), FuzzyDate::default()).holds(day(2030, 1, 1)));
    assert!(!span(FuzzyDate::default(), date(2024, 9, 28)).holds(day(2024, 8, 1)));
}
