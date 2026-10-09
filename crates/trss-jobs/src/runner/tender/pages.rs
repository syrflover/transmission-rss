//! The pages of a bound run on the job's remote screen, for a find job and a
//! site's check alike (`docs/specs/jobs.md`, 작업 화면 안의 인증과 브라우저
//! 수명).
//!
//! While a run is bound to a job, the job's watch keeps a follower
//! ([`ScreenTender::follow_pages_of`]) that looks at the run's pages once a second:
//!
//! - It writes the pages that stayed, in the order they came, as the tabs of
//!   the screen ([`screen::ScreenStore::set_pages`]). A page counts once it is seen
//!   twice in a row, so one that opens and closes at once is never a tab.
//! - It closes the pages a person asked to close
//!   ([`screen::ScreenStore::take_close`]), never the page the run was bound with, in
//!   the browser.
//! - It decides which page the screen shows ([`Pages::look`]): the page a
//!   person chose ([`screen::ScreenStore::take_switch`]); else a page that has stayed
//!   since the last look (a popup); else, when the page shown is gone, the
//!   newest page left; else the page it shows now, so a person's choice of an
//!   older tab is not undone at the next look.

use std::{sync::Arc, time::Duration};

use tokio_util::sync::{CancellationToken, DropGuard};
use trss_subtitles::auth::AuthBrowser;

use super::ScreenTender;
use crate::screen;

/// How often the follower looks at the run's pages.
const PAGE_POLL: Duration = Duration::from_secs(1);

/// The pages of one run as the follower has seen them.
#[derive(Debug)]
struct Pages {
    /// The pages that stayed, in the order they came; a closed page stays in
    /// the list, as a page's ID never comes back.
    order: Vec<String>,
    /// The pages new at the last look.
    seen_once: Vec<String>,
    /// The page the screen shows.
    shown: String,
}

impl Pages {
    /// A run bound at its page `first`, whose screen shows `shown`.
    fn new(first: &str, shown: &str) -> Pages {
        let mut order = vec![first.to_owned()];
        if shown != first {
            order.push(shown.to_owned());
        }
        Pages {
            order,
            seen_once: Vec::new(),
            shown: shown.to_owned(),
        }
    }

    /// One look at the pages the run has `live`, with the page a person
    /// `chose` since the last look. The page the screen is to show, when it
    /// is not the one it shows.
    fn look(&mut self, live: &[String], chosen: Option<&str>) -> Option<String> {
        let fresh: Vec<String> = live
            .iter()
            .filter(|p| !self.order.contains(p))
            .cloned()
            .collect();
        let mut arrived = None;
        for page in &fresh {
            if self.seen_once.contains(page) {
                self.order.push(page.clone());
                arrived = Some(page.clone());
            }
        }
        self.seen_once = fresh;
        let chosen = chosen
            .filter(|page| live.iter().any(|p| p == page))
            .map(str::to_owned);
        let next = chosen.or(arrived).or_else(|| {
            if live.contains(&self.shown) {
                None
            } else {
                self.tabs(live).pop().or_else(|| live.last().cloned())
            }
        });
        next.filter(|page| *page != self.shown)
    }

    /// The pages of the run `live` that are tabs, in the order they came.
    fn tabs(&self, live: &[String]) -> Vec<String> {
        self.order
            .iter()
            .filter(|p| live.contains(p))
            .cloned()
            .collect()
    }
}

impl ScreenTender {
    /// Starts the follower of the bound run (see the module docs). It ends
    /// with the returned guard, which the job's watch holds.
    pub(super) fn follow_pages_of(
        &self,
        binding: &screen::Binding,
        browser: &Arc<dyn AuthBrowser>,
    ) -> DropGuard {
        let stop = CancellationToken::new();
        tokio::spawn(
            self.clone()
                .follow_pages(binding.clone(), browser.clone(), stop.clone()),
        );
        stop.drop_guard()
    }

    /// Follows the run's pages until the binding is not the one it was made
    /// for (the run or its item changed), the run is not live, or `stop`.
    async fn follow_pages(
        self,
        binding: screen::Binding,
        browser: Arc<dyn AuthBrowser>,
        stop: CancellationToken,
    ) {
        let (job, run) = (binding.job_id.as_str(), binding.run_id.as_str());
        let mut pages = Pages::new(&binding.first_target_id, &binding.target_id);
        // The tabs as last written, so that an unchanged list writes nothing.
        let mut written: Vec<String> = Vec::new();
        loop {
            tokio::select! {
                biased;
                _ = stop.cancelled() => return,
                _ = tokio::time::sleep(PAGE_POLL) => {}
            }
            if !browser.is_live(job, run) {
                return;
            }
            // The binding this follower was made for, as the screen has it
            // now: the screen may show another page than the follower last
            // moved it to.
            match self.screens.bound(job).await {
                Ok(Some(bound)) if bound.run_id == run && bound.item_id == binding.item_id => {
                    pages.shown = bound.target_id;
                }
                Ok(_) => return,
                Err(err) => {
                    eprintln!("Subtitle job {job}: following the run's pages: {err}");
                    continue;
                }
            }
            match self.screens.take_close(job, run).await {
                Ok(targets) => {
                    for target in targets {
                        // The page the run was bound with stays: the browser
                        // always has a window, and the post is where the
                        // person came from.
                        if target != binding.first_target_id
                            && browser.close_page(job, run, &target).await
                        {
                            println!("Subtitle job {job}: a page of the run was closed");
                        }
                    }
                }
                Err(err) => eprintln!("Subtitle job {job}: a request to close a page: {err}"),
            }
            let chosen = match self.screens.take_switch(job, run).await {
                Ok(chosen) => chosen,
                Err(err) => {
                    eprintln!("Subtitle job {job}: a request to show a page: {err}");
                    None
                }
            };
            let live = browser.pages(job, run);
            if live.is_empty() {
                continue;
            }
            let next = pages.look(&live, chosen.as_deref());
            // The tabs come first: a screen that connects to the new page
            // finds its tab in the list.
            let tabs = pages.tabs(&live);
            if tabs != written {
                match self.screens.set_pages(job, run, &tabs, self.now()).await {
                    Ok(_) => written = tabs,
                    Err(err) => eprintln!("Subtitle job {job}: the run's pages: {err}"),
                }
            }
            let Some(next) = next else {
                continue;
            };
            match self.screens.retarget(job, run, &next, self.now()).await {
                Ok(true) => pages.shown = next,
                // The binding is not this run's any more; the next look says.
                Ok(false) => {}
                Err(err) => {
                    eprintln!("Subtitle job {job}: following the run's pages: {err}");
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(pages: &[&str]) -> Vec<String> {
        pages.iter().map(|p| (*p).to_owned()).collect()
    }

    #[test]
    fn a_page_is_shown_once_it_has_stayed_for_two_looks() {
        let mut pages = Pages::new("a", "a");
        assert_eq!(pages.look(&ids(&["a"]), None), None);
        assert_eq!(pages.look(&ids(&["a", "b"]), None), None, "seen once");
        assert_eq!(
            pages.look(&ids(&["a", "b"]), None).as_deref(),
            Some("b"),
            "seen twice"
        );
        pages.shown = "b".to_owned();
        assert_eq!(pages.tabs(&ids(&["a", "b"])), ids(&["a", "b"]));
    }

    #[test]
    fn a_page_that_opens_and_closes_at_once_is_never_shown_nor_a_tab() {
        let mut pages = Pages::new("a", "a");
        assert_eq!(pages.look(&ids(&["a", "blink"]), None), None);
        assert_eq!(pages.look(&ids(&["a"]), None), None);
        assert_eq!(pages.look(&ids(&["a", "b"]), None), None);
        assert_eq!(pages.tabs(&ids(&["a", "blink", "b"])), ids(&["a"]));
    }

    #[test]
    fn a_choice_of_an_older_page_is_kept_until_a_new_page_stays_or_the_shown_one_closes() {
        let mut pages = Pages::new("a", "a");
        for _ in 0..2 {
            pages.look(&ids(&["a", "b"]), None);
        }
        pages.shown = "b".to_owned();
        // The person goes back to the first page.
        assert_eq!(
            pages.look(&ids(&["a", "b"]), Some("a")).as_deref(),
            Some("a")
        );
        pages.shown = "a".to_owned();
        // The look after it does not undo the choice.
        assert_eq!(pages.look(&ids(&["a", "b"]), None), None);
        assert_eq!(pages.look(&ids(&["a", "b"]), None), None);
        // A new page that stays is shown.
        assert_eq!(pages.look(&ids(&["a", "b", "c"]), None), None);
        assert_eq!(
            pages.look(&ids(&["a", "b", "c"]), None).as_deref(),
            Some("c")
        );
    }

    #[test]
    fn a_choice_of_a_page_that_is_gone_is_ignored() {
        let mut pages = Pages::new("a", "a");
        assert_eq!(pages.look(&ids(&["a"]), Some("gone")), None);
    }

    #[test]
    fn when_the_shown_page_closes_the_newest_page_left_is_shown() {
        let mut pages = Pages::new("a", "a");
        for _ in 0..2 {
            pages.look(&ids(&["a", "b", "c"]), None);
        }
        pages.shown = "c".to_owned();
        assert_eq!(pages.look(&ids(&["a", "b"]), None).as_deref(), Some("b"));
        pages.shown = "b".to_owned();
        assert_eq!(pages.look(&ids(&["a"]), None).as_deref(), Some("a"));
    }

    #[test]
    fn closing_a_page_that_is_not_shown_changes_nothing() {
        let mut pages = Pages::new("a", "a");
        for _ in 0..2 {
            pages.look(&ids(&["a", "b", "c"]), None);
        }
        pages.shown = "c".to_owned();
        assert_eq!(pages.look(&ids(&["a", "c"]), None), None);
        assert_eq!(pages.tabs(&ids(&["a", "c"])), ids(&["a", "c"]));
    }

    #[test]
    fn a_run_bound_at_a_popup_keeps_its_first_page_first() {
        let mut pages = Pages::new("a", "b");
        assert_eq!(pages.tabs(&ids(&["b", "a"])), ids(&["a", "b"]));
        assert_eq!(pages.look(&ids(&["a", "b"]), None), None);
    }
}
