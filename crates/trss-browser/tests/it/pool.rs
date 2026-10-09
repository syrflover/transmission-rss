//! The pool against a fake launcher with a scripted DevTools.

use crate::support;

use std::{os::unix::fs::PermissionsExt, time::Duration};

use serde_json::json;
use support::{eventually, make_pool, Setup, TOKEN};
use trss_browser::{BrowserError, BrowserPolicy, DownloadState};

/// What Chromium names a download: a UUID.
const GUID_1: &str = "0f6e2b0e-4a52-4d3c-9d3e-2f1c7a5b8e90";
const GUID_2: &str = "7c1d9a3e-52b0-4f8e-a6d1-93e0b4c2f7a1";

fn policy(idle_secs: u64, max: usize) -> BrowserPolicy {
    BrowserPolicy {
        idle: Duration::from_secs(idle_secs),
        max_concurrent: max,
    }
}

fn index_of(commands: &[support::Command], session: &str, method: &str) -> Option<usize> {
    commands
        .iter()
        .position(|c| c.session.as_deref() == Some(session) && c.method == method)
}

#[tokio::test]
async fn a_new_pool_resets_the_launcher_and_empties_the_downloads_folder() {
    let setup = Setup::with(policy(300, 1), TOKEN, |root| {
        std::fs::create_dir_all(root.join("old-run")).unwrap();
        std::fs::write(root.join("old-run/guid"), "half a file").unwrap();
    })
    .await;
    assert_eq!(setup.fake.calls(), ["POST /reset"]);
    assert_eq!(std::fs::read_dir(&setup.downloads_root).unwrap().count(), 0);
    // The browser runs as another user and makes a folder here for each run.
    let mode = std::fs::metadata(&setup.downloads_root)
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o7777, 0o1777);
}

#[tokio::test]
async fn a_launcher_that_was_not_up_at_creation_is_reset_at_the_first_start() {
    let dir = tempfile::tempdir().unwrap();
    let clock = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0));
    let fake = support::fake_launcher(dir.path().to_owned(), 4, clock.clone()).await;
    fake.state
        .fail_reset
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let policy = std::sync::Arc::new(std::sync::Mutex::new(BrowserPolicy::default()));
    let pool = make_pool(&fake, dir.path(), TOKEN, &clock, &policy).await;
    assert_eq!(fake.calls(), ["POST /reset"]);

    fake.state
        .fail_reset
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let run = pool.start("job").await.unwrap();
    assert_eq!(
        fake.calls_matching("POST /reset").len(),
        2,
        "{:?}",
        fake.calls()
    );
    // Reset once; later starts do not reset again.
    run.end().await;
    pool.start("job-2").await.unwrap();
    assert_eq!(fake.calls_matching("POST /reset").len(), 2);
}

#[tokio::test]
async fn a_run_is_set_up_before_anything_loads() {
    let setup = Setup::new(policy(300, 1)).await;
    let run = setup.pool.start("01JOB").await.unwrap();

    assert!(run.run_id().starts_with("01JOB-"), "{}", run.run_id());
    assert_eq!(run.job_id(), "01JOB");
    assert_eq!(run.downloads_dir(), setup.downloads_root.join(run.run_id()));
    assert_eq!(setup.fake.run_ids(), [run.run_id().to_owned()]);

    let commands = setup.fake.commands(run.run_id());
    let download = commands
        .iter()
        .find(|c| c.method == "Browser.setDownloadBehavior")
        .unwrap();
    assert_eq!(download.params["behavior"], "allowAndName");
    assert_eq!(download.params["eventsEnabled"], true);
    assert_eq!(
        download.params["downloadPath"],
        setup
            .downloads_root
            .join(run.run_id())
            .to_string_lossy()
            .as_ref()
    );
    let auto = commands
        .iter()
        .find(|c| c.method == "Target.setAutoAttach" && c.session.is_none())
        .unwrap();
    assert_eq!(auto.params["flatten"], true);
    assert_eq!(auto.params["waitForDebuggerOnStart"], true);

    // The page the browser opened itself is blocked before it is let go.
    eventually("the first page to be let go", || {
        index_of(
            &setup.fake.commands(run.run_id()),
            "S0",
            "Runtime.runIfWaitingForDebugger",
        )
        .is_some()
    })
    .await;
    let commands = setup.fake.commands(run.run_id());
    let blocked = index_of(&commands, "S0", "Network.setBlockedURLs").unwrap();
    let enable = index_of(&commands, "S0", "Network.enable").unwrap();
    assert!(enable < blocked);
    // No response bodies are kept: nothing reads them.
    assert_eq!(commands[enable].params["maxTotalBufferSize"], 0);
    assert_eq!(commands[enable].params["maxResourceBufferSize"], 0);
    assert!(blocked < index_of(&commands, "S0", "Runtime.runIfWaitingForDebugger").unwrap());
    let urls = commands[blocked].params["urls"].as_array().unwrap();
    let urls: Vec<&str> = urls.iter().filter_map(|u| u.as_str()).collect();
    assert!(urls.iter().any(|u| u.contains("googlesyndication")));
    assert!(urls.iter().any(|u| u.contains("doubleclick")));
    assert!(urls.iter().any(|u| u.contains("googletagmanager")));
    assert!(urls.iter().any(|u| u.contains("google-analytics")));
    assert!(urls.iter().any(|u| u.contains("adtrafficquality")));
    assert!(
        !urls.iter().any(|u| u.contains("cloudflare")),
        "the Turnstile service must load"
    );
}

#[tokio::test]
async fn every_page_is_set_up_new_tabs_and_popups_included() {
    let setup = Setup::new(policy(300, 1)).await;
    let run = setup.pool.start("job").await.unwrap();

    let page = run.new_page("https://example.invalid/post").await.unwrap();
    let session = page.session_id().unwrap();
    let commands = setup.fake.commands(run.run_id());
    let blocked = index_of(&commands, &session, "Network.setBlockedURLs").unwrap();
    let navigate = index_of(&commands, &session, "Page.navigate").unwrap();
    assert!(
        blocked < navigate,
        "the page was told to load before it was set up"
    );
    assert_eq!(
        commands[navigate].params["url"],
        "https://example.invalid/post"
    );

    // A popup the page opens by itself.
    setup
        .fake
        .emit_attached(run.run_id(), "TPOP", "SPOP", "page");
    eventually("the popup to be set up", || {
        let commands = setup.fake.commands(run.run_id());
        index_of(&commands, "SPOP", "Network.setBlockedURLs").is_some()
            && index_of(&commands, "SPOP", "Runtime.runIfWaitingForDebugger").is_some()
    })
    .await;
    // A frame of another site gets the blocklist too; a worker is only let go.
    setup
        .fake
        .emit_attached(run.run_id(), "TFRAME", "SFRAME", "iframe");
    setup
        .fake
        .emit_attached(run.run_id(), "TWORK", "SWORK", "service_worker");
    eventually("the frame and the worker to be let go", || {
        let commands = setup.fake.commands(run.run_id());
        index_of(&commands, "SFRAME", "Runtime.runIfWaitingForDebugger").is_some()
            && index_of(&commands, "SWORK", "Runtime.runIfWaitingForDebugger").is_some()
    })
    .await;
    let commands = setup.fake.commands(run.run_id());
    assert!(index_of(&commands, "SFRAME", "Network.setBlockedURLs").is_some());
    assert!(index_of(&commands, "SWORK", "Network.setBlockedURLs").is_none());

    let pages: Vec<String> = run
        .pages()
        .iter()
        .map(|p| p.target_id().to_owned())
        .collect();
    assert!(pages.contains(&"TPOP".to_owned()) && pages.contains(&page.target_id().to_owned()));
    assert!(
        !pages.contains(&"TFRAME".to_owned()),
        "a frame is not a page"
    );

    // The page answers scripts.
    assert_eq!(page.evaluate("1 + 1").await.unwrap(), json!("1 + 1"));
}

#[tokio::test]
async fn with_a_cap_of_one_the_second_job_waits_for_the_first() {
    let setup = Setup::new(policy(300, 1)).await;
    let first = setup.pool.start("job-a").await.unwrap();

    let second = tokio::spawn({
        let pool = setup.pool.clone();
        async move { pool.start("job-b").await }
    });
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!second.is_finished(), "the second job did not wait");
    assert_eq!(setup.fake.calls_matching("POST /runs").len(), 1);
    assert_eq!(setup.fake.run_ids().len(), 1);

    first.end().await;
    let second = tokio::time::timeout(Duration::from_secs(5), second)
        .await
        .expect("the second job never got the slot")
        .unwrap()
        .unwrap();
    assert_eq!(second.job_id(), "job-b");
    assert_ne!(second.run_id(), first.run_id());
    assert_eq!(setup.fake.run_ids(), [second.run_id().to_owned()]);
}

#[tokio::test]
async fn a_start_that_is_given_up_while_waiting_leaves_nothing() {
    let setup = Setup::new(policy(300, 1)).await;
    let _first = setup.pool.start("job-a").await.unwrap();
    let waiting = tokio::time::timeout(Duration::from_millis(300), setup.pool.start("job-b")).await;
    assert!(waiting.is_err());
    assert_eq!(setup.pool.status().len(), 1);
    assert_eq!(setup.fake.calls_matching("POST /runs").len(), 1);
}

#[tokio::test]
async fn a_raised_cap_lets_the_waiting_job_in_and_the_two_runs_stay_apart() {
    let setup = Setup::new(policy(300, 1)).await;
    let a = setup.pool.start("job-a").await.unwrap();
    let waiting = tokio::spawn({
        let pool = setup.pool.clone();
        async move { pool.start("job-b").await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!waiting.is_finished());

    setup.set_policy(policy(300, 2));
    let b = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .expect("the raised cap did not let the job in")
        .unwrap()
        .unwrap();

    // Two runs: their own ids, folders and DevTools.
    assert_ne!(a.run_id(), b.run_id());
    assert_ne!(a.downloads_dir(), b.downloads_dir());
    assert!(a.downloads_dir().is_dir() && b.downloads_dir().is_dir());
    let pa = a.new_page("about:blank").await.unwrap();
    let pb = b.new_page("about:blank").await.unwrap();
    assert!(setup
        .fake
        .commands(a.run_id())
        .iter()
        .any(|c| c.method == "Target.createTarget"));
    assert_eq!(
        setup
            .fake
            .commands(b.run_id())
            .iter()
            .filter(|c| c.method == "Target.createTarget")
            .count(),
        1
    );
    let _ = (pa, pb);

    // A download of one run reaches that run alone.
    std::fs::write(a.downloads_dir().join("g1"), "from a").unwrap();
    setup.fake.emit(
        a.run_id(),
        json!({"method": "Browser.downloadWillBegin", "params": {"guid": "g1", "url": "https://h.example/f?sig=SECRET", "suggestedFilename": "a.zip"}}),
    );
    setup.fake.emit(
        a.run_id(),
        json!({"method": "Browser.downloadProgress", "params": {"guid": "g1", "state": "completed", "receivedBytes": 6, "totalBytes": 6}}),
    );
    let got = a.download_finished().await.unwrap();
    assert_eq!(got.path, a.downloads_dir().join("g1"));
    let none = tokio::time::timeout(Duration::from_millis(300), b.download_finished()).await;
    assert!(none.is_err(), "the other run saw the download");

    // A third job waits at the cap of two.
    let third = tokio::time::timeout(Duration::from_millis(300), setup.pool.start("job-c")).await;
    assert!(third.is_err());
}

#[tokio::test]
async fn a_job_that_has_a_run_gets_it_back_and_a_new_run_after_it_ends() {
    let setup = Setup::new(policy(300, 2)).await;
    let first = setup.pool.start("job").await.unwrap();
    let again = setup.pool.start("job").await.unwrap();
    assert_eq!(first.run_id(), again.run_id());
    assert_eq!(setup.fake.calls_matching("POST /runs").len(), 1);

    first.end().await;
    let next = setup.pool.start("job").await.unwrap();
    assert_ne!(next.run_id(), first.run_id());
    assert!(first.is_ended() && again.is_ended() && !next.is_ended());
}

#[tokio::test]
async fn an_ended_run_refuses_every_operation_and_cannot_reach_the_next_one() {
    let setup = Setup::new(policy(300, 1)).await;
    let old = setup.pool.start("job").await.unwrap();
    let old_page = old.new_page("about:blank").await.unwrap();
    old.end().await;

    assert!(setup
        .fake
        .calls()
        .contains(&format!("DELETE /runs {}", old.run_id())));
    assert!(
        !old.downloads_dir().exists(),
        "the run's downloads folder stays"
    );

    let ended = |err: BrowserError| matches!(err, BrowserError::RunEnded { ref run } if run == old.run_id());
    assert!(ended(
        old.command("Browser.getVersion", json!({}))
            .await
            .unwrap_err()
    ));
    assert!(ended(old.new_page("about:blank").await.unwrap_err()));
    assert!(ended(old.events().err().unwrap()));
    assert!(ended(old.touch().unwrap_err()));
    assert!(ended(old.mark_busy().unwrap_err()));
    assert!(ended(old.download_finished().await.unwrap_err()));
    assert!(ended(
        old_page.send("Page.reload", json!({})).await.unwrap_err()
    ));
    assert!(ended(
        old_page
            .navigate("https://example.invalid/")
            .await
            .unwrap_err()
    ));
    assert!(ended(old_page.evaluate("1").await.unwrap_err()));

    // The next job's run is its own.
    let next = setup.pool.start("other-job").await.unwrap();
    assert!(ended(
        old.command("Browser.getVersion", json!({}))
            .await
            .unwrap_err()
    ));
    assert!(next.command("Browser.getVersion", json!({})).await.is_ok());
    let sent_to_next = setup.fake.commands(next.run_id());
    assert!(sent_to_next
        .iter()
        .all(|c| c.method != "Browser.getVersion" || c.session.is_none()));
    assert_eq!(
        sent_to_next
            .iter()
            .filter(|c| c.method == "Browser.getVersion")
            .count(),
        1,
        "only the next run's own command reached it"
    );
}

#[tokio::test]
async fn the_reaper_ends_a_run_that_has_been_idle_past_the_idle_time() {
    let setup = Setup::new(policy(300, 1)).await;
    let run = setup.pool.start("job").await.unwrap();

    setup.advance(Duration::from_secs(299));
    assert!(setup.pool.reap_once().await.is_empty());
    assert!(!run.is_ended());

    setup.advance(Duration::from_secs(2));
    assert_eq!(setup.pool.reap_once().await, [run.run_id().to_owned()]);
    assert!(run.is_ended());
    assert!(setup.fake.run_ids().is_empty());
    assert!(setup.pool.status().is_empty());
    // The slot is free again.
    setup.pool.start("job").await.unwrap();
}

#[tokio::test]
async fn the_idle_time_is_read_from_the_policy_each_pass() {
    let setup = Setup::new(policy(3600, 1)).await;
    let run = setup.pool.start("job").await.unwrap();
    setup.advance(Duration::from_secs(120));
    assert!(setup.pool.reap_once().await.is_empty());
    setup.set_policy(policy(60, 1));
    assert_eq!(setup.pool.reap_once().await.len(), 1);
    assert!(run.is_ended());
}

#[tokio::test]
async fn a_running_job_step_keeps_the_run_and_the_idle_time_counts_from_its_end() {
    let setup = Setup::new(policy(300, 1)).await;
    let run = setup.pool.start("job").await.unwrap();

    let guard = run.busy_guard().unwrap();
    setup.advance(Duration::from_secs(3600));
    assert!(setup.pool.reap_once().await.is_empty());
    assert!(run.status().busy);

    drop(guard);
    assert!(!run.status().busy);
    // Not at once: the idle time starts when the step ended.
    assert!(setup.pool.reap_once().await.is_empty());
    setup.advance(Duration::from_secs(299));
    assert!(setup.pool.reap_once().await.is_empty());
    setup.advance(Duration::from_secs(2));
    assert_eq!(setup.pool.reap_once().await.len(), 1);
}

#[tokio::test]
async fn steps_that_overlap_all_have_to_end_before_the_run_is_idle() {
    let setup = Setup::new(policy(60, 1)).await;
    let run = setup.pool.start("job").await.unwrap();
    run.mark_busy().unwrap();
    run.mark_busy().unwrap();
    run.mark_idle();
    setup.advance(Duration::from_secs(600));
    assert!(setup.pool.reap_once().await.is_empty());
    run.mark_idle();
    setup.advance(Duration::from_secs(61));
    assert_eq!(setup.pool.reap_once().await.len(), 1);
}

#[tokio::test]
async fn a_download_under_way_is_not_cut_by_the_idle_time() {
    let setup = Setup::new(policy(300, 1)).await;
    let run = setup.pool.start("job").await.unwrap();
    setup.fake.emit(
        run.run_id(),
        json!({"method": "Browser.downloadWillBegin", "params": {"guid": "g", "url": "https://files.example/x?sig=abc", "suggestedFilename": "big.zip"}}),
    );
    eventually("the download to be seen", || {
        run.status().downloads_in_progress == 1
    })
    .await;

    setup.advance(Duration::from_secs(7200));
    assert!(setup.pool.reap_once().await.is_empty());
    assert!(!run.is_ended());

    // It ends; the idle time counts from there.
    std::fs::write(run.downloads_dir().join("g"), "zip").unwrap();
    setup.fake.emit(
        run.run_id(),
        json!({"method": "Browser.downloadProgress", "params": {"guid": "g", "state": "completed", "receivedBytes": 3, "totalBytes": 3}}),
    );
    eventually("the download to end", || {
        run.status().downloads_in_progress == 0
    })
    .await;
    assert!(setup.pool.reap_once().await.is_empty());
    setup.advance(Duration::from_secs(301));
    assert_eq!(setup.pool.reap_once().await.len(), 1);
}

#[tokio::test]
async fn a_touch_moves_the_idle_time() {
    let setup = Setup::new(policy(300, 1)).await;
    let run = setup.pool.start("job").await.unwrap();
    setup.advance(Duration::from_secs(250));
    run.touch().unwrap();
    setup.advance(Duration::from_secs(250));
    assert!(setup.pool.reap_once().await.is_empty());
    setup.advance(Duration::from_secs(51));
    assert_eq!(setup.pool.reap_once().await.len(), 1);
}

#[tokio::test]
async fn status_queries_start_nothing_and_do_not_move_the_idle_time() {
    let setup = Setup::new(policy(300, 1)).await;
    // Nothing to see, and asking starts nothing.
    assert!(setup.pool.status().is_empty());
    assert!(setup.pool.run_of_job("job").is_none());
    assert_eq!(setup.fake.calls(), ["POST /reset"]);

    let run = setup.pool.start("job").await.unwrap();
    let before = run.status().last_activity;
    let calls = setup.fake.calls().len();

    setup.advance(Duration::from_secs(200));
    let status = setup.pool.status();
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].last_activity, before);
    let found = setup.pool.run_of_job("job").unwrap();
    assert_eq!(found.run_id(), run.run_id());
    assert_eq!(run.status().last_activity, before);
    assert_eq!(
        setup.fake.calls().len(),
        calls,
        "a query reached the launcher"
    );

    // So the run is still reaped at its own time, however much was asked.
    for _ in 0..20 {
        setup.pool.status();
        setup.pool.run_of_job("job");
    }
    setup.advance(Duration::from_secs(101));
    assert_eq!(setup.pool.reap_once().await.len(), 1);
}

#[tokio::test]
async fn a_download_is_moved_into_the_given_folder_by_its_suggested_name() {
    let setup = Setup::new(policy(300, 1)).await;
    let run = setup.pool.start("job").await.unwrap();
    std::fs::write(run.downloads_dir().join(GUID_1), b"PK\x03\x04zip").unwrap();
    setup.fake.emit(
        run.run_id(),
        json!({"method": "Browser.downloadWillBegin", "params": {"guid": GUID_1, "url": "https://drive.example/download?id=1&sig=SECRET", "suggestedFilename": "../자막 01.zip"}}),
    );
    setup.fake.emit(
        run.run_id(),
        json!({"method": "Browser.downloadProgress", "params": {"guid": GUID_1, "state": "completed", "receivedBytes": 7, "totalBytes": 7}}),
    );

    let download = tokio::time::timeout(Duration::from_secs(5), run.download_finished())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(download.state, DownloadState::Completed);
    assert_eq!(download.file_name, "../자막 01.zip");
    assert_eq!(download.host.as_deref(), Some("drive.example"));
    let shown = format!("{download:?}");
    assert!(
        !shown.contains("SECRET") && !shown.contains("sig="),
        "{shown}"
    );

    let receive = setup.dir.path().join("receive/job");
    let moved = run.move_download(&download, &receive).await.unwrap();
    assert_eq!(moved.name, "자막 01.zip");
    assert_eq!(moved.size, 7);
    assert_eq!(moved.path, receive.join("자막 01.zip"));
    assert_eq!(std::fs::read(&moved.path).unwrap(), b"PK\x03\x04zip");
    assert!(!run.downloads_dir().join(GUID_1).exists());
}

#[tokio::test]
async fn a_canceled_download_is_reported_and_not_moved() {
    let setup = Setup::new(policy(300, 1)).await;
    let run = setup.pool.start("job").await.unwrap();
    setup.fake.emit(
        run.run_id(),
        json!({"method": "Browser.downloadWillBegin", "params": {"guid": "g", "url": "https://h.example/x", "suggestedFilename": "x.zip"}}),
    );
    setup.fake.emit(
        run.run_id(),
        json!({"method": "Browser.downloadProgress", "params": {"guid": "g", "state": "canceled", "receivedBytes": 0, "totalBytes": 0}}),
    );
    let download = run.download_finished().await.unwrap();
    assert_eq!(download.state, DownloadState::Canceled);
    let refused = run
        .move_download(&download, &setup.dir.path().join("out"))
        .await;
    assert!(matches!(
        refused,
        Err(BrowserError::DownloadNotCompleted(_))
    ));
    // A canceled download is not busy.
    assert!(!run.status().busy);
}

#[tokio::test]
async fn a_lost_control_connection_ends_the_run() {
    let setup = Setup::new(policy(300, 1)).await;
    let run = setup.pool.start("job").await.unwrap();
    setup.fake.cut_connection(run.run_id());

    eventually("the run to end", || run.is_ended()).await;
    eventually("the launcher to be asked to end it", || {
        setup
            .fake
            .calls()
            .contains(&format!("DELETE /runs {}", run.run_id()))
    })
    .await;
    eventually("the slot to be free", || setup.pool.status().is_empty()).await;
    assert!(matches!(
        run.command("Browser.getVersion", json!({})).await,
        Err(BrowserError::RunEnded { .. })
    ));
}

#[tokio::test]
async fn the_reaper_reconciles_with_the_launcher() {
    let setup = Setup::new(policy(300, 2)).await;
    let kept = setup.pool.start("kept").await.unwrap();
    let lost = setup.pool.start("lost").await.unwrap();

    // The launcher lost one (its Chromium exited) and holds one nobody asked for.
    setup.fake.lose(lost.run_id());
    setup.fake.add_orphan("stray-run");
    setup.pool.reap_once().await;

    assert!(lost.is_ended());
    assert!(!kept.is_ended());
    assert_eq!(setup.fake.run_ids(), [kept.run_id().to_owned()]);
    assert!(setup
        .fake
        .calls()
        .contains(&"DELETE /runs stray-run".to_owned()));
    // What the stray run downloaded is of no job, and goes.
    assert!(!setup.downloads_root.join("stray-run").exists());
}

#[tokio::test]
async fn a_run_made_ready_after_the_launchers_list_was_read_is_not_taken_for_gone() {
    let setup = Setup::new(policy(300, 2)).await;
    let first = setup.pool.start("first").await.unwrap();
    // The launcher reads its runs, then takes its time to answer.
    setup
        .fake
        .state
        .list_delay_ms
        .store(700, std::sync::atomic::Ordering::SeqCst);
    let pass = tokio::spawn({
        let pool = setup.pool.clone();
        async move { pool.reap_once().await }
    });
    eventually("the list to be asked for", || {
        setup.fake.calls_matching("GET /runs").len() == 1
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let second = setup.pool.start("second").await.unwrap();
    setup
        .fake
        .state
        .list_delay_ms
        .store(0, std::sync::atomic::Ordering::SeqCst);
    pass.await.unwrap();

    // The list did not have the second run, which was not there yet.
    assert!(
        !second.is_ended(),
        "a run started during the pass was ended"
    );
    assert!(!first.is_ended());
    assert_eq!(setup.fake.run_ids().len(), 2);
    assert!(!setup
        .fake
        .calls()
        .contains(&format!("DELETE /runs {}", second.run_id())));

    // The next pass knows both.
    setup.pool.reap_once().await;
    assert!(!second.is_ended());
}

#[tokio::test]
async fn a_run_the_launcher_did_not_confirm_gone_keeps_its_slot() {
    let setup = Setup::new(policy(300, 1)).await;
    let run = setup.pool.start("job-a").await.unwrap();
    setup
        .fake
        .state
        .fail_delete
        .store(true, std::sync::atomic::Ordering::SeqCst);
    run.end().await;
    assert!(run.is_ended(), "the handle works no more");

    let next = tokio::time::timeout(Duration::from_millis(400), setup.pool.start("job-b")).await;
    assert!(
        next.is_err(),
        "a second browser started while the first was not confirmed gone"
    );

    setup
        .fake
        .state
        .fail_delete
        .store(false, std::sync::atomic::Ordering::SeqCst);
    setup.pool.reap_once().await;
    let next = tokio::time::timeout(Duration::from_secs(5), setup.pool.start("job-b"))
        .await
        .expect("the slot did not free")
        .unwrap();
    assert_eq!(next.job_id(), "job-b");
}

#[tokio::test]
async fn a_start_the_launcher_refuses_frees_its_slot() {
    let dir = tempfile::tempdir().unwrap();
    let clock = std::sync::Arc::new(std::sync::atomic::AtomicI64::new(0));
    // The launcher's own cap is lower than the policy's.
    let fake = support::fake_launcher(dir.path().to_owned(), 0, clock.clone()).await;
    let policy = std::sync::Arc::new(std::sync::Mutex::new(BrowserPolicy::default()));
    let pool = make_pool(&fake, dir.path(), TOKEN, &clock, &policy).await;

    let refused = pool.start("job").await;
    assert!(
        matches!(refused, Err(BrowserError::Launcher(ref e)) if e.status() == Some(429)),
        "{refused:?}"
    );
    assert!(pool.status().is_empty());
    assert!(!dir.path().join("job").exists());
}

#[tokio::test]
async fn a_wrong_token_is_refused_and_not_echoed() {
    let setup = Setup::with(policy(300, 1), "not-the-token-s3cret", |_| {}).await;
    let refused = setup.pool.start("job").await;
    let Err(err) = refused else {
        panic!("started with a wrong token")
    };
    assert!(
        matches!(&err, BrowserError::Launcher(e) if e.status() == Some(401)),
        "{err:?}"
    );
    assert!(!format!("{err} {err:?}").contains("s3cret"));
    assert!(setup.pool.status().is_empty());
}

#[tokio::test]
async fn shutdown_ends_every_run() {
    let setup = Setup::new(policy(300, 2)).await;
    let a = setup.pool.start("a").await.unwrap();
    let b = setup.pool.start("b").await.unwrap();
    setup.pool.shutdown().await;
    assert!(a.is_ended() && b.is_ended());
    assert!(setup.fake.run_ids().is_empty());
    assert!(setup.pool.status().is_empty());
}

/// A download of `guid` that the browser reports as complete.
async fn completed_download(
    setup: &Setup,
    run: &trss_browser::BrowserRun,
    guid: &str,
) -> trss_browser::Download {
    setup.fake.emit(
        run.run_id(),
        json!({"method": "Browser.downloadWillBegin", "params": {"guid": guid, "url": "https://h.example/f", "suggestedFilename": "x.zip"}}),
    );
    setup.fake.emit(
        run.run_id(),
        json!({"method": "Browser.downloadProgress", "params": {"guid": guid, "state": "completed", "receivedBytes": 4, "totalBytes": 4}}),
    );
    tokio::time::timeout(Duration::from_secs(5), run.download_finished())
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn a_symlink_in_the_browsers_folder_is_not_moved_into_the_receive_area() {
    use std::os::unix::fs::symlink;
    let setup = Setup::new(policy(300, 1)).await;
    let run = setup.pool.start("job").await.unwrap();
    let database = setup.dir.path().join("trss.db");
    std::fs::write(&database, b"the database").unwrap();
    let receive = setup.dir.path().join("receive");

    // The file is a link to a file of the app.
    symlink(&database, run.downloads_dir().join(GUID_1)).unwrap();
    let download = completed_download(&setup, &run, GUID_1).await;
    let refused = run.move_download(&download, &receive).await;
    assert!(
        matches!(refused, Err(BrowserError::UnsafeDownload(_))),
        "{refused:?}"
    );
    assert_eq!(std::fs::read(&database).unwrap(), b"the database");
    assert_eq!(std::fs::read_dir(&receive).map_or(0, |d| d.count()), 0);

    // The run's folder is a link to somewhere else.
    let elsewhere = setup.dir.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::fs::write(elsewhere.join(GUID_2), b"not the run's").unwrap();
    let download = completed_download(&setup, &run, GUID_2).await;
    std::fs::remove_dir_all(run.downloads_dir()).unwrap();
    symlink(&elsewhere, run.downloads_dir()).unwrap();
    let refused = run.move_download(&download, &receive).await;
    assert!(
        matches!(refused, Err(BrowserError::UnsafeDownload(_))),
        "{refused:?}"
    );
    assert!(elsewhere.join(GUID_2).exists());
    assert_eq!(std::fs::read_dir(&receive).map_or(0, |d| d.count()), 0);
}

#[tokio::test]
async fn a_hard_link_and_a_made_up_name_in_the_browsers_folder_are_not_moved() {
    let setup = Setup::new(policy(300, 1)).await;
    let run = setup.pool.start("job").await.unwrap();
    let receive = setup.dir.path().join("receive");

    // A second name for a file that is not the run's.
    let other = setup.dir.path().join("other");
    std::fs::write(&other, b"someone else's").unwrap();
    std::fs::hard_link(&other, run.downloads_dir().join(GUID_1)).unwrap();
    let download = completed_download(&setup, &run, GUID_1).await;
    let refused = run.move_download(&download, &receive).await;
    assert!(
        matches!(refused, Err(BrowserError::UnsafeDownload(_))),
        "{refused:?}"
    );
    assert_eq!(std::fs::read(&other).unwrap(), b"someone else's");

    // A guid that is not a UUID, however the browser got it there: `..` is
    // the run's folder's parent, which is a folder of the app.
    for guid in ["..", "../trss.db", "g1", ""] {
        let made_up = trss_browser::Download {
            guid: guid.to_owned(),
            file_name: "x.zip".to_owned(),
            host: None,
            source: None,
            answer: None,
            state: DownloadState::Completed,
            path: run.downloads_dir().join(guid),
            received_bytes: 1,
        };
        let refused = run.move_download(&made_up, &receive).await;
        assert!(
            matches!(refused, Err(BrowserError::UnsafeDownload(_))),
            "{guid:?}: {refused:?}"
        );
    }
    assert_eq!(std::fs::read_dir(&receive).map_or(0, |d| d.count()), 0);
    assert!(setup.downloads_root.is_dir());
}

#[tokio::test]
async fn asking_for_the_run_of_a_job_counts_as_using_it() {
    let setup = Setup::new(policy(300, 1)).await;
    let run = setup.pool.start("job").await.unwrap();
    setup.advance(Duration::from_secs(250));
    let again = setup.pool.start("job").await.unwrap();
    assert_eq!(again.run_id(), run.run_id());

    // The idle time counts from the second start.
    setup.advance(Duration::from_secs(250));
    assert!(setup.pool.reap_once().await.is_empty());
    setup.advance(Duration::from_secs(51));
    assert_eq!(setup.pool.reap_once().await.len(), 1);
    // And an idle end is not followed by a step on the same handle.
    assert!(matches!(
        run.mark_busy(),
        Err(BrowserError::RunEnded { .. })
    ));
    assert!(run.busy_guard().is_err());
}

#[tokio::test]
async fn a_download_over_the_size_limit_is_canceled_in_the_browser() {
    let setup = Setup::new(policy(300, 1)).await;
    let run = setup.pool.start("job").await.unwrap();
    setup.fake.emit(
        run.run_id(),
        json!({"method": "Browser.downloadWillBegin", "params": {"guid": GUID_1, "url": "https://h.example/big", "suggestedFilename": "big.zip"}}),
    );
    setup.fake.emit(
        run.run_id(),
        json!({"method": "Browser.downloadProgress", "params": {"guid": GUID_1, "state": "inProgress", "receivedBytes": trss_browser::MAX_DOWNLOAD_BYTES + 1, "totalBytes": 0}}),
    );
    eventually("the browser to be told to cancel", || {
        setup
            .fake
            .commands(run.run_id())
            .iter()
            .any(|c| c.method == "Browser.cancelDownload" && c.params["guid"] == GUID_1)
    })
    .await;
    assert!(
        !run.status().busy,
        "a download being canceled keeps the run busy"
    );
}

#[tokio::test]
async fn a_download_that_stalls_is_canceled_and_the_run_can_go_idle() {
    let setup = Setup::new(policy(300, 1)).await;
    let run = setup.pool.start("job").await.unwrap();
    setup.fake.emit(
        run.run_id(),
        json!({"method": "Browser.downloadWillBegin", "params": {"guid": GUID_1, "url": "https://h.example/x", "suggestedFilename": "x.zip"}}),
    );
    eventually("the download to be seen", || {
        run.status().downloads_in_progress == 1
    })
    .await;

    // Quiet for less than the stall time: left alone, and the run is busy.
    setup.advance(trss_browser::DOWNLOAD_STALL - Duration::from_secs(1));
    assert!(setup.pool.reap_once().await.is_empty());
    assert!(!setup
        .fake
        .commands(run.run_id())
        .iter()
        .any(|c| c.method == "Browser.cancelDownload"));

    // Quiet for the stall time: canceled, and it no longer holds the run.
    setup.advance(Duration::from_secs(1));
    assert!(setup.pool.reap_once().await.is_empty());
    assert!(setup
        .fake
        .commands(run.run_id())
        .iter()
        .any(|c| c.method == "Browser.cancelDownload" && c.params["guid"] == GUID_1));
    assert!(!run.status().busy);
    assert_eq!(run.status().downloads_in_progress, 0);

    // The idle time counts from the cancellation.
    setup.advance(Duration::from_secs(299));
    assert!(setup.pool.reap_once().await.is_empty());
    setup.advance(Duration::from_secs(2));
    assert_eq!(setup.pool.reap_once().await.len(), 1);
}

#[tokio::test]
async fn a_download_that_keeps_reporting_is_not_a_stall() {
    let setup = Setup::new(policy(300, 1)).await;
    let run = setup.pool.start("job").await.unwrap();
    setup.fake.emit(
        run.run_id(),
        json!({"method": "Browser.downloadWillBegin", "params": {"guid": GUID_1, "url": "https://h.example/x", "suggestedFilename": "x.zip"}}),
    );
    for received in [1_000, 2_000, 3_000] {
        setup.advance(trss_browser::DOWNLOAD_STALL - Duration::from_secs(10));
        setup.fake.emit(
            run.run_id(),
            json!({"method": "Browser.downloadProgress", "params": {"guid": GUID_1, "state": "inProgress", "receivedBytes": received, "totalBytes": 0}}),
        );
        // The pass must see the report before it looks.
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(setup.pool.reap_once().await.is_empty());
    }
    assert!(run.status().busy);
    assert!(!setup
        .fake
        .commands(run.run_id())
        .iter()
        .any(|c| c.method == "Browser.cancelDownload"));
}

#[test]
fn the_pool_configuration_does_not_show_the_token() {
    let config = trss_browser::PoolConfig::new(
        "http://trss-browser:9230".parse().unwrap(),
        "s3cret-token",
        "/data/browser-downloads",
    );
    let shown = format!("{config:?} {config:#?}");
    assert!(!shown.contains("s3cret"), "{shown}");
    assert!(shown.contains("trss-browser"));
}

#[tokio::test]
async fn a_persons_input_reported_from_outside_counts_as_use_and_a_watched_screen_does_not() {
    use std::sync::{
        atomic::{AtomicI64, Ordering},
        Arc, Mutex,
    };
    let dir = tempfile::tempdir().unwrap();
    let downloads = dir.path().join("browser-downloads");
    std::fs::create_dir_all(&downloads).unwrap();
    let clock = Arc::new(AtomicI64::new(1_000_000));
    let fake = support::fake_launcher(downloads.clone(), 4, clock.clone()).await;
    // What the web wrote: (run id, when a person last gave input).
    let inputs: Arc<Mutex<Vec<(String, i64)>>> = Arc::default();
    let mut config = trss_browser::PoolConfig::new(fake.url.clone(), TOKEN, &downloads)
        .with_activity(trss_browser::ActivitySource::new({
            let inputs = inputs.clone();
            move || {
                let inputs = inputs.lock().unwrap().clone();
                Box::pin(async move { inputs })
            }
        }));
    config.slot_recheck = Duration::from_millis(100);
    let pool = trss_browser::BrowserPool::new(
        config,
        Arc::new({
            let clock = clock.clone();
            move || clock.load(Ordering::SeqCst)
        }),
        trss_browser::PolicySource::fixed(policy(300, 2)),
    )
    .await
    .unwrap();
    let advance = |secs: i64| clock.fetch_add(secs * 1000, Ordering::SeqCst);
    let used = pool.start("used").await.unwrap();
    let watched = pool.start("watched").await.unwrap();

    // Both screens are open; frames go to them all along. Only the first one
    // gets input, 250 seconds in, as the web reports it.
    advance(250);
    inputs
        .lock()
        .unwrap()
        .push((used.run_id().to_owned(), clock.load(Ordering::SeqCst)));
    // An input of a run the pool does not have changes nothing.
    inputs
        .lock()
        .unwrap()
        .push(("gone-000000000000".to_owned(), clock.load(Ordering::SeqCst)));
    advance(51);
    // The watched one is idle past its time; the used one counts from the input.
    assert_eq!(pool.reap_once().await, [watched.run_id().to_owned()]);
    assert!(watched.is_ended() && !used.is_ended());
    advance(248);
    assert!(pool.reap_once().await.is_empty());
    advance(2);
    assert_eq!(pool.reap_once().await, [used.run_id().to_owned()]);

    // An input reported for a time after the pass is taken as now, so a clock
    // ahead does not keep a run for longer than the idle time.
    let next = pool.start("used").await.unwrap();
    inputs.lock().unwrap().clear();
    inputs.lock().unwrap().push((
        next.run_id().to_owned(),
        clock.load(Ordering::SeqCst) + 3_600_000,
    ));
    assert!(pool.reap_once().await.is_empty());
    inputs.lock().unwrap().clear();
    advance(301);
    assert_eq!(pool.reap_once().await, [next.run_id().to_owned()]);
}

#[test]
fn a_downloads_folder_this_process_owns_is_opened_to_the_browser() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("browser-downloads");
    // Missing: made. With another mode: opened up. With the mode: left.
    trss_browser::prepare_downloads_root(&folder).unwrap();
    assert_eq!(
        folder.metadata().unwrap().permissions().mode() & 0o7777,
        0o1777
    );
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
    trss_browser::prepare_downloads_root(&folder).unwrap();
    assert_eq!(
        folder.metadata().unwrap().permissions().mode() & 0o7777,
        0o1777
    );
    trss_browser::prepare_downloads_root(&folder).unwrap();
}

#[test]
fn a_downloads_folder_of_another_user_is_reported_with_the_fix_even_with_the_mode() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    // `/tmp` is root's and has mode 1777, but this process could not remove
    // the browser's run folders in it under the sticky bit.
    let err = trss_browser::prepare_downloads_root(std::path::Path::new("/tmp")).unwrap_err();
    assert!(err.contains("/tmp"), "{err}");
    assert!(err.contains("uid 0"), "{err}");
    assert!(err.contains("chown"), "{err}");
    assert!(err.contains("chmod 1777"), "{err}");
}

#[test]
fn a_downloads_folder_of_another_user_with_the_wrong_mode_is_reported_not_half_done() {
    if rustix::process::geteuid().is_root() {
        return;
    }
    // `/usr` is root's with mode 755: this process cannot open it up.
    let err = trss_browser::prepare_downloads_root(std::path::Path::new("/usr")).unwrap_err();
    assert!(err.contains("/usr"), "{err}");
    assert!(err.contains("mode 1777"), "{err}");
    assert!(err.contains("uid 0"), "{err}");
    assert_eq!(
        std::fs::metadata("/usr").unwrap().permissions().mode() & 0o7777,
        0o755
    );
}
