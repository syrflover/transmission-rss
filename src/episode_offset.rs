//! The episode offset of a new season's rule (`docs/specs/collection.md`, 영상
//! 회차 변환): what the app can tell from the first release a rule picks.
//!
//! A rule's `episode` turns a release's number into the number in the video's
//! name. A subscription to a new season often starts at the number the earlier
//! seasons left off at (`- 25` after a 24-episode season), so the first video
//! would be named `S03E25`. When the grounds agree the app sets the offset
//! itself (`−24`), before the rule's first item is named; when they do not, it
//! leaves the value alone and suggests one (or says why it cannot) in the
//! rule's detail.
//!
//! This is not the 자막의 회차 대응 of `docs/specs/library.md`: that maps a
//! subtitle source's numbers to the library's, and nothing here reads or writes it.
//!
//! # What the grounds are
//!
//! - **The first release** the rule picks: the lowest whole number among the
//!   items of the cycle (or the one `다시 받기`) that first picks something for
//!   the rule. Its history records are the rule's first by the time they were
//!   recorded ([`crate::store::history::HistoryStore::first_titles_of_rule`]).
//! - **The season** the videos go to: the season the subscription is connected
//!   to when it is ([`crate::worker::season_link`]), else the season the rule's
//!   save folder names. A connection is made from videos the rule received, so
//!   a rule that has received nothing has none; its folder (`<work>/Season NN`)
//!   is where `trname` will put the first video, which is the same place the
//!   connection will later find it in. The work is the library's work of that
//!   folder under the collect folder.
//! - **The earlier seasons' episodes**: the sum of the AniList episode counts
//!   linked to seasons `1..N` of that work (season info, ticket 0017). Season 1
//!   has no earlier season, so its sum is 0 whether or not the library has the
//!   work. For a later season every earlier season must be in the library with
//!   AniList entries whose counts are all known; otherwise the sum is unknown.
//! - **What the season folder holds** already: the episodes with a video.
//!
//! # What the app decides
//!
//! With the first release `f` and the sum `P` of the earlier seasons:
//!
//! | grounds                                   | result                              |
//! | ----------------------------------------- | ----------------------------------- |
//! | `f = P + 1` and the folder has no video   | offset `−P` is set (`0` for `f = 1`) |
//! | `f = 1`, `P = 0`                          | offset `0` is set whatever the folder holds |
//! | `f = P + 1`, `P > 0`, the folder has videos | suggestion without a value: a video of the season exists, so which episode `f` is cannot be told (a split cour numbered on in the same folder) |
//! | `f > P + 1`, `P > 0`                      | suggestion `−P`                     |
//! | `f ≤ P` (not `P = 0`, `f = 1`)            | nothing: the numbers restart in the season |
//! | `P` unknown, `f > 1`                      | suggestion without a value, with the reason |
//! | `P` unknown, `f = 1`                      | nothing                             |
//! | `P = 0`, `f > 1`                          | nothing: the numbers already are the season's |
//!
//! Only a subscription whose offset is still `0` or `1` (both leave a number
//! as it is) and not set by the app is looked at, and only before it has
//! picked any item. A value the user typed is never replaced; typing `0` or `1`
//! on a rule that has picked nothing is not told apart from not having typed
//! anything.

use std::path::{Component, Path};

use crate::{
    seasons::combine::combine,
    store::{
        channels::{Rule, SeasonRef},
        library::{LibraryError, LibraryStore},
        seasons::{SeasonError, SeasonStore},
    },
    subscriptions::whole_episode,
};

/// Why the earlier seasons' episodes cannot be added up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    /// The library has no work in the rule's folder.
    NoWork,
    /// The work has no folder for this earlier season.
    NoSeason(u32),
    /// This earlier season has no AniList entry linked.
    NoLink(u32),
    /// An entry linked to this earlier season has no episode count.
    NoCount(u32),
}

/// The episodes of the seasons before the rule's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Previous {
    Known(u32),
    Unknown(Missing),
}

/// What is known about where the rule's first video goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Basis {
    /// The season number of the video's folder.
    pub season: u32,
    pub previous: Previous,
    /// The episodes of that season that already have a video, ascending.
    pub held: Vec<u32>,
}

/// What the grounds come to for a first release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The app sets this offset, for these grounds (`basis`, said as done;
    /// [`Verdict::as_suggestion`] says them as an offer).
    Auto {
        offset: i64,
        first: u32,
        total: u32,
        basis: String,
    },
    /// The user is shown this: a value to apply when there is one, and why.
    Suggest { value: Option<i64>, basis: String },
    /// There is nothing to convert or to tell.
    Nothing,
}

impl Verdict {
    /// What the rule's detail offers a rule that is past its first items: the
    /// value to apply, if there is one, and why. An offset the app would have
    /// set is an offer there, because the items already received are not
    /// renamed; an offset that changes nothing offers nothing.
    pub fn as_suggestion(self) -> Option<(Option<i64>, String)> {
        match self {
            Verdict::Auto { offset: 0, .. } | Verdict::Nothing => None,
            Verdict::Auto {
                offset,
                first,
                total,
                ..
            } => Some((
                Some(offset),
                format!(
                    "처음 받은 릴리스가 {first}화예요. AniList 기준 이전 시즌이 {total}화까지라 \
                     번호가 이어지니 {}로 시즌 1화가 돼요.",
                    signed(offset)
                ),
            )),
            Verdict::Suggest { value, basis } => Some((value, basis)),
        }
    }
}

/// A number with a real minus sign, as the screen writes offsets.
pub fn signed(value: i64) -> String {
    if value < 0 {
        format!("−{}", -value)
    } else {
        value.to_string()
    }
}

/// `1–12, 14` for the episodes given in ascending order.
fn ranges(episodes: &[u32]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut at = 0;
    while at < episodes.len() {
        let mut end = at;
        while end + 1 < episodes.len() && episodes[end + 1] == episodes[end] + 1 {
            end += 1;
        }
        out.push(if end == at {
            episodes[at].to_string()
        } else {
            format!("{}–{}", episodes[at], episodes[end])
        });
        at = end + 1;
    }
    out.join(", ")
}

impl Missing {
    /// A clause that says why, to put before the reason the number is unknown.
    fn clause(self) -> String {
        match self {
            Missing::NoWork => "라이브러리에 이 작품의 이전 시즌이 없어서".to_owned(),
            Missing::NoSeason(s) => format!("라이브러리에 시즌 {s} 폴더가 없어서"),
            Missing::NoLink(s) => format!("시즌 {s}에 AniList 항목이 이어져 있지 않아서"),
            Missing::NoCount(s) => format!("시즌 {s}의 AniList 회차 수를 몰라서"),
        }
    }
}

/// What the grounds come to for `first`, the first release's number.
pub fn decide(first: u32, basis: &Basis) -> Verdict {
    let held = !basis.held.is_empty();
    let first_i = i64::from(first);
    match basis.previous {
        Previous::Known(total) => {
            let total_i = i64::from(total);
            if first_i == total_i + 1 && (!held || total == 0) {
                let basis = if total == 0 {
                    "첫 릴리스가 1화라서 회차를 바꾸지 않아요.".to_owned()
                } else {
                    format!(
                        "AniList 기준 이전 시즌이 {total}화까지이고 첫 릴리스가 {first}화라서 \
                         {}로 정했어요.",
                        signed(-total_i)
                    )
                };
                Verdict::Auto {
                    offset: -total_i,
                    first,
                    total,
                    basis,
                }
            } else if first_i == total_i + 1 {
                Verdict::Suggest {
                    value: None,
                    basis: format!(
                        "시즌 폴더에 이미 {}화가 있어서 처음 본 릴리스 {first}화가 시즌 몇 화인지 \
                         알 수 없어요. 회차 변환을 직접 적어 주세요.",
                        ranges(&basis.held)
                    ),
                }
            } else if first_i > total_i + 1 && total > 0 {
                Verdict::Suggest {
                    value: Some(-total_i),
                    basis: format!(
                        "처음 본 릴리스가 {first}화예요. AniList 기준 이전 시즌이 {total}화까지라, \
                         번호가 이어진다면 {}로 시즌 {}화가 돼요.",
                        signed(-total_i),
                        first_i - total_i
                    ),
                }
            } else {
                Verdict::Nothing
            }
        }
        Previous::Unknown(missing) if first > 1 => Verdict::Suggest {
            value: None,
            basis: format!(
                "처음 본 릴리스가 {first}화인데, {} 시즌 몇 화인지 알 수 없어요. \
                 회차 변환을 직접 적어 주세요.",
                missing.clause()
            ),
        },
        Previous::Unknown(_) => Verdict::Nothing,
    }
}

/// The lowest whole episode among the release titles, if any has one.
pub fn first_release(titles: &[String]) -> Option<u32> {
    titles.iter().filter_map(|t| whole_episode(t)).min()
}

/// Whether the app may look at the rule's offset at all: a subscription whose
/// offset leaves numbers as they are and was not set by the app.
pub fn is_open(rule: &Rule) -> bool {
    rule.subscription.is_some() && !rule.episode_auto && matches!(rule.episode, 0 | 1)
}

/// The work folder and season number of a rule's save folder, which must be
/// `<work>/Season NN` as `trname` reads it.
pub fn place_of(directory: &str) -> Option<(String, u32)> {
    let parts: Vec<&str> = Path::new(directory)
        .components()
        .filter_map(|c| match c {
            Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect();
    let [work, season] = parts[..] else {
        return None;
    };
    let season = trname::Directory::from_normalized(season)?.season;
    Some((work.to_owned(), u32::try_from(season).ok()?))
}

#[derive(Debug, thiserror::Error)]
pub enum BasisError {
    #[error("cannot read the library: {0}")]
    Library(#[from] LibraryError),
    #[error("cannot read the season info: {0}")]
    Seasons(#[from] SeasonError),
}

/// Where the rule's first video goes and what is known of it, or `None` when
/// there is no telling (the collect folder is not set, the rule's folder is not
/// `<work>/Season NN`, or it is the specials folder).
pub async fn gather(
    library: &LibraryStore,
    seasons: &SeasonStore,
    collect_folder: &str,
    rule: &Rule,
) -> Result<Option<Basis>, BasisError> {
    let collect_folder = collect_folder.trim_end_matches('/');
    let Some((work_dir, folder_season)) = place_of(&rule.directory) else {
        return Ok(None);
    };
    let linked = rule
        .subscription
        .as_ref()
        .and_then(|s| s.season_id.as_deref())
        .and_then(SeasonRef::parse);
    let (work_id, season) = match linked {
        Some(link) => (Some(link.work_id), link.number),
        None => (
            library.work_in_folder(collect_folder, &work_dir).await?,
            folder_season,
        ),
    };
    if season == 0 {
        return Ok(None);
    }

    let held = match &work_id {
        Some(id) => library
            .season_episodes(id, season)
            .await?
            .map(|episodes| {
                episodes
                    .into_iter()
                    .filter(|(_, held)| held.video)
                    .map(|(number, _)| number)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let previous = match &work_id {
        _ if season == 1 => Previous::Known(0),
        None => Previous::Unknown(Missing::NoWork),
        Some(id) => previous_total(seasons, id, season).await?,
    };
    Ok(Some(Basis {
        season,
        previous,
        held,
    }))
}

/// The AniList episodes of seasons `1..season` of the work.
async fn previous_total(
    seasons: &SeasonStore,
    work_id: &str,
    season: u32,
) -> Result<Previous, SeasonError> {
    let before: Vec<u32> = (1..season).collect();
    let links = seasons
        .links_of_seasons(before.iter().map(|s| (work_id.to_owned(), *s)).collect())
        .await?;
    let mut total: u32 = 0;
    for (number, link) in before.into_iter().zip(links) {
        let Some(link) = link else {
            return Ok(Previous::Unknown(Missing::NoSeason(number)));
        };
        if link.entries.is_empty() {
            return Ok(Previous::Unknown(Missing::NoLink(number)));
        }
        let Some(count) = combine(&link.entries).and_then(|c| c.episodes) else {
            return Ok(Previous::Unknown(Missing::NoCount(number)));
        };
        let Some(sum) = total.checked_add(count) else {
            return Ok(Previous::Unknown(Missing::NoCount(number)));
        };
        total = sum;
    }
    Ok(Previous::Known(total))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basis(previous: Previous, held: &[u32]) -> Basis {
        Basis {
            season: 3,
            previous,
            held: held.to_vec(),
        }
    }

    fn offset_of(verdict: &Verdict) -> Option<i64> {
        match verdict {
            Verdict::Auto { offset, .. } => Some(*offset),
            _ => None,
        }
    }

    #[test]
    fn a_first_release_that_follows_the_earlier_seasons_is_converted_by_their_sum() {
        let verdict = decide(25, &basis(Previous::Known(24), &[]));
        assert_eq!(offset_of(&verdict), Some(-24));
        let Verdict::Auto { basis, .. } = verdict else {
            unreachable!()
        };
        assert!(basis.contains("24화") && basis.contains("25화") && basis.contains("−24"));
    }

    #[test]
    fn the_first_release_of_a_first_season_needs_no_conversion_even_without_the_work() {
        let first = Basis {
            season: 1,
            previous: Previous::Known(0),
            held: vec![],
        };
        assert_eq!(offset_of(&decide(1, &first)), Some(0));
        // Videos already in the folder cannot change that nothing is converted.
        let held = Basis {
            held: vec![1, 2],
            ..first
        };
        assert_eq!(offset_of(&decide(1, &held)), Some(0));
    }

    #[test]
    fn a_first_release_in_the_middle_of_a_season_is_a_suggestion_with_the_earlier_sum() {
        let verdict = decide(27, &basis(Previous::Known(24), &[]));
        let Verdict::Suggest { value, basis } = verdict else {
            panic!("{verdict:?}")
        };
        assert_eq!(value, Some(-24));
        assert!(
            basis.contains("27화") && basis.contains("시즌 3화"),
            "{basis}"
        );
    }

    #[test]
    fn numbers_that_restart_or_already_are_the_seasons_need_nothing() {
        assert_eq!(
            decide(1, &basis(Previous::Known(24), &[])),
            Verdict::Nothing
        );
        assert_eq!(
            decide(13, &basis(Previous::Known(24), &[1, 2])),
            Verdict::Nothing
        );
        assert_eq!(
            decide(27, &basis(Previous::Known(0), &[])),
            Verdict::Nothing
        );
    }

    #[test]
    fn a_season_folder_that_has_videos_keeps_the_app_from_choosing() {
        // A split cour numbered on in the same folder: 25 would be E01, which
        // the folder has.
        let verdict = decide(25, &basis(Previous::Known(24), &[1, 2, 3, 4, 5, 6]));
        let Verdict::Suggest { value, basis } = verdict else {
            panic!("{verdict:?}")
        };
        assert_eq!(value, None);
        assert!(basis.contains("1–6화"), "{basis}");
    }

    #[test]
    fn unknown_earlier_seasons_are_told_and_not_guessed() {
        let verdict = decide(25, &basis(Previous::Unknown(Missing::NoCount(2)), &[]));
        let Verdict::Suggest { value, basis } = verdict else {
            panic!("{verdict:?}")
        };
        assert_eq!(value, None);
        assert!(basis.contains("시즌 2의 AniList 회차 수"), "{basis}");
        assert_eq!(
            decide(1, &basis_unknown()),
            Verdict::Nothing,
            "a first release numbered 1 needs no sum"
        );
    }

    fn basis_unknown() -> Basis {
        basis(Previous::Unknown(Missing::NoWork), &[])
    }

    #[test]
    fn the_first_release_is_the_lowest_whole_episode() {
        let titles = |list: &[&str]| list.iter().map(|t| t.to_string()).collect::<Vec<_>>();
        assert_eq!(
            first_release(&titles(&[
                "[SubsPlease] Show - 26 (1080p) [AAAA1111].mkv",
                "[SubsPlease] Show - 25v2 (1080p) [BBBB2222].mkv",
                "[SubsPlease] Show (01-12) (Batch)",
            ])),
            Some(25)
        );
        assert_eq!(
            first_release(&titles(&["[SubsPlease] Show (01-12) (Batch)"])),
            None
        );
    }

    #[test]
    fn a_save_folder_names_its_work_and_season_the_way_trname_reads_it() {
        assert_eq!(place_of("Show/Season 03"), Some(("Show".into(), 3)));
        assert_eq!(place_of("./Show/Season 3/"), Some(("Show".into(), 3)));
        assert_eq!(place_of("Show/Season 00"), Some(("Show".into(), 0)));
        assert_eq!(place_of("Show"), None);
        assert_eq!(place_of("A/Show/Season 03"), None);
        assert_eq!(place_of("Show/Specials"), None);
    }

    // --- what is known of the place, from the library and the season info ---

    use std::collections::BTreeSet;

    use crate::{
        discovery::{EpisodeFile, FileKind, Scan, ScannedWork, WorkRead},
        store::{
            channels::{RuleState, Subscription, SubtitleMode},
            db::Db,
            seasons::{Entry, FuzzyDate},
        },
    };

    fn entry(id: i64, episodes: Option<u32>) -> Entry {
        Entry {
            id,
            romaji: None,
            english: None,
            native: None,
            format: None,
            status: None,
            episodes,
            start: FuzzyDate::default(),
            end: FuzzyDate::default(),
            studios: Vec::new(),
            genres: Vec::new(),
            description: None,
            airing: Vec::new(),
            sequels: Vec::new(),
            fetched_at: 1,
        }
    }

    fn rule(directory: &str, season_id: Option<&str>) -> Rule {
        Rule {
            id: "r".into(),
            channel_id: "c".into(),
            position: 0,
            version: 1,
            r#match: Some("Show".into()),
            regex: false,
            case_insensitive: false,
            directory: directory.into(),
            episode: 1,
            episode_auto: false,
            state: RuleState::Active,
            subscription: Some(Subscription {
                anissia_anime_no: 7,
                subtitles: SubtitleMode::Undecided,
                creator: None,
                season_id: season_id.map(str::to_owned),
                season_blocked: None,
                subscribed_at: 1,
                titled_at: None,
            }),
            resumed_at: None,
        }
    }

    struct Place {
        library: LibraryStore,
        seasons: SeasonStore,
        work: String,
    }

    /// `Show` under `/shows` with the given seasons, each with the videos
    /// named.
    async fn place(seasons: &[(u32, &[&str])]) -> Place {
        let db = Db::open_blocking(":memory:").unwrap();
        let library = LibraryStore::new(db.clone());
        let files = seasons
            .iter()
            .flat_map(|(season, episodes)| {
                episodes.iter().map(move |episode| EpisodeFile {
                    path: format!("Season {season:02}/Show S{season:02}E{episode}.mkv"),
                    kind: FileKind::Video,
                    season: *season,
                    episode: (*episode).to_owned(),
                })
            })
            .collect();
        let scan = Scan {
            works: vec![WorkRead::Read(ScannedWork {
                dir_name: "Show".into(),
                seasons: seasons.iter().map(|(s, _)| *s).collect::<BTreeSet<_>>(),
                files,
                unrecognized: Vec::new(),
            })],
        };
        let (folder, _) = library
            .add_folder("/shows".into(), scan, 100, &[])
            .await
            .unwrap();
        let work = library.works(&folder.id).await.unwrap().remove(0).id;
        Place {
            library,
            seasons: SeasonStore::new(db),
            work,
        }
    }

    impl Place {
        /// Links entries with these counts to the season.
        async fn link(&self, season: u32, counts: &[Option<u32>]) {
            let mut ids = Vec::new();
            for (index, count) in counts.iter().enumerate() {
                let id = i64::from(season) * 10 + index as i64;
                self.seasons.put_entry(entry(id, *count)).await.unwrap();
                ids.push(id);
            }
            let link = self.seasons.link(&self.work, season).await.unwrap();
            self.seasons
                .set_links(&self.work, season, link.version, ids)
                .await
                .unwrap();
        }

        async fn basis(&self, rule: &Rule) -> Option<Basis> {
            gather(&self.library, &self.seasons, "/shows/", rule)
                .await
                .unwrap()
        }
    }

    #[tokio::test]
    async fn the_earlier_seasons_are_the_sum_of_their_linked_counts_and_a_split_cour_adds_up() {
        let place = place(&[(1, &["01"]), (2, &["01"]), (3, &[])]).await;
        place.link(1, &[Some(12)]).await;
        // Two cours linked to one local season.
        place.link(2, &[Some(12), Some(13)]).await;

        let basis = place.basis(&rule("Show/Season 03", None)).await.unwrap();

        assert_eq!(basis.season, 3);
        assert_eq!(basis.previous, Previous::Known(37));
        assert!(basis.held.is_empty());
    }

    #[tokio::test]
    async fn a_season_with_videos_says_which_episodes_it_holds() {
        let place = place(&[(1, &["01"]), (2, &["01", "02", "04"])]).await;
        place.link(1, &[Some(12)]).await;

        let basis = place.basis(&rule("Show/Season 02", None)).await.unwrap();

        assert_eq!(basis.previous, Previous::Known(12));
        assert_eq!(basis.held, [1, 2, 4]);
    }

    #[tokio::test]
    async fn an_earlier_season_that_cannot_be_counted_makes_the_sum_unknown() {
        let place = place(&[(1, &["01"]), (2, &["01"]), (4, &[])]).await;
        let at = |season: u32| rule(&format!("Show/Season {season:02}"), None);

        // Season 1 has no AniList entry yet.
        let basis = place.basis(&at(3)).await.unwrap();
        assert_eq!(basis.previous, Previous::Unknown(Missing::NoLink(1)));

        // Season 1 is linked, season 2's entry does not know its count.
        place.link(1, &[Some(12)]).await;
        place.link(2, &[None]).await;
        let basis = place.basis(&at(3)).await.unwrap();
        assert_eq!(basis.previous, Previous::Unknown(Missing::NoCount(2)));

        // Season 3 has no folder at all.
        place.link(2, &[Some(12)]).await;
        let basis = place.basis(&at(4)).await.unwrap();
        assert_eq!(basis.previous, Previous::Unknown(Missing::NoSeason(3)));
    }

    #[tokio::test]
    async fn the_first_season_has_nothing_before_it_and_a_stranger_has_no_work() {
        let place = place(&[(1, &["01"])]).await;

        let first = place.basis(&rule("Show/Season 01", None)).await.unwrap();
        assert_eq!((first.previous, first.held), (Previous::Known(0), vec![1]));

        // A work the library does not have: nothing before season 1, nothing
        // known before a later season.
        let fresh = place.basis(&rule("Fresh/Season 01", None)).await.unwrap();
        assert_eq!((fresh.previous, fresh.held), (Previous::Known(0), vec![]));
        let later = place.basis(&rule("Fresh/Season 02", None)).await.unwrap();
        assert_eq!(later.previous, Previous::Unknown(Missing::NoWork));
    }

    #[tokio::test]
    async fn a_connected_season_is_the_place_not_the_folders_name() {
        let place = place(&[(1, &["01"]), (2, &[])]).await;
        place.link(1, &[Some(12)]).await;
        let connected = rule("Elsewhere/Season 09", Some(&format!("{}:2", place.work)));

        let basis = place.basis(&connected).await.unwrap();

        assert_eq!((basis.season, basis.previous), (2, Previous::Known(12)));
    }

    #[tokio::test]
    async fn a_folder_that_trname_cannot_read_or_the_specials_have_no_place() {
        let place = place(&[(0, &["01"]), (1, &[])]).await;
        for directory in [
            "Show",
            "Show/Specials",
            "Show/Season 00",
            "A/Show/Season 03",
            "",
        ] {
            assert_eq!(
                place.basis(&rule(directory, None)).await,
                None,
                "{directory}"
            );
        }
    }

    #[test]
    fn episodes_are_written_as_ranges() {
        assert_eq!(ranges(&[1, 2, 3, 5, 7, 8]), "1–3, 5, 7–8");
        assert_eq!(ranges(&[4]), "4");
    }
}
