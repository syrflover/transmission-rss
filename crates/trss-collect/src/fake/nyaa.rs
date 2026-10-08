//! A fake nyaa search RSS, as the past search reads it.

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    extract::State,
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use tokio::{net::TcpListener, task::JoinHandle};

/// What a fake nyaa knows.
#[derive(Default)]
struct NyaaState {
    /// `(title, number)`, newest first. The number (a hash of the title) makes
    /// the torrent's hash.
    releases: Vec<(String, u32)>,
    /// The `q` of every search received, with when it arrived.
    requests: Vec<(String, std::time::Instant)>,
    /// Answer every search with this status and `Retry-After`, if set.
    refuse: Option<(u16, Option<u64>)>,
}

/// A tracker whose search RSS (`/?page=rss&q=…`) answers the way nyaa's does:
/// every word of `q` must be a word of the title (`(a|b)` is either word),
/// punctuation separates words, and at most [`FakeNyaa::PAGE`] results come
/// back whatever `p` says. The pages' HTML is not served at all.
pub struct FakeNyaa {
    pub addr: SocketAddr,
    state: Arc<Mutex<NyaaState>>,
    task: JoinHandle<()>,
}

impl FakeNyaa {
    /// How many results one search returns at most.
    pub const PAGE: usize = 75;

    pub async fn start() -> Self {
        let state = Arc::new(Mutex::new(NyaaState::default()));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind fake nyaa");
        let addr = listener.local_addr().unwrap();
        let app = Router::new()
            .route("/", get(serve_nyaa))
            .with_state((state.clone(), addr))
            .layer(axum::middleware::from_fn(
                |request: axum::extract::Request, next: axum::middleware::Next| async move {
                    // Only the RSS is served; a request for a page is an error.
                    let wants_rss = request
                        .uri()
                        .query()
                        .is_some_and(|q| q.split('&').any(|p| p == "page=rss"));
                    if wants_rss {
                        next.run(request).await
                    } else {
                        StatusCode::FORBIDDEN.into_response()
                    }
                },
            ));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        FakeNyaa { addr, state, task }
    }

    /// A channel URL on this tracker (`page=rss` and the filters of a channel).
    pub fn url(&self, secret: &str) -> String {
        format!("http://{}/?page=rss&c=1_2&f=0&token={secret}", self.addr)
    }

    /// Sets the tracker's releases, newest first.
    pub fn set_releases(&self, titles: &[String]) {
        let releases = titles
            .iter()
            .map(|t| (t.clone(), crc32fast::hash(t.as_bytes())))
            .collect();
        self.state.lock().unwrap().releases = releases;
    }

    /// Answers every search with `status` (and `Retry-After`); `None` ends it.
    pub fn refuse(&self, refusal: Option<(u16, Option<u64>)>) {
        self.state.lock().unwrap().refuse = refusal;
    }

    /// The `q` of each search received so far.
    pub fn queries(&self) -> Vec<String> {
        self.state
            .lock()
            .unwrap()
            .requests
            .iter()
            .map(|(q, _)| q.clone())
            .collect()
    }

    /// The time between each search and the one before it.
    pub fn gaps(&self) -> Vec<Duration> {
        let state = self.state.lock().unwrap();
        state
            .requests
            .windows(2)
            .map(|pair| pair[1].1.duration_since(pair[0].1))
            .collect()
    }

    /// The torrent hash of the release titled `title`, the same whatever else
    /// the tracker lists.
    pub fn hash_for(title: &str) -> String {
        format!("dddd{:036}", crc32fast::hash(title.as_bytes()))
    }
}

impl Drop for FakeNyaa {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn nyaa_matches(query: &str, title: &str) -> bool {
    let title_words = words(title);
    // Terms are split at spaces outside parentheses.
    let mut terms = Vec::new();
    let mut depth = 0;
    let mut current = String::new();
    for c in query.chars() {
        match c {
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth -= 1;
                current.push(c);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    terms.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        terms.push(current);
    }
    terms.iter().all(|term| {
        if term.starts_with('(') && term.ends_with(')') {
            let alternatives: Vec<String> =
                term[1..term.len() - 1].split('|').flat_map(words).collect();
            return alternatives.iter().any(|a| title_words.contains(a));
        }
        // A term made of punctuation only (`-`) asks for nothing.
        words(term).iter().all(|a| title_words.contains(a))
    })
}

async fn serve_nyaa(
    State((state, addr)): State<(Arc<Mutex<NyaaState>>, SocketAddr)>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> Response {
    let query = params.get("q").cloned().unwrap_or_default();
    let mut st = state.lock().unwrap();
    st.requests.push((query.clone(), std::time::Instant::now()));
    if let Some((status, retry_after)) = st.refuse {
        let mut response = StatusCode::from_u16(status).unwrap().into_response();
        if let Some(seconds) = retry_after {
            response.headers_mut().insert(
                "retry-after",
                HeaderValue::from_str(&seconds.to_string()).unwrap(),
            );
        }
        return response;
    }
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="utf-8"?><rss xmlns:atom="http://www.w3.org/2005/Atom" xmlns:nyaa="https://nyaa.si/xmlns/nyaa" version="2.0"><channel><title>Nyaa - Search</title><description>RSS Feed</description><link>https://nyaa.si/</link>"#,
    );
    for (title, number) in st
        .releases
        .iter()
        .filter(|(title, _)| nyaa_matches(&query, title))
        .take(FakeNyaa::PAGE)
    {
        let dn: String = url::form_urlencoded::byte_serialize(title.as_bytes()).collect();
        let link = format!(
            "http://{addr}/download/{number}.torrent?xt=urn:btih:dddd{number:036}&amp;dn={dn}"
        );
        let escaped = title.replace('&', "&amp;").replace('<', "&lt;");
        xml.push_str(&format!(
            "<item><title>{escaped}</title><link>{link}</link><guid isPermaLink=\"true\">http://{addr}/view/{number}</guid><nyaa:seeders>1</nyaa:seeders></item>"
        ));
    }
    xml.push_str("</channel></rss>");
    ([("content-type", "application/rss+xml")], xml).into_response()
}
