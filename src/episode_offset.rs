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
//! | `f ≤ P`, `f − 1` is the sum of seasons `k..N−1` for some `k ≥ 2` | suggestion `−(f − 1)`: the numbers run on from season `k` (user decision, 2026-10-02) |
//! | `f ≤ P` otherwise (not `P = 0`, `f = 1`)  | nothing: the numbers restart in the season |
//! | `P` unknown, `f > 1`                      | suggestion without a value, with the reason |
//! | `P` unknown, `f = 1`                      | nothing                             |
//! | `P = 0`, `f > 1`                          | nothing: the numbers already are the season's |
//! | `f = 1`, the season's cours `1..C` are all in the folder | suggestion `C + 1`, before all of the above (see below) |
//!
//! The app sets an offset only on a subscription that has not picked any item
//! yet, whose offset is not automatic and which the app has never decided
//! before ([`may_decide`], and
//! [`EpisodeMark::decided`](crate::store::channels::EpisodeMark)), whatever
//! value its field holds: a value carried over from the previous season's
//! rule (`−24` for a third season that starts at `- 49`) is replaced by `−48`
//! as `0` or `1` is (user decision, 2026-10-02). The value it replaced is
//! kept with it. A field that already names the
//! releases as the offset would ([`same_effect`]) is left alone. After the
//! user changed or undid the app's value, the app never decides the rule
//! again.
//!
//! Suggestions are made to a rule the app has not decided, whatever its field
//! holds, when the value differs from it ([`worth_offering`], user direction,
//! 2026-10-02): a third season that started receiving with a carried-over
//! `−12` is offered `−24` once the earlier seasons are linked. A note without
//! a value is shown only while the field leaves numbers as they are (`0` or
//! `1`): it asks the user to write a value, and one is written already.
//!
//! Some groups number a season on from a later season rather than from the
//! first: a third season after two of 24 that starts at `- 25` counts from
//! season 2. That is never set by itself, because `f ≤ P` is also what a
//! season whose numbers restart looks like; it is offered when `f − 1` is
//! exactly the episodes of the seasons right before the rule's, counted back
//! from season `N − 1` (`2기부터 이어 센 번호로 보여요. 회차 변환을 −24로
//! 할까요?`). When several such runs match (an earlier season of 0 episodes),
//! the shortest is offered and the grounds say the others match too. Like the
//! `f > P + 1` suggestion it does not look at the season folder: by the time
//! the rule's detail offers it, the folder holds the videos the rule received
//! unconverted.
//!
//! A split cour whose second part restarts at `- 01` in the same season folder
//! (user decision, 2026-10-02) is offered a positive offset: the season has two
//! or more linked AniList entries, all with episode counts (its cours, in the
//! order they are linked); the folder holds every episode of the first `C`
//! episodes, the cours before a later one, and none after them; and the first
//! release is `1`. The offer is `C + 1`, which makes `- 01` episode `C + 1`
//! (a positive offset is where the numbering starts, `starts_episode_at`):
//! `2쿨을 1화부터 센 번호로 보여요. 1화를 13화로 받도록 회차 변환을 13으로
//! 할까요?` after a cour of 12. The sentence says what the value does, and
//! writes it without a sign, as the field shows it: `+13` would read as
//! "add 13". It is never set by the app. A folder missing an episode of those cours,
//! or holding one past them, offers nothing: which cour restarted is not clear.

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
    /// The AniList episodes of each season before the rule's, from season 1,
    /// when all are known (empty otherwise, and for season 1).
    pub earlier: Vec<u32>,
    /// The AniList episodes of each entry linked to that season, in the order
    /// they are linked, when there are two or more and all are known (empty
    /// otherwise).
    pub cours: Vec<u32>,
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
                     번호가 이어지니 {}{} 시즌 1화가 돼요.",
                    signed(offset),
                    particle_ro(offset)
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
    if let Some(verdict) = restarted_cour(first, basis) {
        return verdict;
    }
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
                        "AniList 기준 이전 시즌이 {total}화까지이고 첫 화가 {first}화라서 \
                         회차 변환을 {}{} 정했어요.",
                        signed(-total_i),
                        particle_ro(-total_i)
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
                         번호가 이어진다면 {}{} 시즌 {}화가 돼요.",
                        signed(-total_i),
                        particle_ro(-total_i),
                        first_i - total_i
                    ),
                }
            } else {
                run_on(first, basis)
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

/// A first release `f ≤ P` whose `f − 1` is the episodes of the seasons right
/// before the rule's, counted back from the last of them: a suggestion
/// `−(f − 1)` (see the module docs). The whole run (from season 1) is the
/// `f = P + 1` case and is not looked at here.
fn run_on(first: u32, basis: &Basis) -> Verdict {
    if first <= 1 {
        return Verdict::Nothing;
    }
    let want = u64::from(first - 1);
    // Season numbers `k` whose run `k..N−1` adds up to `f − 1`, nearest first.
    let mut starts: Vec<usize> = Vec::new();
    let mut sum: u64 = 0;
    for k in (2..=basis.earlier.len()).rev() {
        sum += u64::from(basis.earlier[k - 1]);
        if sum == want {
            starts.push(k);
        }
        if sum > want {
            break;
        }
    }
    let Some(&nearest) = starts.first() else {
        return Verdict::Nothing;
    };
    let offset = -i64::from(first - 1);
    let mut text = format!(
        "{nearest}기부터 이어 센 번호로 보여요. 회차 변환을 {}{} 할까요?",
        signed(offset),
        particle_ro(offset)
    );
    if starts.len() > 1 {
        let others: Vec<String> = starts[1..].iter().map(|k| format!("{k}기")).collect();
        text.push_str(&format!(
            " {}부터 센 것으로도 맞아서 가장 가까운 시즌부터 센 것으로 봤어요.",
            others.join("·")
        ));
    }
    Verdict::Suggest {
        value: Some(offset),
        basis: text,
    }
}

/// A later cour of the season that restarts at `- 01`, offered `C + 1` (see
/// the module docs), or `None` when the grounds do not say so.
fn restarted_cour(first: u32, basis: &Basis) -> Option<Verdict> {
    if first != 1 || basis.cours.len() < 2 {
        return None;
    }
    let held: std::collections::BTreeSet<u32> = basis.held.iter().copied().collect();
    // The cours before the last one whose episodes are all in the folder.
    let mut whole = None;
    let mut sum: u32 = 0;
    for (index, count) in basis.cours[..basis.cours.len() - 1].iter().enumerate() {
        let end = sum.checked_add(*count)?;
        if *count == 0 || !(sum + 1..=end).all(|e| held.contains(&e)) {
            break;
        }
        sum = end;
        whole = Some(index + 1);
    }
    let cours = whole?;
    // Episodes past them: part of the next cour is there already, so which
    // cour restarted cannot be told.
    if held.range(sum + 1..).next().is_some() {
        return None;
    }
    let value = i64::from(sum) + 1;
    Some(Verdict::Suggest {
        value: Some(value),
        basis: format!(
            "{}쿨을 1화부터 센 번호로 보여요. 1화를 {value}화로 받도록 회차 변환을 \
             {value}{} 할까요?",
            cours + 1,
            particle_ro(value)
        ),
    })
}

/// `으로` or `로` after a number read in Sino-Korean, by its last digit.
fn particle_ro(value: i64) -> &'static str {
    match value.unsigned_abs() % 10 {
        0 | 3 | 6 => "으로",
        _ => "로",
    }
}

/// The lowest whole episode among the release titles, if any has one.
pub fn first_release(titles: &[String]) -> Option<u32> {
    titles.iter().filter_map(|t| whole_episode(t)).min()
}

/// Whether the app may set the rule's offset, as far as the rule itself
/// tells: a subscription whose offset is not automatic, whatever value it
/// holds. The rule must also not have been decided before (kept beside it,
/// [`crate::store::channels::EpisodeMark::decided`]) and must not have picked
/// any item yet.
pub fn may_decide(rule: &Rule) -> bool {
    rule.subscription.is_some() && !rule.episode_auto
}

/// Whether a suggestion of `value` says something to the rule: a value that
/// names releases otherwise than its field does, or, without a value, a field
/// that still leaves numbers as they are (`0` or `1`). The rule must also be
/// one the app may decide and has not decided.
pub fn worth_offering(rule: &Rule, value: Option<i64>) -> bool {
    match value {
        Some(value) => !same_effect(value, rule.episode),
        None => matches!(rule.episode, 0 | 1),
    }
}

/// Whether two offsets name every release alike: equal, or both `0` and `1`,
/// which leave numbers as they are.
pub fn same_effect(a: i64, b: i64) -> bool {
    a == b || (matches!(a, 0 | 1) && matches!(b, 0 | 1))
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

/// The work (when the library has it) and the season number the rule's videos
/// go to, or `None` when there is no telling (the rule's folder is not
/// `<work>/Season NN`, or it is the specials folder).
async fn locate(
    library: &LibraryStore,
    collect_folder: &str,
    rule: &Rule,
) -> Result<Option<(Option<String>, u32)>, LibraryError> {
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
    Ok((season != 0).then_some((work_id, season)))
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
    let Some((work_id, season)) = locate(library, collect_folder, rule).await? else {
        return Ok(None);
    };

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
    let (previous, earlier) = match &work_id {
        _ if season == 1 => (Previous::Known(0), Vec::new()),
        None => (Previous::Unknown(Missing::NoWork), Vec::new()),
        Some(id) => previous_total(seasons, id, season).await?,
    };
    let cours = match &work_id {
        Some(id) => cours_of(seasons, id, season).await?,
        None => Vec::new(),
    };
    Ok(Some(Basis {
        season,
        previous,
        earlier,
        cours,
        held,
    }))
}

/// The season the rule's videos go to and the AniList episode count of that
/// season, which is `None` when the library has no such season, no AniList
/// entry is linked to it, or an entry has no count. The past episode search's
/// range starts from it.
pub async fn season_total(
    library: &LibraryStore,
    seasons: &SeasonStore,
    collect_folder: &str,
    rule: &Rule,
) -> Result<Option<(u32, Option<u32>)>, BasisError> {
    let Some((work_id, season)) = locate(library, collect_folder, rule).await? else {
        return Ok(None);
    };
    let Some(work_id) = work_id else {
        return Ok(Some((season, None)));
    };
    let links = seasons.links_of_seasons(vec![(work_id, season)]).await?;
    let total = links
        .into_iter()
        .next()
        .flatten()
        .filter(|link| !link.entries.is_empty())
        .and_then(|link| combine(&link.entries).and_then(|c| c.episodes));
    Ok(Some((season, total)))
}

/// The AniList episodes of each entry linked to the season, when two or more
/// are and all counts are known.
async fn cours_of(
    seasons: &SeasonStore,
    work_id: &str,
    season: u32,
) -> Result<Vec<u32>, SeasonError> {
    let link = seasons
        .links_of_seasons(vec![(work_id.to_owned(), season)])
        .await?
        .into_iter()
        .next()
        .flatten();
    let Some(link) = link.filter(|link| link.entries.len() >= 2) else {
        return Ok(Vec::new());
    };
    Ok(link
        .entries
        .iter()
        .map(|e| e.episodes)
        .collect::<Option<Vec<u32>>>()
        .unwrap_or_default())
}

/// The AniList episodes of seasons `1..season` of the work: their sum, and
/// each season's count when all are known.
async fn previous_total(
    seasons: &SeasonStore,
    work_id: &str,
    season: u32,
) -> Result<(Previous, Vec<u32>), SeasonError> {
    let before: Vec<u32> = (1..season).collect();
    let links = seasons
        .links_of_seasons(before.iter().map(|s| (work_id.to_owned(), *s)).collect())
        .await?;
    let unknown = |missing| Ok((Previous::Unknown(missing), Vec::new()));
    let mut total: u32 = 0;
    let mut counts = Vec::new();
    for (number, link) in before.into_iter().zip(links) {
        let Some(link) = link else {
            return unknown(Missing::NoSeason(number));
        };
        if link.entries.is_empty() {
            return unknown(Missing::NoLink(number));
        }
        let Some(count) = combine(&link.entries).and_then(|c| c.episodes) else {
            return unknown(Missing::NoCount(number));
        };
        let Some(sum) = total.checked_add(count) else {
            return unknown(Missing::NoCount(number));
        };
        total = sum;
        counts.push(count);
    }
    Ok((Previous::Known(total), counts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basis(previous: Previous, held: &[u32]) -> Basis {
        Basis {
            season: 3,
            previous,
            earlier: Vec::new(),
            cours: Vec::new(),
            held: held.to_vec(),
        }
    }

    /// Season `counts.len() + 1`, after seasons of these counts.
    fn after(counts: &[u32], held: &[u32]) -> Basis {
        Basis {
            season: counts.len() as u32 + 1,
            previous: Previous::Known(counts.iter().sum()),
            earlier: counts.to_vec(),
            cours: Vec::new(),
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
            earlier: vec![],
            cours: vec![],
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

    #[test]
    fn numbers_run_on_from_a_later_season_are_suggested_not_set() {
        // Two seasons of 24, a third that starts at `- 25`: counted from
        // season 2.
        let verdict = decide(25, &after(&[24, 24], &[]));
        assert_eq!(
            verdict,
            Verdict::Suggest {
                value: Some(-24),
                basis: "2기부터 이어 센 번호로 보여요. 회차 변환을 −24로 할까요?".to_owned(),
            }
        );
        // Seasons 2 and 3 of a fourth: `- 25` after 12, 12, 12.
        let verdict = decide(25, &after(&[12, 12, 12], &[]));
        assert_eq!(
            verdict.as_suggestion().map(|(value, _)| value),
            Some(Some(-24))
        );
        // A number that matches no run back from the season before is nothing.
        assert_eq!(decide(13, &after(&[24, 24], &[])), Verdict::Nothing);
        // Only runs that end at the season right before count: 13 after
        // 12, 24 would follow season 1 alone.
        assert_eq!(decide(13, &after(&[12, 24], &[])), Verdict::Nothing);
        // Numbers that restart are nothing.
        assert_eq!(decide(1, &after(&[24, 24], &[])), Verdict::Nothing);
    }

    #[test]
    fn several_runs_that_match_offer_the_shortest_and_say_so() {
        // Season 2 has no episodes, so seasons 3 and 2..3 both add up to 12.
        let verdict = decide(13, &after(&[12, 0, 12], &[]));
        let Verdict::Suggest { value, basis } = verdict else {
            panic!("{verdict:?}")
        };
        assert_eq!(value, Some(-12));
        assert!(
            basis.starts_with("3기부터 이어 센 번호로 보여요."),
            "{basis}"
        );
        assert!(basis.contains("2기부터 센 것으로도 맞아서"), "{basis}");
    }

    #[test]
    fn numbers_run_on_are_still_offered_once_the_folder_has_their_videos() {
        // The rule received `- 25` unconverted, as `S03E25`.
        let verdict = decide(25, &after(&[24, 24], &[25]));
        assert_eq!(
            verdict.as_suggestion().map(|(value, _)| value),
            Some(Some(-24))
        );
    }

    /// Season 2 after a season of 12, made of cours of these counts, with
    /// these episodes in its folder.
    fn split(cours: &[u32], held: impl IntoIterator<Item = u32>) -> Basis {
        Basis {
            season: 2,
            previous: Previous::Known(12),
            earlier: vec![12],
            cours: cours.to_vec(),
            held: held.into_iter().collect(),
        }
    }

    #[test]
    fn a_second_cour_that_restarts_at_one_is_offered_where_the_first_ended() {
        let verdict = decide(1, &split(&[12, 12], 1..=12));
        assert_eq!(
            verdict,
            Verdict::Suggest {
                value: Some(13),
                basis:
                    "2쿨을 1화부터 센 번호로 보여요. 1화를 13화로 받도록 회차 변환을 13으로 할까요?"
                        .to_owned(),
            }
        );
        // The value makes `- 01` episode 13, as the rename reads it.
        let folder = std::path::Path::new("/shows/Show/Season 02");
        assert_eq!(
            crate::worker::revisions::episode_name(
                folder,
                "[SubsPlease] Show - 01 (1080p) [ABCD0001].mkv",
                13
            )
            .as_deref(),
            Some("Show S02E13.mkv")
        );
        // A third cour after two whole ones.
        let verdict = decide(1, &split(&[12, 11, 13], 1..=23));
        let Verdict::Suggest { value, basis } = verdict else {
            panic!("{verdict:?}")
        };
        assert_eq!(value, Some(24));
        assert!(basis.starts_with("3쿨을"), "{basis}");
    }

    #[test]
    fn a_restart_is_offered_nothing_unless_the_earlier_cours_are_whole() {
        // Episode 7 of the first cour is missing.
        let missing = (1..=12).filter(|e| *e != 7);
        assert_eq!(decide(1, &split(&[12, 12], missing)), Verdict::Nothing);
        // Part of the second cour is there already.
        assert_eq!(decide(1, &split(&[12, 12], 1..=14)), Verdict::Nothing);
        // Nothing of the season is there.
        assert_eq!(decide(1, &split(&[12, 12], [])), Verdict::Nothing);
        // One entry: no cours to tell apart.
        assert_eq!(decide(1, &split(&[], 1..=12)), Verdict::Nothing);
        // A release that does not restart.
        assert_eq!(restarted_cour(13, &split(&[12, 12], 1..=12)), None);
    }

    #[test]
    fn a_number_takes_the_particle_its_reading_ends_with() {
        assert_eq!(particle_ro(13), "으로");
        assert_eq!(particle_ro(10), "으로");
        assert_eq!(particle_ro(-26), "으로");
        assert_eq!(particle_ro(25), "로");
        assert_eq!(particle_ro(-24), "로");
        assert_eq!(particle_ro(7), "로");
        let Verdict::Auto { basis, .. } = decide(31, &basis(Previous::Known(30), &[])) else {
            panic!("a sum of 30 and a first release of 31 set −30")
        };
        assert!(basis.contains("−30으로 정했어요"), "{basis}");
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
