//! Preparing a run and following what its browser reports.
//!
//! One task per run reads the browser's events: the pages and frames that
//! appear (each is set up before it loads anything, then let go), and the
//! downloads. When the DevTools connection is lost the run ends with it.

use std::{sync::Arc, time::Duration};

use serde_json::{json, Value};
use tokio::sync::broadcast::{self, error::RecvError};

use super::{BrowserError, PoolInner};
use crate::{
    blocklist::BLOCKED_URLS,
    cdp::{CdpError, Connection, Event},
    pool::run::RunInner,
};

fn cdp_error(err: CdpError) -> BrowserError {
    BrowserError::Cdp(err)
}

/// Starts the launcher's run for `entry`, connects to its DevTools and sets
/// the browser up: downloads are saved under the run's folder and named by the
/// browser, and every page that appears is set up first.
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
        "Browser.downloadWillBegin" => {
            if let Some(guid) = text(&params["guid"]) {
                entry.download_began(
                    &guid,
                    params["url"].as_str().unwrap_or_default(),
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
    if matches!(kind.as_str(), "page" | "iframe") {
        if let Err(err) = configure(&conn, &session).await {
            eprintln!(
                "Browser: cannot set up a {kind} of the run of job {}: {err}",
                entry.job_id
            );
        }
    }
    // Whatever happened, a target that waits must go on.
    if waiting {
        let _ = conn
            .command(Some(&session), "Runtime.runIfWaitingForDebugger", json!({}))
            .await;
    }
    entry.target_ready(&target, &session);
}

/// Blocks the ad and tracker addresses in a page or frame, and attaches to
/// the frames and popups it makes.
async fn configure(conn: &Connection, session: &str) -> Result<(), CdpError> {
    // Nothing here reads a response body, so none is kept: Chromium's
    // defaults keep up to 100 MB of them per page. A source that needs to
    // read bodies enables the Network domain with limits of its own.
    conn.command(
        Some(session),
        "Network.enable",
        json!({ "maxTotalBufferSize": 0, "maxResourceBufferSize": 0 }),
    )
    .await?;
    conn.command(
        Some(session),
        "Network.setBlockedURLs",
        json!({ "urls": BLOCKED_URLS }),
    )
    .await?;
    // Frames of other sites are targets of their own: set them up too. A
    // refusal leaves this one set up.
    let _ = conn
        .command(
            Some(session),
            "Target.setAutoAttach",
            json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true }),
        )
        .await;
    Ok(())
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
}
