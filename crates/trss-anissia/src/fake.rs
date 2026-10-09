//! A stand-in for Anissia in tests: a local HTTP server answering the four
//! requests the client sends, with knobs for `429`s, failures, padding and
//! answers that are not the API's. No test reaches the real Anissia.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use serde_json::{json, Value};
use trss_core::fake_http::{chunks, padded, refuse};

use super::AnissiaConfig;

#[derive(Default)]
pub struct FakeState {
    /// The entries of each week's schedule, as Anissia lists them.
    pub schedules: HashMap<u8, Vec<Value>>,
    /// The captions of each anime.
    pub captions: HashMap<i64, Vec<Value>>,
    /// Every anime of the full list, newest first, as Anissia lists them.
    pub catalogue: Vec<Value>,
    /// How many anime a page of the full list has (Anissia's is 30).
    pub page_size: usize,
    /// The lines of the recent captions list, newest first, as Anissia lists
    /// them (`animeNo` and `subject` among the fields).
    pub recent: Vec<Value>,
    /// How many lines a page of the recent list has (Anissia's is 20).
    pub recent_page_size: usize,
    /// The next this many requests answer `429` with this `Retry-After`.
    pub rate_limited: u32,
    /// `None` sends no `Retry-After`.
    pub retry_after: Option<u64>,
    /// The next this many requests answer `500`.
    pub failing: u32,
    /// Requests for these paths (`/anime/schedule/5`) always answer `500`.
    pub failing_paths: std::collections::HashSet<String>,
    /// Answers carry this many bytes of padding (an unknown field).
    pub padding: usize,
    /// Whether padded answers leave out `Content-Length` (sent in chunks).
    pub chunked: bool,
    /// When set, every answer is this text instead.
    pub raw: Option<String>,
    /// When each request arrived, with its path.
    pub requests: Vec<(Instant, String)>,
}

#[derive(Clone)]
pub struct Fake {
    pub state: Arc<Mutex<FakeState>>,
    pub origin: String,
}

impl Fake {
    pub async fn start() -> Fake {
        let bound = trss_core::loopback::bind().await;
        let origin = format!("http://{}", bound.addr);
        let fake = Fake {
            state: Arc::new(Mutex::new(FakeState {
                page_size: 30,
                recent_page_size: 20,
                ..FakeState::default()
            })),
            origin,
        };
        let app = Router::new()
            .route("/anime/schedule/{week}", get(schedule))
            .route("/anime/caption/animeNo/{no}", get(captions))
            .route("/anime/list/{page}", get(list))
            .route("/anime/caption/recent/{page}", get(recent))
            .with_state(fake.clone());
        bound.spawn(|listener| async move {
            axum::serve(listener, app).await.unwrap();
        });
        fake
    }

    pub fn config(&self) -> AnissiaConfig {
        AnissiaConfig {
            base_url: self.origin.parse().unwrap(),
        }
    }

    /// An entry as Anissia lists it (the live shape, `week` a string).
    pub fn entry(&self, week: u8, no: i64, time: &str, subject: &str, original: &str) -> Value {
        json!({
            "week": week.to_string(), "animeNo": no, "status": "ON", "time": time,
            "subject": subject, "originalSubject": original, "genres": "판타지,액션",
            "captionCount": 1, "startDate": "2026-10-07", "endDate": "",
            "website": "https://example.test/anime", "x": "https://x.com/example",
        })
    }

    /// Makes `entries` the schedule of `week`.
    pub fn set_week(&self, week: u8, entries: Vec<Value>) {
        self.state.lock().unwrap().schedules.insert(week, entries);
    }

    /// A finished anime as the full list gives it (`status` is `END`, with an
    /// end date), which no week's schedule lists any more.
    pub fn finished(&self, week: u8, no: i64, subject: &str, original: &str) -> Value {
        json!({
            "animeNo": no, "status": "END", "week": week.to_string(), "time": "23:30",
            "subject": subject, "originalSubject": original, "captionCount": 1,
            "genres": "판타지", "startDate": "2021-04-04", "endDate": "2021-06-27",
            "website": "https://example.test/finished", "x": "", "note": "", "agendaNo": 0,
            "captions": [],
        })
    }

    /// Makes `entries` the full list, which searches filter and page.
    pub fn set_catalogue(&self, entries: Vec<Value>) {
        self.state.lock().unwrap().catalogue = entries;
    }

    /// Makes `captions` the captions of anime `no`.
    pub fn set_captions(&self, no: i64, captions: Vec<Value>) {
        self.state.lock().unwrap().captions.insert(no, captions);
    }

    /// A caption as Anissia lists it.
    pub fn caption(&self, episode: &str, updated: &str, creator: &str) -> Value {
        json!({ "episode": episode, "updDt": updated, "website": "https://blog.test/1", "name": creator })
    }

    /// A line of the recent list as Anissia gives it (observed 2026-10-02).
    pub fn recent_line(
        &self,
        anime_no: i64,
        episode: &str,
        updated: &str,
        website: &str,
        creator: &str,
    ) -> Value {
        json!({
            "animeNo": anime_no, "subject": format!("작품 {anime_no}"), "episode": episode,
            "updDt": updated, "website": website, "name": creator,
        })
    }

    /// Makes `lines` the recent captions list, which pages as Anissia's does.
    pub fn set_recent(&self, lines: Vec<Value>) {
        self.state.lock().unwrap().recent = lines;
    }

    pub fn requests(&self) -> Vec<(Instant, String)> {
        self.state.lock().unwrap().requests.clone()
    }

    /// How many requests arrived for paths that contain `part`.
    pub fn count(&self, part: &str) -> usize {
        self.requests()
            .iter()
            .filter(|(_, p)| p.contains(part))
            .count()
    }
}

/// The answer to one request, with the knobs applied.
fn answer(fake: &Fake, path: String, data: Option<Value>) -> Response {
    let mut state = fake.state.lock().unwrap();
    let fails = state.failing_paths.contains(&path);
    state.requests.push((Instant::now(), path));
    if fails {
        return (StatusCode::INTERNAL_SERVER_ERROR, "{}").into_response();
    }
    let faults = &mut *state;
    if let Some(refusal) = refuse(
        &mut faults.rate_limited,
        faults.retry_after,
        &mut faults.failing,
    ) {
        let mut response = (StatusCode::from_u16(refusal.status).unwrap(), "{}").into_response();
        if let Some(seconds) = refusal.retry_after {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, seconds.into());
        }
        return response;
    }
    if let Some(raw) = state.raw.clone() {
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            raw,
        )
            .into_response();
    }
    let Some(data) = data else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let value = json!({ "code": "ok", "data": data });
    if state.padding == 0 {
        return (StatusCode::OK, axum::Json(value)).into_response();
    }
    let text = padded(value, &["padding"], state.padding);
    if state.chunked {
        let pieces: Vec<Result<Bytes, std::io::Error>> = chunks(text.as_bytes())
            .map(|c| Ok(Bytes::copy_from_slice(c)))
            .collect();
        let body = axum::body::Body::from_stream(futures::stream::iter(pieces));
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response();
    }
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        text,
    )
        .into_response()
}

async fn schedule(State(fake): State<Fake>, Path(week): Path<String>) -> Response {
    let data = week.parse::<u8>().ok().filter(|w| *w <= 8).map(|w| {
        Value::Array(
            fake.state
                .lock()
                .unwrap()
                .schedules
                .get(&w)
                .cloned()
                .unwrap_or_default(),
        )
    });
    answer(&fake, format!("/anime/schedule/{week}"), data)
}

async fn captions(State(fake): State<Fake>, Path(no): Path<String>) -> Response {
    // Unknown anime have no captions: Anissia answers an empty list.
    let data = Value::Array(
        no.parse::<i64>()
            .ok()
            .and_then(|n| fake.state.lock().unwrap().captions.get(&n).cloned())
            .unwrap_or_default(),
    );
    answer(&fake, format!("/anime/caption/animeNo/{no}"), Some(data))
}

#[derive(serde::Deserialize)]
struct ListQuery {
    q: Option<String>,
}

/// `GET /anime/list/<page>?q=`: the anime of the full list whose title or
/// original title contains every word of `q`, thirty (`page_size`) to a page,
/// in the paged shape Anissia answers with.
async fn list(
    State(fake): State<Fake>,
    Path(page): Path<String>,
    Query(query): Query<ListQuery>,
) -> Response {
    let q = query.q.unwrap_or_default();
    let data = page.parse::<usize>().ok().map(|index| {
        let state = fake.state.lock().unwrap();
        let words: Vec<String> = q.split_whitespace().map(str::to_lowercase).collect();
        let matching: Vec<&Value> = state
            .catalogue
            .iter()
            .filter(|entry| {
                let text = format!(
                    "{} {}",
                    entry["subject"].as_str().unwrap_or(""),
                    entry["originalSubject"].as_str().unwrap_or("")
                )
                .to_lowercase();
                words.iter().all(|w| text.contains(w))
            })
            .collect();
        let size = state.page_size.max(1);
        let total_pages = matching.len().div_ceil(size);
        let content: Vec<Value> = matching
            .iter()
            .skip(index * size)
            .take(size)
            .map(|v| (*v).clone())
            .collect();
        json!({
            "content": content, "empty": content.is_empty(), "first": index == 0,
            "last": index + 1 >= total_pages, "number": index, "size": size,
            "numberOfElements": content.len(), "totalElements": matching.len(),
            "totalPages": total_pages,
        })
    });
    answer(&fake, format!("/anime/list/{page}?q={q}"), data)
}

/// `GET /anime/caption/recent/<page>`: the recent captions, `recent_page_size`
/// to a page counting from 0, in Spring's page shape as observed on
/// 2026-10-02. A page past the end is empty, with `last` true and still `ok`.
async fn recent(State(fake): State<Fake>, Path(page): Path<String>) -> Response {
    let data = page.parse::<usize>().ok().map(|index| {
        let state = fake.state.lock().unwrap();
        let size = state.recent_page_size.max(1);
        let total = state.recent.len();
        let total_pages = total.div_ceil(size);
        let content: Vec<Value> = state
            .recent
            .iter()
            .skip(index * size)
            .take(size)
            .cloned()
            .collect();
        json!({
            "content": content, "empty": content.is_empty(), "first": index == 0,
            "last": index + 1 >= total_pages, "number": index, "size": size,
            "numberOfElements": content.len(), "totalElements": total,
            "totalPages": total_pages,
            "pageable": {
                "offset": index * size, "pageNumber": index, "pageSize": size,
                "paged": true, "unpaged": false,
                "sort": {"empty": true, "sorted": false, "unsorted": true},
            },
            "sort": {"empty": true, "sorted": false, "unsorted": true},
        })
    });
    answer(&fake, format!("/anime/caption/recent/{page}"), data)
}
