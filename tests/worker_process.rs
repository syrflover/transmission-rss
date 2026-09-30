//! The real `trss-worker` binary: signals, exclusivity between processes,
//! recovery when Transmission was down, and what it prints.

mod common;

use std::{
    fs::File,
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

use common::*;
use transmission_rss::store::history::HistoryResult;

struct Proc {
    child: Child,
    stdout: PathBuf,
    stderr: PathBuf,
}

impl Proc {
    fn spawn(h: &Harness, name: &str, extra_env: &[(&str, &str)]) -> Proc {
        let stdout = h.dir.path().join(format!("{name}.out"));
        let stderr = h.dir.path().join(format!("{name}.err"));
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_trss-worker"));
        cmd.current_dir(h.dir.path())
            .env("TRSS_DB_PATH", h.db_path())
            .env("TRANSMISSION_URL", h.tr.url())
            .env_remove("CHANNELS_CONFIG_URL")
            .stdout(File::create(&stdout).unwrap())
            .stderr(File::create(&stderr).unwrap());
        for (key, value) in extra_env {
            cmd.env(key, value);
        }
        Proc {
            child: cmd.spawn().expect("spawn trss-worker"),
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
    assert!(output.contains("trss-worker stopped"), "{output}");
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
async fn the_worker_keeps_running_while_transmission_is_down_and_adds_when_it_returns() {
    let mut h = harness_with_channel_a().await;
    h.tr.stop().await;
    let mut worker = Proc::spawn(&h, "w", &[("TRSS_WORKER_INTERVAL_SECS", "1")]);

    wait_until("failed additions", || async {
        count_result(&h, HistoryResult::AddFailed).await == 3
    })
    .await;
    assert!(worker.is_running());
    let failed = h.item("Sayonara Lara - 03 (1080p)").await;
    assert!(failed.reason.is_some());

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
async fn two_worker_processes_run_a_period_once() {
    let h = harness_with_channel_a().await;
    let mut a = Proc::spawn(&h, "a", &[]);
    let mut b = Proc::spawn(&h, "b", &[]);

    a.wait_output("trss-worker started").await;
    b.wait_output("trss-worker started").await;
    wait_until("one cycle", || cycle_finished(&h)).await;
    wait_until("the other to have skipped", || async {
        let skipped = |p: &Proc| {
            let out = p.output();
            out.contains("Another worker is running a cycle")
                || out.contains("A cycle started recently")
        };
        skipped(&a) || skipped(&b)
    })
    .await;
    // Give a wrongly admitted second cycle time to show itself.
    tokio::time::sleep(Duration::from_millis(500)).await;

    assert_eq!(h.feeds.hits("feed-a"), 1, "the feed was read once");
    assert_eq!(h.tr.calls_of("torrent-add").len(), 3);
    let finished = |p: &Proc| p.output().matches("Cycle finished").count();
    assert_eq!(finished(&a) + finished(&b), 1);

    for w in [&a, &b] {
        w.sigterm();
    }
    for w in [&mut a, &mut b] {
        assert!(w.wait_exit(Duration::from_secs(10)).await.success());
    }
}

#[tokio::test]
async fn logs_and_history_never_contain_secret_query_values() {
    let h = harness_with_channel_a().await;
    // A feed answering 500 and one whose server is not there: errors that
    // quote their request URL if nobody strips it.
    h.add_channel("broken", "/media/x", &[], vec![rule("x", "x")])
        .await;
    h.feeds.set_status("broken", 500);
    let dead = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };
    h.channels
        .create_channel(transmission_rss::store::channels::ChannelInput::new(
            format!("http://{dead}/feed?filter=1080p&token={SECRET}"),
            "/media/y",
        ))
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

    let items = h.history_items().await;
    assert!(!items.is_empty());
    for item in items {
        let text = format!("{item:?}");
        assert!(!text.contains(SECRET), "{text}");
    }
}

#[tokio::test]
async fn the_worker_needs_transmission_and_a_database_but_not_the_yaml_url() {
    let h = Harness::new().await;
    let run = |vars: &[(&str, &str)], remove: &[&str]| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_trss-worker"));
        cmd.current_dir(h.dir.path())
            .env_remove("CHANNELS_CONFIG_URL")
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
