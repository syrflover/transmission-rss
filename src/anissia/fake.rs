//! A stand-in for Anissia in tests: a local HTTP server answering the two
//! requests the client sends, with knobs for `429`s, failures, padding and
//! answers that are not the API's. No test reaches the real Anissia.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use serde_json::{json, Value};

use super::AnissiaConfig;

#[derive(Default)]
pub struct FakeState {
    /// The entries of each week's schedule, as Anissia lists them.
    pub schedules: HashMap<u8, Vec<Value>>,
    /// The captions of each anime.
    pub captions: HashMap<i64, Vec<Value>>,
    /// The next this many requests answer `429` with this `Retry-After`.
    pub rate_limited: u32,
    /// `None` sends no `Retry-After`.
    pub retry_after: Option<u64>,
    /// The next this many requests answer `500`.
    pub failing: u32,
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
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let fake = Fake {
            state: Arc::new(Mutex::new(FakeState::default())),
            origin,
        };
        let app = Router::new()
            .route("/anime/schedule/{week}", get(schedule))
            .route("/anime/caption/animeNo/{no}", get(captions))
            .with_state(fake.clone());
        tokio::spawn(async move {
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

    /// Makes `captions` the captions of anime `no`.
    pub fn set_captions(&self, no: i64, captions: Vec<Value>) {
        self.state.lock().unwrap().captions.insert(no, captions);
    }

    /// A caption as Anissia lists it.
    pub fn caption(&self, episode: &str, updated: &str, creator: &str) -> Value {
        json!({ "episode": episode, "updDt": updated, "website": "https://blog.test/1", "name": creator })
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
    state.requests.push((Instant::now(), path));
    if state.rate_limited > 0 {
        state.rate_limited -= 1;
        let mut response = (StatusCode::TOO_MANY_REQUESTS, "{}").into_response();
        if let Some(seconds) = state.retry_after {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, seconds.into());
        }
        return response;
    }
    if state.failing > 0 {
        state.failing -= 1;
        return (StatusCode::INTERNAL_SERVER_ERROR, "{}").into_response();
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
    let mut value = json!({ "code": "ok", "data": data });
    if state.padding == 0 {
        return (StatusCode::OK, axum::Json(value)).into_response();
    }
    value["padding"] = json!("x".repeat(state.padding));
    let text = value.to_string();
    if state.chunked {
        let chunks: Vec<Result<Bytes, std::io::Error>> = text
            .into_bytes()
            .chunks(64 * 1024)
            .map(|c| Ok(Bytes::copy_from_slice(c)))
            .collect();
        let body = axum::body::Body::from_stream(futures::stream::iter(chunks));
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
