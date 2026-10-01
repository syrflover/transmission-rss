//! Running a search: the first page, and, when it may be cut, the extra
//! searches of the episodes still missing.
//!
//! The tracker returns [`PAGE_LIMIT`] results at the most and ignores the page
//! number, so a first page of exactly that many may have left results out. The
//! episodes the work lacks, in the range, and that no result of the first page
//! is an episode of are then searched in groups of [`BATCH_SIZE`], written in
//! the notation of the first results' titles
//! (`[SubsPlease] One Piece - (1000|1001|1002) 1080p`), one after the other
//! with the host's request spacing between them ([`super::client`]), and the
//! results are merged into one list without repeats (an item the tracker
//! returns twice, by its identity key, is kept once).
//!
//! A first page that is not full ends the search at once. An extra search that
//! fails keeps what was read and says so; only the first page's failure fails
//! the search.

use std::collections::HashSet;

use super::{
    client::{SearchClient, SearchError},
    judge::{Range, World},
    query::batch_query,
    release::{read, Kind, Notation},
    BATCH_SIZE, MAX_EXTRA_SEARCHES, MAX_RESULTS, PAGE_LIMIT,
};
use crate::{store::channels::Channel, transmission::Redactor, worker::feed::FeedItem};

/// What a search read.
#[derive(Debug, Clone, Default)]
pub struct Found {
    /// Every result, once, newest first within each page, the first page first.
    pub items: Vec<FeedItem>,
    /// Whether the first page was full.
    pub first_full: bool,
    /// How many extra searches were sent, and how many the missing episodes
    /// needed (fewer were sent when that is more than [`MAX_EXTRA_SEARCHES`]).
    pub extra_sent: usize,
    pub extra_needed: usize,
    /// What the person should know about how complete the results are.
    pub notes: Vec<String>,
}

/// The values of a run that the tests set (the defaults are the module's constants).
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub batch_size: usize,
    pub max_extra: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            batch_size: BATCH_SIZE,
            max_extra: MAX_EXTRA_SEARCHES,
        }
    }
}

/// The notation most of the picked individual results write their number in.
fn common_notation(titles: &[&str]) -> Option<Notation> {
    let mut counts: Vec<(Notation, usize)> = Vec::new();
    for title in titles {
        let Some(notation) = read(title).notation else {
            continue;
        };
        match counts.iter_mut().find(|(n, _)| *n == notation) {
            Some((_, count)) => *count += 1,
            None => counts.push((notation, 1)),
        }
    }
    counts
        .iter()
        .fold(None::<&(Notation, usize)>, |best, next| match best {
            Some(best) if best.1 >= next.1 => Some(best),
            _ => Some(next),
        })
        .map(|(notation, _)| *notation)
}

struct Merge {
    items: Vec<FeedItem>,
    seen: HashSet<String>,
    full: bool,
}

impl Merge {
    fn add(&mut self, page: Vec<FeedItem>) {
        for item in page {
            if self.items.len() >= MAX_RESULTS {
                self.full = true;
                return;
            }
            if self.seen.insert(item.identity_key.clone()) {
                self.items.push(item);
            }
        }
    }
}

/// Runs a search of `query` for a rule whose match phrase is `phrase`.
/// `picks` is the rule's judgement of a title; `progress(sent, needed)` is
/// called before each extra search.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    client: &SearchClient,
    channel: &Channel,
    redactor: &Redactor,
    query: &str,
    phrase: &str,
    range: Range,
    world: &World,
    picks: &(dyn Fn(&str) -> bool + Sync),
    limits: Limits,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Found, SearchError> {
    let first = client.page(channel, query, redactor).await?;
    let mut merged = Merge {
        items: Vec::new(),
        seen: HashSet::new(),
        full: false,
    };
    merged.add(first.items);
    let mut found = Found {
        first_full: first.raw_count >= PAGE_LIMIT,
        ..Found::default()
    };
    if !found.first_full {
        found.items = merged.items;
        return Ok(found);
    }

    // The episodes of the range the work lacks and no first result is.
    let covered: HashSet<u32> = merged
        .items
        .iter()
        .filter(|item| picks(&item.title))
        .filter_map(|item| match read(&item.title).kind {
            Kind::Episode { episode, .. } => Some(episode.number),
            _ => None,
        })
        .collect();
    let wanted: Vec<u32> = (range.from..=range.to)
        .filter(|n| !world.has_release(*n) && !covered.contains(n))
        .collect();
    let groups: Vec<&[u32]> = wanted.chunks(limits.batch_size.max(1)).collect();
    found.extra_needed = groups.len();

    if groups.is_empty() {
        found.items = merged.items;
        return Ok(found);
    }
    let picked: Vec<&str> = merged
        .items
        .iter()
        .filter(|item| picks(&item.title))
        .map(|item| item.title.as_str())
        .collect();
    let Some(notation) = common_notation(&picked) else {
        found.notes.push(
            "첫 결과가 가득 찼지만 릴리스 제목에서 회차 표기를 읽지 못해 빠진 회차를 따로 검색하지 못했어요."
                .to_owned(),
        );
        found.items = merged.items;
        return Ok(found);
    };

    let sending = groups.len().min(limits.max_extra);
    if groups.len() > sending {
        found.notes.push(format!(
            "빠진 회차가 많아서 추가 검색을 {sending}번까지만 보냈어요. 범위의 뒤쪽 회차는 찾지 못했을 수 있어요."
        ));
    }
    let mut cut = 0;
    for (index, numbers) in groups.into_iter().take(sending).enumerate() {
        let Some(words) = batch_query(query, phrase, notation, numbers) else {
            found
                .notes
                .push("검색어에 일치 문구가 없어서 빠진 회차를 따로 검색하지 못했어요.".to_owned());
            break;
        };
        progress(index, sending);
        match client.page(channel, &words, redactor).await {
            Ok(page) => {
                found.extra_sent += 1;
                if page.raw_count >= PAGE_LIMIT {
                    cut += 1;
                }
                merged.add(page.items);
            }
            Err(err) => {
                found.notes.push(match err {
                    SearchError::Busy(wait) => format!(
                        "추가 검색 중 서버가 {}초 기다리라고 해서 {}번째부터는 보내지 않았어요.",
                        wait.as_secs(),
                        index + 1
                    ),
                    _ => format!(
                        "추가 검색 {}번째를 읽지 못해서 거기서 멈췄어요. 읽은 결과만 보여드려요.",
                        index + 1
                    ),
                });
                break;
            }
        }
    }
    progress(found.extra_sent, sending);
    if cut > 0 {
        found.notes.push(format!(
            "추가 검색 {cut}번의 결과가 가득 차서 일부가 잘렸을 수 있어요."
        ));
    }
    if merged.full {
        found.notes.push(format!(
            "결과가 {MAX_RESULTS}개를 넘어서 나머지는 담지 않았어요."
        ));
    }
    found.items = merged.items;
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(key: &str, title: &str) -> FeedItem {
        FeedItem {
            identity_key: key.to_owned(),
            title: title.to_owned(),
            stored_title: title.to_owned(),
            link: String::new(),
            stored_link: String::new(),
        }
    }

    fn merge() -> Merge {
        Merge {
            items: Vec::new(),
            seen: HashSet::new(),
            full: false,
        }
    }

    #[test]
    fn an_item_two_pages_return_is_kept_once_in_its_first_place() {
        let mut merged = merge();
        merged.add(vec![item("a", "One - 03"), item("b", "One - 02")]);
        merged.add(vec![item("b", "One - 02"), item("c", "One - 01")]);
        let keys: Vec<_> = merged
            .items
            .iter()
            .map(|i| i.identity_key.as_str())
            .collect();
        assert_eq!(keys, ["a", "b", "c"]);
        assert!(!merged.full);
    }

    #[test]
    fn the_results_kept_are_bounded() {
        let mut merged = merge();
        merged.add(
            (0..MAX_RESULTS + 5)
                .map(|n| item(&format!("k{n}"), "One - 01"))
                .collect(),
        );
        assert_eq!(merged.items.len(), MAX_RESULTS);
        assert!(merged.full);
    }

    #[test]
    fn the_notation_most_titles_use_is_the_one_searched_with() {
        let titles = [
            "[A] Show - 1001 (1080p)",
            "[A] Show - 1002 (1080p)",
            "[B] Show S02E03 [1080p]",
        ];
        assert_eq!(common_notation(&titles), Some(Notation::Dash { width: 4 }));
        assert_eq!(common_notation(&["Show Movie"]), None);
    }
}
