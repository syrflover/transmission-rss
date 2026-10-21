//! What a remote screen tells a person about the pages of its run: the state
//! of the back and forward buttons and the host of the page shown, and the
//! tabs (see the protocol in [`super`]).
//!
//! Only the host of a page is ever sent. A full address can carry a signed
//! download link or a token, so no path, query or fragment leaves this module:
//! not in a host, not in a title (a page with no title is named by its address
//! in the browser's own list, with its escapes undone, and a title may quote
//! an address: [`tab_title`] replaces any title that may name one by nothing),
//! and never in a log.

use serde_json::{json, Value};
use url::{Position, Url};

/// The longest title of a tab, in characters; a longer one is cut.
const TITLE_CHARS: usize = 40;

/// The host of a page: the host name of an `http` or `https` address, with no
/// user, port, path, query or fragment. Pages with no host (`about:blank`,
/// `data:`, `blob:`, an error page) have none.
pub fn host_of(address: &str) -> Option<String> {
    let url = Url::parse(address).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.host_str().filter(|h| !h.is_empty()).map(str::to_owned)
}

/// The title a tab shows for a page at `address` titled `title`: cut to
/// [`TITLE_CHARS`] characters, and empty when the title may name an address
/// ([`names_address`]; the browser names a page that has no title by its
/// address), which the caller shows as the host.
pub fn tab_title(title: &str, address: &str) -> String {
    let title = title.trim();
    if title.is_empty() || names_address(title, address) {
        return String::new();
    }
    let mut cut: String = title.chars().take(TITLE_CHARS).collect();
    if title.chars().count() > TITLE_CHARS {
        cut.push('…');
    }
    cut
}

/// Whether `title` may name an address, so that it must not be sent: the
/// page's whole address; any address (`://`); the page's host on its own (the
/// browser's title of an untitled page at its root), or followed by a path, a
/// query, a fragment or a port; or the page's path and query, or its query
/// alone. The browser shows the address of an untitled page with its escapes
/// undone (`my%20file` as `my file`), so the title and the address are
/// compared with their escapes undone, and without case. A title that merely
/// mentions the site (`글 | blog.example.org`) is kept.
///
/// Short paths (under four characters) are not looked for: they would blank
/// ordinary titles, and they carry no signature.
fn names_address(title: &str, address: &str) -> bool {
    if address.is_empty() {
        return false;
    }
    let text = undone(title);
    if title.contains(address) || text.contains("://") {
        return true;
    }
    let Ok(url) = Url::parse(address) else {
        return false;
    };
    // `data:`, `blob:` and the like have no path to name: the whole address
    // is the title when the page has none.
    if !matches!(url.scheme(), "http" | "https") {
        return title.starts_with(&format!("{}:", url.scheme()));
    }
    if let Some(host) = url.host_str().filter(|h| !h.is_empty()) {
        let host = host.to_lowercase();
        if text.trim() == host {
            return true;
        }
        let mut from = 0;
        while let Some(at) = text[from..].find(&host) {
            let end = from + at + host.len();
            if matches!(text[end..].chars().next(), Some('/' | '?' | '#' | ':')) {
                return true;
            }
            from = end;
        }
    }
    let path = undone(&url[Position::BeforePath..Position::AfterQuery]);
    if path.chars().count() >= 4 && text.contains(&path) {
        return true;
    }
    url.query()
        .map(undone)
        .is_some_and(|query| query.chars().count() >= 4 && text.contains(&query))
}

/// `text` with its percent escapes undone (as UTF-8, lossily) and in lower
/// case.
fn undone(text: &str) -> String {
    percent_encoding::percent_decode_str(text)
        .decode_utf8_lossy()
        .to_lowercase()
}

/// A page's history as the browser reports it
/// (`Page.getNavigationHistory`): the index of the entry shown, and the ID and
/// address of each entry.
#[derive(Debug, Clone, PartialEq)]
pub struct History {
    pub current: usize,
    pub entries: Vec<(i64, String)>,
}

impl History {
    pub fn parse(answer: &Value) -> Option<History> {
        let current = usize::try_from(answer["currentIndex"].as_i64()?).ok()?;
        let entries = answer["entries"]
            .as_array()?
            .iter()
            .map(|e| Some((e["id"].as_i64()?, e["url"].as_str()?.to_owned())))
            .collect::<Option<Vec<_>>>()?;
        (current < entries.len()).then_some(History { current, entries })
    }

    /// The first entry that is not the blank page the server passed through
    /// while it prepared the page; a person never goes back before it. With
    /// nothing but blank pages, there is none (the history's length).
    fn floor(&self) -> usize {
        self.entries
            .iter()
            .position(|(_, url)| url != "about:blank")
            .unwrap_or(self.entries.len())
    }

    /// The entry a step back goes to, when going back is allowed.
    pub fn back(&self) -> Option<i64> {
        (self.current > self.floor()).then(|| self.entries[self.current - 1].0)
    }

    /// The entry a step forward goes to, when there is one.
    pub fn forward(&self) -> Option<i64> {
        self.entries.get(self.current + 1).map(|(id, _)| *id)
    }

    /// What the buttons and the host show for the page of the current entry.
    pub fn nav(&self) -> Nav {
        Nav {
            back: self.back().is_some(),
            forward: self.forward().is_some(),
            host: host_of(&self.entries[self.current].1),
        }
    }
}

/// The state of the back and forward buttons and the host of the page shown:
/// the `nav` message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nav {
    pub back: bool,
    pub forward: bool,
    pub host: Option<String>,
}

impl Nav {
    pub fn message(&self) -> String {
        json!({
            "type": "nav", "back": self.back, "forward": self.forward, "host": self.host,
        })
        .to_string()
    }
}

/// The `tabs` message: the run's pages in the order the worker listed them
/// (`pages`), each with the title and the host the browser reports in
/// `targets` (`Target.getTargets`'s `targetInfos`). A listed page the browser
/// does not report (just closed) is left out, and a page the worker did not
/// list never is a tab. `shown` is the page the screen shows, and `first` the
/// run's first page, which has no close control.
pub fn tabs_message(pages: &[String], targets: &Value, shown: &str, first: Option<&str>) -> String {
    let infos = targets.as_array().map(Vec::as_slice).unwrap_or_default();
    let tabs: Vec<Value> = pages
        .iter()
        .filter_map(|id| {
            let info = infos.iter().find(|t| t["targetId"].as_str() == Some(id))?;
            let address = info["url"].as_str().unwrap_or_default();
            Some(json!({
                "id": id,
                "title": tab_title(info["title"].as_str().unwrap_or_default(), address),
                "host": host_of(address),
                "shown": id == shown,
                "closable": first != Some(id.as_str()),
            }))
        })
        .collect();
    json!({ "type": "tabs", "tabs": tabs }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history(current: usize, urls: &[&str]) -> History {
        History {
            current,
            entries: urls
                .iter()
                .enumerate()
                .map(|(n, u)| (n as i64 + 10, (*u).to_owned()))
                .collect(),
        }
    }

    #[test]
    fn only_the_host_of_a_web_address_is_shown() {
        assert_eq!(
            host_of("https://files.example.com:8443/get/a.srt?sig=SECRET&exp=1#frag").as_deref(),
            Some("files.example.com")
        );
        assert_eq!(
            host_of("http://user:pass@blog.example.org/post/1").as_deref(),
            Some("blog.example.org")
        );
        assert_eq!(host_of("about:blank"), None);
        assert_eq!(host_of("data:text/html,<p>hi</p>"), None);
        assert_eq!(
            host_of("blob:https://blog.example.org/0b9d6a27-3e1c-4c5f-8f2a-1c2d3e4f5a6b"),
            None
        );
        assert_eq!(host_of("chrome-error://chromewebdata/"), None);
        assert_eq!(host_of("file:///etc/passwd"), None);
        assert_eq!(host_of("not an address"), None);
    }

    #[test]
    fn a_title_that_is_the_address_is_dropped_and_a_long_one_is_cut() {
        let address = "https://files.example.com/get/a.srt?sig=SECRET";
        // What the browser says for a page that has no title.
        assert_eq!(tab_title(address, address), "");
        assert_eq!(
            tab_title("files.example.com/get/a.srt?sig=SECRET", address),
            ""
        );
        assert_eq!(tab_title("  ", address), "");
        assert_eq!(
            tab_title("자막 올림 - 블로그", address),
            "자막 올림 - 블로그"
        );
        // A title that quotes the path and query is no better than the address.
        assert_eq!(tab_title("get /get/a.srt?sig=SECRET", address), "");
        let long = "가".repeat(60);
        let cut = tab_title(&long, address);
        assert_eq!(cut.chars().count(), TITLE_CHARS + 1);
        assert!(cut.ends_with('…'));
        // A page with no web address: a title that is its address is dropped.
        assert_eq!(
            tab_title("data:text/html,<p>hi</p>", "data:text/html,<p>hi</p>"),
            ""
        );
        assert_eq!(tab_title("about:blank", "about:blank"), "");
        assert_eq!(tab_title("제목", "about:blank"), "제목");
    }

    #[test]
    fn a_title_that_names_an_address_in_any_form_is_dropped() {
        // The browser shows an untitled page's address with its escapes
        // undone and its host in Unicode.
        let address = "https://files.example.com/get/my%20file.srt?sig=SECRET";
        assert_eq!(
            tab_title("files.example.com/get/my file.srt?sig=SECRET", address),
            ""
        );
        assert_eq!(
            tab_title("FILES.EXAMPLE.COM/get/my file.srt?sig=SECRET", address),
            ""
        );
        let korean = "https://blog.example.org/%EC%9E%90%EB%A7%89/1?sig=S1";
        assert_eq!(tab_title("blog.example.org/자막/1?sig=S1", korean), "");
        let idn = "https://xn--3e0b707e.example/%EC%9E%90%EB%A7%89/1?sig=S1";
        assert_eq!(tab_title("한국.example/자막/1?sig=S1", idn), "");
        // The host with a port, a query or a fragment after it.
        assert_eq!(tab_title("files.example.com:8443", address), "");
        assert_eq!(tab_title("files.example.com?sig=SECRET", address), "");
        // The query alone, and any other address a page quotes.
        assert_eq!(tab_title("받기 sig=SECRET", address), "");
        assert_eq!(tab_title("see https://other.example/x?t=1", address), "");
        // An untitled page at its root is named by its host; a title that
        // only mentions the site is the page's own.
        let root = "https://blog.example.org/";
        assert_eq!(tab_title("blog.example.org", root), "");
        assert_eq!(
            tab_title("글 | blog.example.org", root),
            "글 | blog.example.org"
        );
        assert_eq!(
            tab_title("글 | blog.example.org", korean),
            "글 | blog.example.org"
        );
    }

    #[test]
    fn back_stops_at_the_first_page_that_is_not_blank() {
        // The page the server prepared: a blank page, then the post.
        let h = history(1, &["about:blank", "https://blog.example.org/post/1"]);
        assert_eq!(h.back(), None);
        assert_eq!(h.forward(), None);
        assert_eq!(h.nav().host.as_deref(), Some("blog.example.org"));
        // A person went on to another page.
        let h = history(
            2,
            &[
                "about:blank",
                "https://blog.example.org/post/1",
                "https://blog.example.org/post/0?x=1",
            ],
        );
        assert_eq!(h.back(), Some(11));
        assert!(!h.nav().forward);
        // And back: forward is on, back is not.
        let h = history(
            1,
            &[
                "about:blank",
                "https://blog.example.org/post/1",
                "https://blog.example.org/post/0?x=1",
            ],
        );
        assert_eq!(h.back(), None);
        assert_eq!(h.forward(), Some(12));
        // On the blank page itself nothing is behind it.
        let h = history(0, &["about:blank", "https://blog.example.org/post/1"]);
        assert_eq!(h.back(), None);
        assert_eq!(h.nav().host, None);
        // Only blank pages: no back at all.
        let h = history(1, &["about:blank", "about:blank"]);
        assert_eq!(h.back(), None);
        // A page with no blank page before it can go back to its earlier ones.
        let h = history(1, &["https://a.example/", "https://b.example/"]);
        assert_eq!(h.back(), Some(10));
    }

    #[test]
    fn a_history_is_read_from_the_browsers_answer() {
        let answer = json!({
            "currentIndex": 1,
            "entries": [
                { "id": 3, "url": "about:blank", "title": "" },
                { "id": 7, "url": "https://blog.example.org/post/1", "title": "글" },
            ],
        });
        let h = History::parse(&answer).unwrap();
        assert_eq!(h.current, 1);
        assert_eq!(h.entries[1].0, 7);
        assert!(History::parse(&json!({ "currentIndex": 5, "entries": [] })).is_none());
        assert!(History::parse(&json!({})).is_none());
    }

    #[test]
    fn no_message_carries_a_path_a_query_or_a_fragment() {
        let address = "https://files.example.com/get/a.srt?sig=SECRET#frag";
        let h = history(
            1,
            &[
                "about:blank",
                address,
                "https://files.example.com/next/b?t=TOKEN",
            ],
        );
        let nav = h.nav().message();
        let targets = json!([
            { "targetId": "A", "type": "page", "title": address, "url": address },
            { "targetId": "B", "type": "page", "title": "글", "url": "about:blank" },
            { "targetId": "X", "type": "page", "title": "not listed", "url": "https://x.example/" },
        ]);
        let tabs = tabs_message(
            &["A".to_owned(), "B".to_owned(), "gone".to_owned()],
            &targets,
            "A",
            Some("A"),
        );
        for message in [&nav, &tabs] {
            for secret in ["/get", "sig", "SECRET", "frag", "TOKEN", "/next", "?"] {
                assert!(!message.contains(secret), "{secret} in {message}");
            }
        }
        let tabs: Value = serde_json::from_str(&tabs).unwrap();
        let tabs = tabs["tabs"].as_array().unwrap();
        // The page the worker did not list is no tab, and one the browser
        // does not report is not either.
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0]["host"], "files.example.com");
        assert_eq!(tabs[0]["title"], "");
        assert_eq!(tabs[0]["shown"], true);
        assert_eq!(tabs[0]["closable"], false);
        assert_eq!(tabs[1]["host"], Value::Null);
        assert_eq!(tabs[1]["title"], "글");
        assert_eq!(tabs[1]["closable"], true);
    }
}
