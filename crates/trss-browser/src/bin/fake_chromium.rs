//! A stand-in for Chromium, for the launcher's tests.
//!
//! It takes the arguments the launcher gives Chromium and serves what the
//! launcher uses of DevTools on `--remote-debugging-port`: `/json/version`
//! and an echo WebSocket at the path it names. It writes `args.txt` (one
//! argument a line) and `display.txt` into `--user-data-dir`, and a `Cookies`
//! file as a site would leave one.
//!
//! Test switches, given as arguments like the rest:
//!
//! - `--fake-exit-after-ms=N`: exits by itself after N milliseconds.
//! - `--fake-exit-at-start`: exits with status 3 before it serves anything.
//! - `--fake-ignore-term`: ignores SIGTERM, so that only SIGKILL stops it.
//! - `--fake-never-ready`: runs but never serves.
//! - `--fake-ready-after-ms=N`: starts serving after N milliseconds.

use std::{path::PathBuf, time::Duration};

use axum::{
    extract::ws::{Message, WebSocketUpgrade},
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use tokio::signal::unix::{signal, SignalKind};

const SOCKET_PATH: &str = "/devtools/browser/fake";

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().find_map(|a| a.strip_prefix(name))
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--fake-exit-at-start") {
        std::process::exit(3);
    }
    let port: u16 = flag(&args, "--remote-debugging-port=")
        .and_then(|p| p.parse().ok())
        .expect("--remote-debugging-port=N");
    let profile = PathBuf::from(flag(&args, "--user-data-dir=").expect("--user-data-dir=DIR"));

    std::fs::create_dir_all(profile.join("Default")).expect("profile folder");
    std::fs::write(profile.join("args.txt"), args.join("\n")).expect("args.txt");
    std::fs::write(
        profile.join("display.txt"),
        std::env::var("DISPLAY").unwrap_or_default(),
    )
    .expect("display.txt");
    std::fs::write(profile.join("Default/Cookies"), "a cookie").expect("Cookies");

    if args.iter().any(|a| a == "--fake-ignore-term") {
        let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
        tokio::spawn(async move { while term.recv().await.is_some() {} });
    }
    if let Some(ms) = flag(&args, "--fake-exit-after-ms=").and_then(|m| m.parse().ok()) {
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            std::process::exit(0);
        });
    }
    if args.iter().any(|a| a == "--fake-never-ready") {
        std::future::pending::<()>().await;
    }
    if let Some(ms) = flag(&args, "--fake-ready-after-ms=").and_then(|m| m.parse().ok()) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    let app = Router::new()
        .route(
            "/json/version",
            get(move || async move {
                Json(serde_json::json!({
                    "Browser": "Fake/1",
                    "webSocketDebuggerUrl": format!("ws://127.0.0.1:{port}{SOCKET_PATH}"),
                }))
            }),
        )
        .route(SOCKET_PATH, get(echo));
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("the debugging port");
    axum::serve(listener, app).await.expect("serve");
}

async fn echo(upgrade: WebSocketUpgrade) -> impl IntoResponse {
    upgrade.on_upgrade(|mut socket| async move {
        while let Some(Ok(message)) = socket.recv().await {
            match message {
                Message::Text(_) | Message::Binary(_) => {
                    if socket.send(message).await.is_err() {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    })
}
