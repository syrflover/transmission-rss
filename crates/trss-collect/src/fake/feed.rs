//! A fake RSS server for channel feeds, with routes set by the test, request
//! counts and requests held at a [`Gate`].

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use axum::{
    body::Bytes,
    extract::{Path, RawQuery, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use tokio::task::JoinHandle;
use trss_core::loopback::Served;
use trss_transmission::fake::Gate;

enum Route {
    Xml(String),
    Status(u16),
    /// An empty feed padded with a comment to this many bytes, streamed
    /// without a length.
    Large(usize),
}

#[derive(Default)]
struct FeedState {
    routes: HashMap<String, Route>,
    requests: Vec<String>,
    holds: HashMap<String, Arc<Gate>>,
}

pub struct FeedServer {
    pub addr: SocketAddr,
    state: Arc<Mutex<FeedState>>,
    task: JoinHandle<()>,
}

impl FeedServer {
    pub async fn start() -> Self {
        let state = Arc::new(Mutex::new(FeedState::default()));
        let app = Router::new()
            .route("/{*path}", get(serve_feed))
            .with_state(state.clone());
        let Served { addr, task } = trss_core::loopback::serve(|listener| async move {
            axum::serve(listener, app).await.ok();
        })
        .await;
        FeedServer { addr, state, task }
    }

    /// Base URL of a feed path, without a query.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}/{}", self.addr, path)
    }

    pub fn set_xml(&self, path: &str, xml: &str) {
        self.state
            .lock()
            .unwrap()
            .routes
            .insert(path.to_owned(), Route::Xml(xml.to_owned()));
    }

    /// Serves a body of `len` bytes, which a feed read must refuse when it is
    /// over its cap.
    pub fn set_large(&self, path: &str, len: usize) {
        self.state
            .lock()
            .unwrap()
            .routes
            .insert(path.to_owned(), Route::Large(len));
    }

    pub fn set_status(&self, path: &str, status: u16) {
        self.state
            .lock()
            .unwrap()
            .routes
            .insert(path.to_owned(), Route::Status(status));
    }

    /// Every request received so far as `path?query`.
    pub fn requests(&self) -> Vec<String> {
        self.state.lock().unwrap().requests.clone()
    }

    pub fn hits(&self, path: &str) -> usize {
        let prefix = format!("{path}?");
        self.requests()
            .iter()
            .filter(|r| r.starts_with(&prefix) || r.as_str() == path)
            .count()
    }

    pub fn hold(&self, path: &str) -> Arc<Gate> {
        let gate = Gate::new();
        self.state
            .lock()
            .unwrap()
            .holds
            .insert(path.to_owned(), gate.clone());
        gate
    }
}

impl Drop for FeedServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve_feed(
    State(state): State<Arc<Mutex<FeedState>>>,
    Path(path): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    let gate = {
        let mut st = state.lock().unwrap();
        st.requests
            .push(format!("{path}?{}", query.unwrap_or_default()));
        st.holds.get(&path).cloned()
    };
    if let Some(gate) = gate {
        gate.pass().await;
    }

    let st = state.lock().unwrap();
    match st.routes.get(&path) {
        Some(Route::Xml(xml)) => {
            ([("content-type", "application/rss+xml")], xml.clone()).into_response()
        }
        Some(Route::Status(code)) => StatusCode::from_u16(*code).unwrap().into_response(),
        Some(Route::Large(len)) => {
            // An empty feed, so that a reader without a cap would see every
            // item as gone, followed by a comment as long as needed.
            let mut body = br#"<?xml version="1.0"?><rss version="2.0"><channel><title>t</title><link>http://x/</link><description>d</description></channel></rss><!--"#.to_vec();
            body.resize(len - 3, b' ');
            body.extend_from_slice(b"-->");
            let parts: Vec<Result<Bytes, std::convert::Infallible>> = body
                .chunks(64 * 1024)
                .map(|c| Ok(Bytes::copy_from_slice(c)))
                .collect();
            (
                [("content-type", "application/rss+xml")],
                axum::body::Body::from_stream(futures::stream::iter(parts)),
            )
                .into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
