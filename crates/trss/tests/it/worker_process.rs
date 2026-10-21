//! The real `trss` binary: signals, one process on an app data folder,
//! recovery when Transmission was down, and what it prints.

use crate::common;

use std::{
    fs::File,
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

use common::*;
use serde_json::{json, Value};
use trss_collect::store::{
    channels::{ChannelInput, RuleInput},
    history::HistoryResult,
};
use trss_core::settings::SettingsStore;

struct Proc {
    child: Child,
    stdout: PathBuf,
    stderr: PathBuf,
}

impl Proc {
    fn spawn(h: &Harness, name: &str, extra_env: &[(&str, &str)]) -> Proc {
        let stdout = h.dir.path().join(format!("{name}.out"));
        let stderr = h.dir.path().join(format!("{name}.err"));
        let mut cmd = h.trss_process();
        cmd.stdout(File::create(&stdout).unwrap())
            .stderr(File::create(&stderr).unwrap());
        for (key, value) in extra_env {
            cmd.env(key, value);
        }
        Proc {
            child: cmd.spawn().expect("spawn trss"),
            stdout,
            stderr,
        }
    }

    fn output(&self) -> String {
        format!(
            "{}{}",
            std::fs::read_to_string(&self.stdout).unwrap_or_default(),
            std::fs::read_to_string(&self.stderr).unwrap_or_default()
        )
    }

    fn sigterm(&self) {
        let status = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .expect("run kill");
        assert!(status.success());
    }

    fn is_running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    async fn wait_exit(&mut self, within: Duration) -> ExitStatus {
        let deadline = Instant::now() + within;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "worker did not exit within {within:?}; output:\n{}",
                self.output()
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    async fn wait_output(&self, needle: &str) {
        wait_until(&format!("output containing {needle:?}"), || async {
            self.output().contains(needle)
        })
        .await;
    }

    /// The address the process's web listens on, once it has said so.
    async fn web_addr(&self) -> String {
        const SAID: &str = "trss listening on http://";
        let said = || {
            self.output().lines().find_map(|line| {
                let rest = line.strip_prefix(SAID)?;
                Some(rest.split_whitespace().next()?.to_owned())
            })
        };
        wait_until("the web to listen", || async { said().is_some() }).await;
        said().unwrap()
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn wait_until<F, Fut>(what: &str, mut cond: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(30);
    while !cond().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn cycle_finished(h: &Harness) -> bool {
    h.history
        .last_cycle()
        .await
        .unwrap()
        .is_some_and(|c| c.finished_at.is_some())
}

async fn count_result(h: &Harness, result: HistoryResult) -> usize {
    h.history_items()
        .await
        .iter()
        .filter(|i| i.result == result)
        .count()
}

async fn harness_with_channel_a() -> Harness {
    let h = Harness::new().await;
    h.add_channel(
        "feed-a",
        "/media/anime",
        &["[Batch]", "(720p)"],
        feed_a_rules(),
    )
    .await;
    h
}

#[tokio::test]
async fn sigterm_stops_an_idle_worker_cleanly() {
    let h = harness_with_channel_a().await;
    let mut worker = Proc::spawn(&h, "w", &[]);

    wait_until("the first cycle", || cycle_finished(&h)).await;
    assert_eq!(count_result(&h, HistoryResult::Received).await, 3);

    worker.sigterm();
    let status = worker.wait_exit(Duration::from_secs(5)).await;
    assert!(status.success(), "{status:?}\n{}", worker.output());
    let output = worker.output();
    assert!(output.contains("SIGTERM received"), "{output}");
    assert!(output.contains("trss stopped"), "{output}");
}

#[tokio::test]
async fn sigterm_during_adding_lets_in_flight_items_finish_and_leaves_nothing_half_recorded() {
    let h = harness_with_channel_a().await;
    let add_gate = h.tr.hold("torrent-add");
    let mut worker = Proc::spawn(&h, "w", &[]);

    add_gate.wait_arrived().await;
    worker.sigterm();
    worker.wait_output("SIGTERM received").await;
    // Transmission answers only after the signal arrived.
    add_gate.release_all();

    let status = worker.wait_exit(Duration::from_secs(10)).await;
    assert!(status.success(), "{status:?}\n{}", worker.output());

    // What Transmission took is recorded; nothing more was started.
    assert_eq!(h.tr.torrents().len(), 3);
    assert_eq!(count_result(&h, HistoryResult::Received).await, 3);
    assert!(h.tr.calls_of("torrent-rename-path").is_empty());
    assert!(h.tr.calls_of("torrent-remove").is_empty());
    assert!(worker.output().contains("Cycle interrupted by shutdown"));
}

#[tokio::test]
async fn sigterm_while_reading_a_feed_exits_without_waiting_for_it() {
    let h = harness_with_channel_a().await;
    let gate = h.feeds.hold("feed-a");
    let mut worker = Proc::spawn(&h, "w", &[]);

    gate.wait_arrived().await;
    worker.sigterm();

    let status = worker.wait_exit(Duration::from_secs(5)).await;
    assert!(status.success(), "{status:?}\n{}", worker.output());
    assert!(h.history_items().await.is_empty());
    assert!(h.tr.calls_of("torrent-add").is_empty());
}

#[tokio::test]
async fn sigterm_while_transmission_hangs_exits_within_the_grace_period() {
    let h = harness_with_channel_a().await;
    // Transmission takes the connection and never answers `torrent-add`; the
    // request timeout (30 s) is far longer than the shutdown grace (5 s).
    let gate = h.tr.hold("torrent-add");
    let mut worker = Proc::spawn(&h, "w", &[]);

    gate.wait_arrived().await;
    let asked = Instant::now();
    worker.sigterm();

    let status = worker.wait_exit(Duration::from_secs(9)).await;
    assert!(status.success(), "{status:?}\n{}", worker.output());
    assert!(asked.elapsed() < Duration::from_secs(9));
    let output = worker.output();
    assert!(output.contains("Cycle abandoned"), "{output}");
    assert!(output.contains("trss stopped"), "{output}");
    // Nothing was recorded for the items whose adds never answered.
    assert_eq!(count_result(&h, HistoryResult::Received).await, 0);
}

#[tokio::test]
async fn the_worker_keeps_running_while_transmission_is_down_and_adds_when_it_returns() {
    let mut h = harness_with_channel_a().await;
    h.tr.stop().await;
    let mut worker = Proc::spawn(&h, "w", &[("TRSS_WORKER_INTERVAL_SECS", "1")]);

    wait_until("failed additions", || async {
        count_result(&h, HistoryResult::AddFailed).await == 3
    })
    .await;
    assert!(worker.is_running());

    h.tr.restart().await;
    wait_until("received items", || async {
        count_result(&h, HistoryResult::Received).await == 3
    })
    .await;
    assert!(worker.is_running(), "{}", worker.output());
    assert_eq!(h.tr.torrents().len(), 3);

    worker.sigterm();
    let status = worker.wait_exit(Duration::from_secs(10)).await;
    assert!(status.success(), "{status:?}\n{}", worker.output());
}

#[tokio::test]
async fn a_second_process_on_the_same_app_data_stops_before_it_runs_anything() {
    let h = harness_with_channel_a().await;
    let gate = h.feeds.hold("feed-a");

    // The first process starts a cycle and stays in it, waiting for the feed.
    let mut a = Proc::spawn(&h, "a", &[]);
    gate.wait_arrived().await;

    // The second one ticks every second, so it would start a cycle at once.
    let mut b = Proc::spawn(&h, "b", &[("TRSS_WORKER_INTERVAL_SECS", "1")]);
    let status = b.wait_exit(Duration::from_secs(10)).await;
    let output = b.output();
    assert!(!status.success(), "{status:?}\n{output}");
    assert!(
        output.contains("another trss runs on this app data"),
        "{output}"
    );
    // It served nothing and ran nothing.
    assert!(!output.contains("trss listening"), "{output}");
    assert!(!output.contains("trss started"), "{output}");
    assert!(!output.contains("Cycle started"), "{output}");
    assert_eq!(h.feeds.hits("feed-a"), 1, "the feed was read once");
    assert!(h.tr.calls_of("torrent-add").is_empty());

    // The first process finishes its cycle undisturbed.
    gate.release_all();
    wait_until("the first cycle", || cycle_finished(&h)).await;
    assert_eq!(count_result(&h, HistoryResult::Received).await, 3);
    assert_eq!(h.tr.calls_of("torrent-add").len(), 3);
    a.sigterm();
    assert!(a.wait_exit(Duration::from_secs(10)).await.success());

    // Once it stopped, the next process starts.
    let mut c = Proc::spawn(&h, "c", &[]);
    c.wait_output("trss started").await;
    c.sigterm();
    assert!(c.wait_exit(Duration::from_secs(10)).await.success());
}

#[tokio::test]
async fn a_job_left_running_is_interrupted_before_the_web_serves() {
    // That the conversion keeps each step and leaves the waiting jobs is
    // trss-jobs' rule (`interrupted` tests); this is when the process makes it.
    let h = Harness::new().await;
    let job = trss_jobs::NewJob {
        command_id: "c1".to_owned(),
        request: "{}".to_owned(),
        origin: "pick".to_owned(),
        work_id: None,
        season: Some(1),
        movie: None,
        anime_no: None,
        source_id: None,
        creator: Some("creator".to_owned()),
        revision_of: None,
        revises_attributed: false,
        items: vec![trss_jobs::NewItem {
            observation_id: None,
            episode: "1".to_owned(),
            post_url: "https://fake.trss.invalid/ok/ep1".to_owned(),
            found_at: 500,
        }],
    };
    let trss_jobs::Created::Created(id) = trss_jobs::JobRequests::new(h.db.clone())
        .create(job, 900)
        .await
        .unwrap()
    else {
        panic!("the job was not created");
    };
    let state = || {
        let id = id.clone();
        h.db.run(move |c| {
            c.query_row("SELECT state FROM subtitle_jobs WHERE id = ?1", [id], |r| {
                r.get::<_, String>(0)
            })
            .map_err(trss_core::DbError::from)
        })
    };
    // An earlier process left it running in its apply step.
    let running = id.clone();
    h.db.run(move |c| {
        c.execute(
            "UPDATE subtitle_jobs SET state = 'running', stage = 'apply' WHERE id = ?1",
            [running],
        )
        .map_err(trss_core::DbError::from)
    })
    .await
    .unwrap();

    let mut p = Proc::spawn(&h, "p", &[]);
    p.wait_output("trss listening").await;
    // Interrupted before the web took a request: a person's restart then is
    // never turned back by it.
    assert_eq!(state().await.unwrap(), "interrupted");
    let output = p.output();
    let interrupted = output
        .find("Subtitle jobs: 1 left running by an earlier process are interrupted")
        .unwrap_or_else(|| panic!("{output}"));
    assert!(
        interrupted < output.find("trss listening").unwrap(),
        "{output}"
    );
    p.sigterm();
    assert!(p.wait_exit(Duration::from_secs(10)).await.success());
}

#[tokio::test]
async fn the_start_empties_the_receive_attempts_folder_before_the_web_serves() {
    // What emptying keeps is trss-jobs' rule (`interrupted` tests); this is
    // when the process does it.
    let h = Harness::new().await;
    let receive = h.dir.path().join("receive");
    let made = |relative: &str| {
        let path = receive.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"bytes").unwrap();
        path
    };
    let cut = made(".partial/6a1d7d88-3f0e-4a7f-8d57-2b6c4c0e9f02/a.ass");
    let check = made(".tmp/check-job-1/ep1.srt");

    let mut p = Proc::spawn(&h, "p", &[]);
    p.wait_output("trss listening").await;
    assert!(!cut.exists());
    assert!(std::fs::read_dir(receive.join(".partial"))
        .unwrap()
        .next()
        .is_none());
    assert!(check.exists());
    let output = p.output();
    let emptied = output
        .find("trss: removed 1 unfinished receive attempts")
        .unwrap_or_else(|| panic!("{output}"));
    assert!(emptied < output.find("trss listening").unwrap(), "{output}");
    p.sigterm();
    assert!(p.wait_exit(Duration::from_secs(10)).await.success());
}

#[tokio::test]
async fn the_start_empties_the_stored_files_temporary_folder_before_the_web_serves() {
    // A store into the app data folder writes its new copy in
    // `subtitle-files/.tmp/` (trss-jobs' `place::files`): what a process
    // left there goes before any job runs, the stored files beside it stay.
    let h = Harness::new().await;
    let files = h.dir.path().join("subtitle-files");
    let made = |relative: &str| {
        let path = files.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"bytes").unwrap();
        path
    };
    let cut = made(".tmp/0b9e4f61-7c2d-4f0e-9a3b-5d6c7e8f9a01");
    let stored = made("w1/제작자/readme.txt");

    let mut p = Proc::spawn(&h, "p", &[]);
    p.wait_output("trss listening").await;
    assert!(!cut.exists());
    assert!(std::fs::read_dir(files.join(".tmp"))
        .unwrap()
        .next()
        .is_none());
    assert!(stored.exists());
    let output = p.output();
    let emptied = output
        .find("trss: removed 1 unfinished stored-file copies")
        .unwrap_or_else(|| panic!("{output}"));
    assert!(emptied < output.find("trss listening").unwrap(), "{output}");
    p.sigterm();
    assert!(p.wait_exit(Duration::from_secs(10)).await.success());
}

#[tokio::test]
async fn logs_and_history_never_contain_secret_query_values() {
    let h = harness_with_channel_a().await;
    // A feed answering 500 and one whose server is not there: errors that
    // quote their request URL if nobody strips it.
    h.add_channel("broken", "/media/x", &[], vec![rule("x", "x")])
        .await;
    h.feeds.set_status("broken", 500);
    let dead = trss_core::loopback::unused_addr();
    h.channels
        .create_channel(trss_collect::store::channels::ChannelInput::new(format!(
            "http://{dead}/feed?filter=1080p&token={SECRET}"
        )))
        .await
        .unwrap();
    // Not RSS at all.
    h.feeds.set_xml("junk", "<html>not a feed</html>");
    h.add_channel("junk", "/media/z", &[], vec![rule("x", "x")])
        .await;

    let mut worker = Proc::spawn(&h, "w", &[]);
    wait_until("the cycle", || cycle_finished(&h)).await;
    worker.sigterm();
    assert!(worker.wait_exit(Duration::from_secs(10)).await.success());

    let output = worker.output();
    assert!(
        !output.contains(SECRET),
        "secret in the worker's output:\n{output}"
    );
    // The failures are reported, with the channel in masked form.
    assert!(output.contains("token=***"), "{output}");
    assert!(output.contains("HTTP status 500"), "{output}");
    assert_eq!(output.matches("Failed ").count(), 3, "{output}");
    // That history holds no secret is trss-collect's rule (`cycle` tests).
}

#[tokio::test]
async fn the_worker_needs_transmission_and_a_database_but_not_the_yaml_url() {
    let h = Harness::new().await;
    let run = |vars: &[(&str, &str)], remove: &[&str]| {
        let mut cmd = h.trss_process();
        cmd.env_remove("CHANNELS_CONFIG_URL")
            .env_remove("TRSS_DB_PATH")
            .env_remove("TRANSMISSION_URL")
            .stdin(Stdio::null());
        for key in remove {
            cmd.env_remove(key);
        }
        for (key, value) in vars {
            cmd.env(key, value);
        }
        cmd.output().unwrap()
    };

    let out = run(&[("TRSS_DB_PATH", h.db_path().to_str().unwrap())], &[]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("TRANSMISSION_URL"));
    assert!(!String::from_utf8_lossy(&out.stderr).contains("CHANNELS_CONFIG_URL"));

    let out = run(&[("TRANSMISSION_URL", "http://127.0.0.1:1/")], &[]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("TRSS_DB_PATH"));
}

// --- the screens' event stream (ticket 0154) ----------------------------------------------

/// What Transmission says while a test makes it refuse an add.
const REFUSAL: &str = "Transmission is having a bad day";

/// A client of the process's `GET /api/events`, reading the events as a
/// browser's `EventSource` would.
struct EventStream {
    response: reqwest::Response,
    pending: String,
}

impl EventStream {
    async fn open(addr: &str) -> EventStream {
        let response = reqwest::Client::new()
            .get(format!("http://{addr}/api/events"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers()["content-type"],
            "text/event-stream",
            "{:?}",
            response.headers()
        );
        EventStream {
            response,
            pending: String::new(),
        }
    }

    /// The next event as its name and its JSON `data`, skipping comments;
    /// `None` when the server closed the stream, and a panic when nothing
    /// came within `within`.
    async fn next(&mut self, within: Duration) -> Option<(String, Value)> {
        self.try_next(within)
            .await
            .unwrap_or_else(|| panic!("no event within {within:?}; read: {:?}", self.pending))
    }

    /// [`EventStream::next`] that gives `None` when nothing came within
    /// `within`, and `Some(None)` when the server closed the stream.
    async fn try_next(&mut self, within: Duration) -> Option<Option<(String, Value)>> {
        loop {
            if let Some(end) = self.pending.find("\n\n") {
                let block: String = self.pending.drain(..end + 2).collect();
                let mut name = String::new();
                let mut data = String::new();
                for line in block.lines() {
                    if let Some(value) = line.strip_prefix("event:") {
                        name = value.trim().to_owned();
                    } else if let Some(value) = line.strip_prefix("data:") {
                        data.push_str(value.trim());
                    }
                }
                if name.is_empty() && data.is_empty() {
                    continue;
                }
                let data = serde_json::from_str(&data).expect("JSON data");
                return Some(Some((name, data)));
            }
            let Ok(chunk) = tokio::time::timeout(within, self.response.chunk()).await else {
                return None;
            };
            let Some(chunk) = chunk.ok().flatten() else {
                return Some(None);
            };
            self.pending.push_str(std::str::from_utf8(&chunk).unwrap());
        }
    }

    /// Reads the first event, which has to be `ready`.
    async fn ready(&mut self) {
        let first = self.next(Duration::from_secs(10)).await;
        assert_eq!(first, Some(("ready".to_owned(), json!({}))));
    }

    /// Reads until `count` changes of `(kind, id)` came, ignoring the others.
    async fn changes_of(&mut self, kind: &str, id: &str, count: usize) {
        let mut seen = 0;
        while seen < count {
            let (name, data) = self
                .next(Duration::from_secs(30))
                .await
                .expect("the stream ended");
            if name == "change" && data == json!({ "kind": kind, "id": id }) {
                seen += 1;
            }
        }
    }

    /// Whether another change of `(kind, id)` comes within `within`.
    async fn more_changes_of(&mut self, kind: &str, id: &str, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.try_next(left).await {
                Some(Some((name, data))) => {
                    if name == "change" && data == json!({ "kind": kind, "id": id }) {
                        return true;
                    }
                }
                Some(None) | None => break,
            }
        }
        false
    }
}

async fn post_json(addr: &str, path: &str, body: Value) -> (reqwest::StatusCode, Value) {
    let response = reqwest::Client::new()
        .post(format!("http://{addr}{path}"))
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

async fn get_json(addr: &str, path: &str) -> Value {
    let text = reqwest::Client::new()
        .get(format!("http://{addr}{path}"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    serde_json::from_str(&text).unwrap_or_else(|_| panic!("not JSON: {text}"))
}

/// Sends a `receive_once` command for `item_id` to the process's web as a
/// screen does.
async fn post_receive_once(addr: &str, id: &str, item_id: i64) -> reqwest::StatusCode {
    let body = json!({ "id": id, "kind": "receive_once", "payload": { "item_id": item_id } });
    post_json(addr, "/api/commands", body).await.0
}

#[tokio::test]
async fn a_client_of_the_event_stream_hears_a_command_accepted_run_and_ended_done_or_failed() {
    let h = harness_with_channel_a().await;
    // The first cycle's adds are refused: its items are `add_failed` with the
    // rule recorded, which is what `다시 받기` takes.
    h.tr.reject_adds(Some(REFUSAL));
    let worker = Proc::spawn(&h, "w", &[("TRSS_WORKER_INTERVAL_SECS", "3600")]);
    wait_until("the first cycle", || cycle_finished(&h)).await;
    assert_eq!(count_result(&h, HistoryResult::AddFailed).await, 3);
    let items = h.history_items().await;
    let addr = worker.web_addr().await;
    let mut events = EventStream::open(&addr).await;
    events.ready().await;
    let quiet = Duration::from_millis(500);

    // Adding works again: accepted, started and ended done, one change each.
    h.tr.reject_adds(None);
    let status = post_receive_once(&addr, "cmd-done", items[0].id).await;
    assert_eq!(status, 202, "{}", worker.output());
    events.changes_of("command", "cmd-done", 3).await;
    let done = get_json(&addr, "/api/commands/cmd-done").await;
    assert_eq!(done["state"], "done", "{done}");
    assert!(
        !events.more_changes_of("command", "cmd-done", quiet).await,
        "a fourth change for a command that changes in three steps"
    );

    // Refused again: the command ends failed.
    h.tr.reject_adds(Some(REFUSAL));
    let status = post_receive_once(&addr, "cmd-failed", items[1].id).await;
    assert_eq!(status, 202, "{}", worker.output());
    events.changes_of("command", "cmd-failed", 3).await;
    let failed = get_json(&addr, "/api/commands/cmd-failed").await;
    assert_eq!(failed["state"], "failed", "{failed}");
    assert!(
        !events.more_changes_of("command", "cmd-failed", quiet).await,
        "a fourth change for a command that changes in three steps"
    );
}

#[tokio::test]
async fn a_client_of_the_event_stream_hears_what_the_workers_write_changed_in_the_database() {
    let h = harness_with_channel_a().await;
    h.tr.reject_adds(Some(REFUSAL));
    let worker = Proc::spawn(&h, "w", &[("TRSS_WORKER_INTERVAL_SECS", "3600")]);
    wait_until("the first cycle", || cycle_finished(&h)).await;
    assert_eq!(count_result(&h, HistoryResult::AddFailed).await, 3);
    let items = h.history_items().await;
    let addr = worker.web_addr().await;
    let mut events = EventStream::open(&addr).await;
    events.ready().await;

    // The web only accepts the command; the worker part of the process adds
    // the torrent and records the item as received, through its own database
    // connection. Nothing in that code announces: the connection tells which
    // history item its commit changed, as `history`, not as the command.
    h.tr.reject_adds(None);
    let status = post_receive_once(&addr, "cmd-history", items[0].id).await;
    assert_eq!(status, 202, "{}", worker.output());
    events
        .changes_of("history", &items[0].id.to_string(), 1)
        .await;
    // The change came after the commit: the screen's own route shows it.
    let item = get_json(&addr, &format!("/api/history/{}", items[0].id)).await;
    assert_ne!(item["result"], "add_failed", "{item}");
}

#[tokio::test]
async fn a_client_of_the_event_stream_hears_a_past_search_start_get_further_and_end() {
    let h = Harness::without_collect_folder().await;
    let media = h.dir.path().join("media");
    std::fs::create_dir_all(media.join("Show/Season 01")).unwrap();
    SettingsStore::new(h.db.clone())
        .put_collection(0, media.to_str().unwrap().to_owned(), None)
        .await
        .unwrap();
    // A first page that is full and lacks episodes 1 to 5 of the range: one
    // more search, sent after the tracker's spacing.
    let nyaa = FakeNyaa::start().await;
    let releases: Vec<String> = (1..=100)
        .rev()
        .map(|n| {
            format!(
                "[SubsPlease] Show - {n:02} (1080p) [{:08X}].mkv",
                0xA000_0000u32 + n
            )
        })
        .collect();
    nyaa.set_releases(&releases);
    let mut input = ChannelInput::new(nyaa.url(SECRET));
    input.past_search = Some("[SubsPlease] {match} 1080p".to_owned());
    let channel = h
        .channels
        .create_channel_with_rules(
            input,
            vec![RuleInput {
                r#match: Some("Show".to_owned()),
                directory: "Show/Season 01".to_owned(),
                ..Default::default()
            }],
        )
        .await
        .unwrap();
    let worker = Proc::spawn(&h, "w", &[("TRSS_WORKER_INTERVAL_SECS", "3600")]);
    let addr = worker.web_addr().await;
    let mut events = EventStream::open(&addr).await;
    events.ready().await;

    let path = format!("/api/rules/{}/past-search", channel.rules[0].id);
    let body = json!({ "query": "[SubsPlease] Show 1080p", "from": 1, "to": 5 });
    let (status, started) = post_json(&addr, &path, body).await;
    assert_eq!(status, 202, "{started}");
    let id = started["search_id"].as_str().unwrap();

    // Started, got to its extra search (0 of 1), sent it (1 of 1), ended.
    events.changes_of("past_search", id, 4).await;
    let poll = get_json(&addr, &format!("/api/past-searches/{id}")).await;
    assert_eq!(poll["state"], "done", "{poll}\n{}", worker.output());
    let result = &poll["result"];
    assert_eq!(
        (&result["extra_sent"], &result["extra_needed"]),
        (&json!(1), &json!(1))
    );
    assert!(
        !events
            .more_changes_of("past_search", id, Duration::from_millis(500))
            .await
    );
}

#[tokio::test]
async fn sigterm_with_an_event_stream_open_exits_promptly_and_closes_the_stream() {
    let h = Harness::new().await;
    let mut worker = Proc::spawn(&h, "w", &[("TRSS_WORKER_INTERVAL_SECS", "3600")]);
    let addr = worker.web_addr().await;
    let mut events = EventStream::open(&addr).await;
    events.ready().await;

    worker.sigterm();

    // The server waits for open responses before it stops: a stream that did
    // not end would hold it until the container's grace period killed it.
    let status = worker.wait_exit(Duration::from_secs(5)).await;
    assert!(status.success(), "{status:?}\n{}", worker.output());
    assert_eq!(events.next(Duration::from_secs(5)).await, None);
}
