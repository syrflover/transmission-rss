//! A stand-in for AniList in tests: a local HTTP server answering the GraphQL
//! queries the client sends and serving images, with knobs for `429`s,
//! failures and held responses. No test reaches the real AniList.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Instant,
};

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde_json::{json, Value};

use super::AnilistConfig;

/// The most airing schedule nodes the fake answers at once, as AniList does.
const SCHEDULE_PAGE: usize = 25;

#[derive(Default)]
pub struct FakeState {
    /// Pages of entries (as AniList's JSON) by search text.
    pub searches: HashMap<String, Vec<Vec<Value>>>,
    /// Entries by ID.
    pub media: HashMap<i64, Value>,
    /// Image bytes by name (`/img/<name>`).
    pub images: HashMap<String, Vec<u8>>,
    /// The next this many API requests answer `429` with this `Retry-After`.
    pub rate_limited: u32,
    pub retry_after: u64,
    /// The next this many API requests answer `500`.
    pub failing: u32,
    /// API answers carry this many bytes of padding (an unknown field).
    pub padding: usize,
    /// Whether the padded answers leave out `Content-Length` (sent in chunks).
    pub chunked: bool,
    /// When each API request arrived, with its variables.
    pub requests: Vec<(Instant, Value)>,
    /// Image requests, by name.
    pub image_requests: Vec<String>,
}

#[derive(Clone)]
pub struct Fake {
    pub state: Arc<Mutex<FakeState>>,
    /// Images whose answers wait until released.
    held: Arc<Mutex<HashSet<String>>>,
    pub origin: String,
}

impl Fake {
    pub async fn start() -> Fake {
        let bound = trss_core::loopback::bind().await;
        let origin = format!("http://{}", bound.addr);
        let fake = Fake {
            state: Arc::new(Mutex::new(FakeState::default())),
            held: Arc::new(Mutex::new(HashSet::new())),
            origin,
        };
        let app = Router::new()
            .route("/graphql", post(graphql))
            .route("/img/{name}", get(image))
            .with_state(fake.clone());
        bound.spawn(|listener| async move {
            axum::serve(listener, app).await.unwrap();
        });
        fake
    }

    pub fn config(&self) -> AnilistConfig {
        AnilistConfig {
            api_url: format!("{}/graphql", self.origin).parse().unwrap(),
            image_origins: vec![self.origin.clone()],
        }
    }

    /// An entry as AniList answers it, its cover at `/img/<id>.jpg`.
    pub fn entry(&self, id: i64, romaji: &str, synonyms: &[&str]) -> Value {
        json!({
            "id": id,
            "title": { "romaji": romaji, "english": null, "native": null },
            "synonyms": synonyms,
            "format": "TV",
            "seasonYear": 2022,
            "coverImage": {
                "extraLarge": format!("{}/img/{id}.jpg", self.origin),
                "large": format!("{}/img/{id}.jpg", self.origin),
                "medium": format!("{}/img/{id}.jpg", self.origin),
            }
        })
    }

    /// Adds `entries` as the one page of the search for `text`, each also by
    /// ID, each with `image` as its cover.
    pub fn add_search(&self, text: &str, entries: Vec<Value>, image: &[u8]) {
        let mut state = self.state.lock().unwrap();
        for e in &entries {
            let id = e["id"].as_i64().unwrap();
            state.media.insert(id, e.clone());
            state.images.insert(format!("{id}.jpg"), image.to_vec());
        }
        state.searches.insert(text.to_owned(), vec![entries]);
    }

    /// The answer for image `name` waits until [`Fake::release`].
    pub fn hold(&self, name: &str) {
        self.held.lock().unwrap().insert(name.to_owned());
    }

    pub fn release(&self, name: &str) {
        self.held.lock().unwrap().remove(name);
    }

    /// Whether image `name` was asked for.
    pub fn image_asked(&self, name: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .image_requests
            .iter()
            .any(|n| n == name)
    }

    pub fn api_requests(&self) -> Vec<(Instant, Value)> {
        self.state.lock().unwrap().requests.clone()
    }
}

async fn graphql(State(fake): State<Fake>, body: Bytes) -> Response {
    let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let variables = body["variables"].clone();
    let mut state = fake.state.lock().unwrap();
    state.requests.push((Instant::now(), variables.clone()));
    if state.rate_limited > 0 {
        state.rate_limited -= 1;
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, state.retry_after.to_string())],
            "{}",
        )
            .into_response();
    }
    if state.failing > 0 {
        state.failing -= 1;
        return (StatusCode::INTERNAL_SERVER_ERROR, "{}").into_response();
    }
    let padding = state.padding;
    let chunked = state.chunked;
    let answer = |status: StatusCode, mut value: Value| -> Response {
        if padding == 0 {
            return (status, axum::Json(value)).into_response();
        }
        value["extensions"] = json!({ "padding": "x".repeat(padding) });
        let text = value.to_string();
        if chunked {
            let chunks: Vec<Result<Bytes, std::io::Error>> = text
                .into_bytes()
                .chunks(64 * 1024)
                .map(|c| Ok(Bytes::copy_from_slice(c)))
                .collect();
            let body = axum::body::Body::from_stream(futures::stream::iter(chunks));
            return (status, [(header::CONTENT_TYPE, "application/json")], body).into_response();
        }
        (status, [(header::CONTENT_TYPE, "application/json")], text).into_response()
    };
    let query = body["query"].as_str().unwrap_or("");
    if query.contains("Page(") {
        let text = variables["search"].as_str().unwrap_or("");
        let page = variables["page"].as_u64().unwrap_or(1) as usize;
        let pages = state.searches.get(text).cloned().unwrap_or_default();
        let media = pages.get(page - 1).cloned().unwrap_or_default();
        return answer(
            StatusCode::OK,
            json!({ "data": { "Page": {
                "pageInfo": { "hasNextPage": page < pages.len() },
                "media": media,
            } } }),
        );
    }
    let id = variables["id"].as_i64().unwrap_or(0);
    match state.media.get(&id) {
        Some(entry) => {
            let mut entry = entry.clone();
            // Like AniList, an entry's schedule comes at most 25 nodes to a
            // page, the `page` variable choosing which, with `hasNextPage`.
            if let Some(nodes) = entry["airingSchedule"]["nodes"].as_array().cloned() {
                let page = variables["page"].as_u64().unwrap_or(1).max(1) as usize;
                let from = (page - 1) * SCHEDULE_PAGE;
                let nodes: Vec<Value> = nodes.into_iter().skip(from).collect();
                entry["airingSchedule"] = json!({
                    "pageInfo": { "hasNextPage": nodes.len() > SCHEDULE_PAGE },
                    "nodes": nodes.into_iter().take(SCHEDULE_PAGE).collect::<Vec<_>>(),
                });
            }
            answer(StatusCode::OK, json!({ "data": { "Media": entry } }))
        }
        None => answer(
            StatusCode::NOT_FOUND,
            json!({ "errors": [{ "message": "Not Found.", "status": 404 }], "data": { "Media": null } }),
        ),
    }
}

async fn image(State(fake): State<Fake>, Path(name): Path<String>) -> Response {
    let bytes = {
        let mut state = fake.state.lock().unwrap();
        state.image_requests.push(name.clone());
        state.images.get(&name).cloned()
    };
    while fake.held.lock().unwrap().contains(&name) {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    match bytes {
        Some(bytes) => ([(header::CONTENT_TYPE, "image/jpeg")], bytes).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
