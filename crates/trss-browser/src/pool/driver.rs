//! Preparing a run and following what its browser reports.
//!
//! One task per run reads the browser's events: the pages and frames that
//! appear (each has its set-up sent before it is let go), and the
//! downloads. When the DevTools connection is lost the run ends with it.

use std::{sync::Arc, time::Duration};

use serde_json::{json, Value};
use tokio::sync::broadcast::{self, error::RecvError};

use super::{BrowserError, PoolInner};
use crate::{
    blocklist::BLOCKED_URLS,
    cdp::{CdpError, Connection, Event, Sent},
    pool::run::RunInner,
};

fn cdp_error(err: CdpError) -> BrowserError {
    BrowserError::Cdp(err)
}

/// Starts the launcher's run for `entry`, connects to its DevTools and sets
/// the browser up: downloads are saved under the run's folder and named by the
/// browser, and every page that appears has its set-up sent first.
pub(super) async fn prepare(
    pool: &Arc<PoolInner>,
    entry: &Arc<RunInner>,
) -> Result<(), BrowserError> {
    let started = pool.launcher.start(&entry.run_id).await?;
    let _ = entry.container_downloads.set(started.downloads.clone());

    let conn = Connection::connect(&pool.launcher.cdp_url(&entry.run_id), pool.launcher.token())
        .await
        .map_err(cdp_error)?;
    let _ = entry.conn.set(conn.clone());
    if entry.is_ended() {
        // Ended (a shutdown) while it was being started.
        conn.close();
        return Err(BrowserError::RunEnded {
            run: entry.run_id.clone(),
        });
    }
    let events = conn.events();
    tokio::spawn(drive(pool.clone(), entry.clone(), conn.clone(), events));

    conn.command(
        None,
        "Browser.setDownloadBehavior",
        json!({
            "behavior": "allowAndName",
            "downloadPath": started.downloads,
            "eventsEnabled": true,
        }),
    )
    .await
    .map_err(cdp_error)?;
    // Pages and popups from now on, and the one the browser opened itself.
    conn.command(
        None,
        "Target.setAutoAttach",
        json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true }),
    )
    .await
    .map_err(cdp_error)?;

    entry.mark_ready();
    pool.changed.notify_waiters();
    Ok(())
}

/// Follows the run's events until the connection is over, then ends the run.
async fn drive(
    pool: Arc<PoolInner>,
    entry: Arc<RunInner>,
    conn: Connection,
    mut events: broadcast::Receiver<Event>,
) {
    loop {
        tokio::select! {
            _ = conn.closed() => break,
            event = events.recv() => match next_step(event) {
                Step::Event(event) => on_event(&pool, &entry, &conn, event),
                Step::End(why) => {
                    eprintln!("Browser: ending the run of job {}: {why}", entry.job_id);
                    break;
                }
            },
        }
    }
    // Without its control connection nothing can drive the run, and nothing
    // may be left running that no job holds.
    pool.end_entry(&entry).await;
}

/// What to do with what the event channel gave.
enum Step {
    Event(Event),
    End(String),
}

/// An event is handled. A reader that fell behind has lost events it cannot
/// get back: a page that waits for the debugger because its attach was
/// missed never goes on, and a download that ended is never announced. So
/// the run ends, and the job asks for a new one.
fn next_step(received: Result<Event, RecvError>) -> Step {
    match received {
        Ok(event) => Step::Event(event),
        Err(RecvError::Lagged(missed)) => Step::End(format!(
            "it fell behind by {missed} events of the browser and cannot tell what it missed"
        )),
        Err(RecvError::Closed) => Step::End("the browser's events are over".to_owned()),
    }
}

/// Asks the browser to cancel the download `guid`, as far as it can be
/// asked. The run no longer waits for the download either way.
pub(super) async fn cancel_download(entry: &RunInner, guid: &str) {
    if let Some(conn) = entry.conn.get() {
        let _ = conn
            .command_within(
                None,
                "Browser.cancelDownload",
                json!({ "guid": guid }),
                Duration::from_secs(5),
            )
            .await;
    }
}

fn on_event(pool: &Arc<PoolInner>, entry: &Arc<RunInner>, conn: &Connection, event: Event) {
    let text = |value: &Value| value.as_str().map(str::to_owned);
    let params = &event.params;
    match event.method.as_str() {
        "Target.attachedToTarget" => {
            let (Some(session), Some(target), Some(kind)) = (
                text(&params["sessionId"]),
                text(&params["targetInfo"]["targetId"]),
                text(&params["targetInfo"]["type"]),
            ) else {
                return;
            };
            entry.target_attached(&target, &session, &kind);
            let waiting = params["waitingForDebugger"].as_bool().unwrap_or(false);
            tokio::spawn(attach(
                entry.clone(),
                conn.clone(),
                session,
                target,
                kind,
                waiting,
            ));
        }
        "Target.detachedFromTarget" => {
            if let Some(session) = text(&params["sessionId"]) {
                entry.session_detached(&session);
            }
        }
        "Target.targetDestroyed" => {
            if let Some(target) = text(&params["targetId"]) {
                entry.target_destroyed(&target);
            }
        }
        // A navigation the browser turns into a download is answered first:
        // what the answer said is the download's (`download_began`). The
        // documents of the run's pages are told on (`page_documents`).
        "Network.responseReceived" => entry.response_received(params),
        "Browser.downloadWillBegin" => {
            if let Some(guid) = text(&params["guid"]) {
                entry.download_began(
                    &guid,
                    params["url"].as_str().unwrap_or_default(),
                    params["frameId"].as_str(),
                    params["suggestedFilename"].as_str().unwrap_or_default(),
                    pool.now(),
                );
            }
        }
        "Browser.downloadProgress" => {
            if let (Some(guid), Some(state)) = (text(&params["guid"]), text(&params["state"])) {
                let received = params["receivedBytes"].as_f64().unwrap_or(0.0) as u64;
                if entry.download_progress(&guid, &state, received, pool.now()) {
                    println!(
                        "Browser: a download of the run of job {} is over a size limit; canceling it",
                        entry.job_id
                    );
                    let entry = entry.clone();
                    tokio::spawn(async move { cancel_download(&entry, &guid).await });
                }
            }
        }
        _ => {}
    }
}

/// Sets up a target that is waiting for the debugger and lets it go.
async fn attach(
    entry: Arc<RunInner>,
    conn: Connection,
    session: String,
    target: String,
    kind: String,
    waiting: bool,
) {
    attach_with(
        &conn,
        &session,
        &kind,
        waiting,
        || entry.target_ready(&target, &session),
        |err| {
            eprintln!(
                "Browser: cannot set up a {kind} of the run of job {}: {err}",
                entry.job_id
            );
        },
    )
    .await;
}

/// Sends a target's set-up commands and, when it waits for the debugger, the
/// command that lets it go, in that order and without waiting for an answer in
/// between; then waits for the answers. A page that waits does not answer
/// `Network.enable` until it runs, so waiting for the set-up first would hold
/// it for the whole command time limit and then let it go unset up. The
/// browser handles a session's commands in the order it received them, so the
/// blocklist is in place before the target's first request, and a set-up
/// command that fails or never answers cannot hold the target back.
///
/// `ready` is called once the target is let go (and answered, for one that
/// waited). `failed` gets each set-up error that leaves the target without
/// its blocklist.
async fn attach_with(
    conn: &Connection,
    session: &str,
    kind: &str,
    waiting: bool,
    ready: impl FnOnce(),
    mut failed: impl FnMut(&CdpError),
) {
    let setup = matches!(kind, "page" | "iframe").then(|| configure(conn, session));
    // Whatever happened to the set-up, a target that waits must go on.
    if waiting {
        let run = conn.send(Some(session), "Runtime.runIfWaitingForDebugger", json!({}));
        if let Ok(run) = run {
            let _ = run.answer().await;
        }
    }
    ready();
    if let Some(setup) = setup {
        setup.finish(&mut failed).await;
    }
}

/// The set-up commands of a page or frame that were sent.
struct Configuring {
    enable: Result<Sent, CdpError>,
    blocked: Result<Sent, CdpError>,
    auto_attach: Result<Sent, CdpError>,
}

/// Sends the commands that block the ad and tracker addresses in a page or
/// frame, and attach to the frames and popups it makes. Nothing here waits for
/// an answer, and they are sent in this order.
fn configure(conn: &Connection, session: &str) -> Configuring {
    Configuring {
        // Nothing reads a response body, so none is kept: Chromium's
        // defaults keep up to 100 MB of them per page. A source that needs
        // to read bodies enables the Network domain with limits of its own.
        enable: conn.send(
            Some(session),
            "Network.enable",
            json!({ "maxTotalBufferSize": 0, "maxResourceBufferSize": 0 }),
        ),
        blocked: conn.send(
            Some(session),
            "Network.setBlockedURLs",
            json!({ "urls": BLOCKED_URLS }),
        ),
        // Frames of other sites are targets of their own: set them up too.
        // A refusal leaves this one set up.
        auto_attach: conn.send(
            Some(session),
            "Target.setAutoAttach",
            json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true }),
        ),
    }
}

impl Configuring {
    /// Waits for the answers together, so one that never comes holds the
    /// rest for one command time limit at most, and tells the first failure
    /// that leaves the target unblocked or its network unobserved: a run that
    /// ends halfway fails both, and once is enough to say so.
    async fn finish(self, failed: &mut impl FnMut(&CdpError)) {
        let (enable, blocked, _) = tokio::join!(
            answer(self.enable),
            answer(self.blocked),
            answer(self.auto_attach),
        );
        if let Some(err) = [enable, blocked].into_iter().find_map(Result::err) {
            failed(&err);
        }
    }
}

async fn answer(sent: Result<Sent, CdpError>) -> Result<Value, CdpError> {
    sent?.answer().await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(method: &str) -> Event {
        Event {
            method: method.to_owned(),
            params: json!({}),
            session_id: None,
            seq: 0,
        }
    }

    #[tokio::test]
    async fn a_reader_that_fell_behind_ends_the_run_instead_of_going_on_blind() {
        let (tx, mut rx) = broadcast::channel(2);
        for n in 0..10 {
            tx.send(event(&format!("E{n}"))).unwrap();
        }
        // The oldest events are gone: the reader is told so.
        let first = rx.recv().await;
        assert!(matches!(first, Err(RecvError::Lagged(8))), "{first:?}");
        assert!(matches!(
            next_step(first),
            Step::End(why) if why.contains("fell behind by 8")
        ));
        // What is left is read as usual.
        assert!(matches!(next_step(rx.recv().await), Step::Event(e) if e.method == "E8"));
        assert!(matches!(next_step(Err(RecvError::Closed)), Step::End(_)));
    }

    /// What the fake browser does with the commands of a session.
    #[derive(Clone, Copy, PartialEq)]
    enum Peer {
        /// Answers each command at once.
        Plain,
        /// Answers `Network.enable` only after `Runtime.runIfWaitingForDebugger`,
        /// as a page that waits for the debugger does.
        EnableAfterRun,
        /// Never answers `Network.enable`.
        EnableNever,
        /// Refuses `Network.setBlockedURLs`.
        RefuseBlocked,
    }

    /// A DevTools socket that records the methods it receives, in order.
    async fn fake_peer(peer: Peer) -> (Connection, Arc<std::sync::Mutex<Vec<String>>>) {
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = seen.clone();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut held = None;
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let request: Value = serde_json::from_str(text.as_str()).unwrap();
                let method = request["method"].as_str().unwrap().to_owned();
                // Every command is the target's own: one sent without its
                // session would set up (or let go) nothing.
                let mut what = method.clone();
                if !request["sessionId"].as_str().is_some_and(|s| !s.is_empty()) {
                    what.push_str(" without a session");
                }
                record.lock().unwrap().push(what);
                let mut answers = Vec::new();
                let answer = json!({ "id": request["id"], "result": {} });
                match method.as_str() {
                    "Network.enable" if peer == Peer::EnableNever => {}
                    "Network.enable" if peer == Peer::EnableAfterRun => held = Some(answer),
                    "Network.setBlockedURLs" if peer == Peer::RefuseBlocked => {
                        answers.push(json!({
                            "id": request["id"],
                            "error": { "code": -32000, "message": "refused" },
                        }));
                    }
                    "Runtime.runIfWaitingForDebugger" => {
                        answers.push(answer);
                        answers.extend(held.take());
                    }
                    _ => answers.push(answer),
                }
                for answer in answers {
                    socket
                        .send(Message::text(answer.to_string()))
                        .await
                        .unwrap();
                }
            }
        });
        let conn = Connection::connect(&url, "token").await.unwrap();
        (conn, seen)
    }

    const SETUP_THEN_RUN: [&str; 4] = [
        "Network.enable",
        "Network.setBlockedURLs",
        "Target.setAutoAttach",
        "Runtime.runIfWaitingForDebugger",
    ];

    #[tokio::test]
    async fn a_waiting_page_is_let_go_before_the_set_up_is_answered() {
        // Its `Network.enable` is answered only once the page runs: waiting
        // for that answer first would hold the page for the whole command
        // time limit (and then let it go unset up).
        let (conn, seen) = fake_peer(Peer::EnableAfterRun).await;
        let failures = std::sync::Mutex::new(Vec::new());
        let readied = std::sync::atomic::AtomicBool::new(false);
        tokio::time::timeout(
            Duration::from_secs(5),
            attach_with(
                &conn,
                "S",
                "page",
                true,
                || readied.store(true, std::sync::atomic::Ordering::SeqCst),
                |err| failures.lock().unwrap().push(err.to_string()),
            ),
        )
        .await
        .expect("the page was held for the set-up's answer");
        assert!(readied.load(std::sync::atomic::Ordering::SeqCst));
        // The wire order is the set-up, then the release.
        assert_eq!(*seen.lock().unwrap(), SETUP_THEN_RUN);
        assert!(failures.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_set_up_command_that_is_never_answered_does_not_hold_the_target() {
        let (conn, seen) = fake_peer(Peer::EnableNever).await;
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn({
            let conn = conn.clone();
            async move {
                attach_with(
                    &conn,
                    "S",
                    "iframe",
                    true,
                    move || {
                        let _ = ready_tx.send(());
                    },
                    |_| {},
                )
                .await;
            }
        });
        tokio::time::timeout(Duration::from_secs(2), ready_rx)
            .await
            .expect("the target was held by a set-up command nobody answers")
            .unwrap();
        assert_eq!(*seen.lock().unwrap(), SETUP_THEN_RUN);
        task.abort();
    }

    #[tokio::test]
    async fn a_refused_set_up_command_still_lets_the_target_go_and_is_reported() {
        let (conn, seen) = fake_peer(Peer::RefuseBlocked).await;
        let failures = std::sync::Mutex::new(Vec::new());
        let mut readied = false;
        attach_with(
            &conn,
            "S",
            "page",
            true,
            || readied = true,
            |err| failures.lock().unwrap().push(err.to_string()),
        )
        .await;
        assert!(readied);
        // The refusal does not stop the commands after it.
        assert_eq!(*seen.lock().unwrap(), SETUP_THEN_RUN);
        let failures = failures.into_inner().unwrap();
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(
            failures[0].contains("Network.setBlockedURLs"),
            "{failures:?}"
        );
    }

    #[tokio::test]
    async fn a_target_that_is_not_a_page_is_only_let_go_and_one_that_runs_is_only_set_up() {
        let (conn, seen) = fake_peer(Peer::Plain).await;
        attach_with(&conn, "W", "service_worker", true, || {}, |_| {}).await;
        assert_eq!(*seen.lock().unwrap(), ["Runtime.runIfWaitingForDebugger"]);

        let (conn, seen) = fake_peer(Peer::Plain).await;
        attach_with(&conn, "F", "iframe", false, || {}, |_| {}).await;
        assert_eq!(
            *seen.lock().unwrap(),
            [
                "Network.enable",
                "Network.setBlockedURLs",
                "Target.setAutoAttach"
            ]
        );
    }
}
