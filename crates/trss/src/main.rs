use std::{io, path::PathBuf, process::ExitCode};

use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use trss::{
    wake::Wakes,
    web::{
        self as web,
        browser_net::{self, GuardedListener},
        env::WebEnv,
        screen_api::RemoteScreens,
        AppState,
    },
    worker::{env::FAKE_SUBTITLE_SOURCE_VAR, Worker, WorkerEnv},
};
use trss_anilist::AnilistConfig;
use trss_anissia::{Anissia, AnissiaConfig};
use trss_collect::{
    anissia::{captions::CaptionObserver, AnissiaQueue},
    store::anissia::AnissiaStore,
};
use trss_core::{access::check_app_data, db::DB_PATH_ENV, lock_path_for, Db, LockFile};
use trss_jobs::{place::unpack, JobRun, ReceiveArea, Runner, Unpacker};
use trss_library::{
    artwork::{AppData, Artwork},
    seasons::Seasons,
};
use trss_subtitles::{
    auth::BrowserAuth, blogger::BloggerSource, drive::Drive, erulabo::ErulaboSource,
    fake::FakeSource, naver::NaverSource, tistory::TistorySource, winpng::BrowserReader, Sources,
};

#[tokio::main]
async fn main() -> ExitCode {
    dotenv::dotenv().ok();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("trss: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Serves the screens and the API, and runs the collection cycles, the
/// commands the web accepts and the subtitle jobs beside them, until SIGTERM
/// or SIGINT. The web stores the commands and the jobs in the database and
/// rings the worker part's loops, which start them at once ([`trss::wake`]).
async fn run() -> Result<(), String> {
    let worker_env = WorkerEnv::from_env().map_err(|e| e.to_string())?;
    let web_env = WebEnv::from_env().map_err(|e| e.to_string())?;
    let anilist = AnilistConfig::from_env()?;
    let anissia_config = AnissiaConfig::from_env()?;
    let db_path: PathBuf = std::env::var_os(DB_PATH_ENV)
        .ok_or_else(|| format!("environment variable {DB_PATH_ENV} is not set"))?
        .into();

    let index = web_env.static_dir.join("index.html");
    if !index.is_file() {
        return Err(format!(
            "no frontend build at {} (set {} to the directory produced by `bun run build` in web/)",
            web_env.static_dir.display(),
            web::env::STATIC_DIR_VAR,
        ));
    }

    // Before anything is opened: a file an earlier deploy made as another user
    // stops the start here, with its path and the uid that is needed.
    check_app_data(&db_path).map_err(|e| e.to_string())?;
    let open = || async {
        Db::open(&db_path).await.map_err(|e| {
            format!(
                "cannot open the app database (set {DB_PATH_ENV} to a file on a local volume): {e}"
            )
        })
    };
    let db = open().await?;
    // The web's own connection, as when it was a process of its own: the
    // screens read beside a cycle's long write instead of waiting for it.
    let web_db = open().await?;
    // Cover images live in the app data folder: the database's folder.
    let app_data = AppData::for_database(&db_path);
    remove_old_wake_socket(&db_path);

    // From here on SIGTERM and SIGINT stop the process the orderly way, the
    // waits of the start included.
    let cancel = CancellationToken::new();
    tokio::spawn(shutdown_on_signal(cancel.clone()));

    // The web first, so the screens answer while the worker part connects to
    // the server browser.
    //
    // With the server browser, the web is on its network: every connection
    // from there is refused, and a web that cannot tell which addresses those
    // are does not start.
    let refused = match &web_env.browser {
        Some(access) => {
            let Some(found) = cancel
                .run_until_cancelled(browser_net::find(&access.url))
                .await
            else {
                println!("trss stopped");
                return Ok(());
            };
            let subnet = found.map_err(|e| {
                format!("will not start without refusing the server browser's network: {e}")
            })?;
            println!("trss refuses connections from the server browser's network {subnet}");
            Some(subnet)
        }
        None => None,
    };
    let listener = TcpListener::bind(web_env.addr)
        .await
        .map_err(|e| format!("cannot listen on {}: {e}", web_env.addr))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    println!(
        "trss listening on http://{addr} (static files: {})",
        web_env.static_dir.display()
    );

    let receive = ReceiveArea::in_app_data(app_data.root());
    // What the web stores for the worker part starts at once: the web rings
    // the loop that runs it.
    let wakes = Wakes::default();
    let state = AppState::new(web_db.clone())
        .with_artwork(Artwork::new(
            web_db.clone(),
            Some(app_data.clone()),
            anilist.clone(),
        ))
        .with_anissia(Anissia::with_defaults(web_db, anissia_config.clone()))
        .with_receive_area(&receive)
        .with_wakes(wakes.clone())
        .with_web_hosts(web_env.hosts.clone());
    let state = match &web_env.browser {
        Some(access) => state.with_remote_screens(RemoteScreens::new(access)?),
        None => state,
    };
    // What a killed process, or an upload cut short in this one, left in the
    // receive area (a staging folder, or the folder of a job that was never
    // recorded). The web takes the uploads, so it sweeps: at start and every
    // hour. Anything an upload or a receipt going on owns is younger than the
    // hour, or has a record.
    let sweeping = state.uploads.keep_sweeping(
        std::time::Duration::from_secs(3600),
        std::time::Duration::from_secs(3600),
        |result| match result {
            Ok(removed) => println!("trss: removed {removed} abandoned upload folders"),
            Err(e) => eprintln!("trss: cannot sweep the receive area: {e}"),
        },
    );
    let router = web::router(&web_env.static_dir, state);
    let serving = tokio::spawn({
        let cancel = cancel.clone();
        let listener = GuardedListener::new(listener, refused);
        async move {
            // A web that stops, or panics, stops the rest: the process is one
            // unit.
            let _stop_all = cancel.clone().drop_guard();
            axum::serve(listener, router)
                .with_graceful_shutdown(cancel.cancelled_owned())
                .await
                .map_err(|e| e.to_string())
        }
    });

    // The real sites are always on; the fake one only when asked for. One
    // Drive serves every source that links Drive files, so their requests to
    // Drive's hosts are spaced together. erulabo's files come only through
    // the site's check in the server browser: without one its posts wait.
    // Its Drive is only asked for a `HEAD` of the files it came from (the
    // daily recheck's observation).
    let drive = Drive::new();
    let mut sources = Sources::none()
        .with_tistory(TistorySource::new(drive.clone()))
        .with_blogger(BloggerSource::new(drive.clone()))
        .with_naver(NaverSource::new(drive.clone()))
        .with_erulabo(ErulaboSource::new(drive));
    if std::env::var(FAKE_SUBTITLE_SOURCE_VAR).as_deref() == Ok("1") {
        println!("The fake subtitle source is on ({FAKE_SUBTITLE_SOURCE_VAR})");
        sources = sources.with_fake(FakeSource);
    }
    // One worker at a time uses the browser container: a second one would
    // reset it and end the first one's runs. The lock lives as long as this
    // process (`browser_lock`).
    let (browser, browser_lock) = match &worker_env.browser {
        Some(browser_env) => match trss::worker::browser::take_lock(&db_path)
            .map_err(|e| format!("cannot take the lock of the server browser: {e}"))?
        {
            Some(lock) => match trss_browser::prepare_downloads_root(&browser_env.downloads) {
                Ok(()) => {
                    let connecting = trss::worker::browser::connect(
                        db.clone(),
                        browser_env,
                        trss_core::system_clock(),
                    );
                    let Some(connected) = cancel.run_until_cancelled(connecting).await else {
                        // The web is winding down with the same token.
                        sweeping.abort();
                        let served = serving.await.map_err(|e| e.to_string())?;
                        println!("trss stopped");
                        return served;
                    };
                    (Some(connected?), Some(lock))
                }
                // A first start has a downloads folder Docker made as root.
                // That is no reason to stop the whole process (its restart
                // policy would loop on it): it runs as it did without a
                // browser, and the message says what to give the folder.
                Err(reason) => {
                    eprintln!("trss: running without a server browser: {reason}");
                    // Keep the lock: while this worker holds it no other one
                    // uses the browser, so the screens bound to an earlier
                    // worker's runs are no one's and are closed at the start.
                    (None, Some(lock))
                }
            },
            None => {
                eprintln!(
                    "trss: another worker uses the browser container; this one runs without a server browser"
                );
                (None, None)
            }
        },
        None => (None, None),
    };
    let jobs = Runner::new(
        JobRun::new(db.clone()),
        sources,
        ReceiveArea::in_app_data(app_data.root()),
        trss_core::system_clock(),
    );
    // The posts whose subtitle is in WinPNG images are read by the server
    // browser, and the posts whose file comes after a person's check on the
    // site are brought to that check in it; a worker without one leaves them
    // waiting.
    let jobs = match &browser {
        Some(pool) => jobs
            .with_winpng(BrowserReader::shared(pool.clone()))
            .with_auth(BrowserAuth::shared(pool.clone())),
        None => jobs,
    };
    // Received archives are unpacked by the program beside this one, in a
    // process of its own with limits; without it they wait for a build that
    // has it.
    let extract = std::env::current_exe().map(|exe| exe.with_file_name(unpack::PROGRAM));
    let jobs = match extract {
        Ok(program) if program.is_file() => {
            println!("Received archives are unpacked by {}", program.display());
            jobs.with_unpacker(Unpacker::new(program))
        }
        _ => {
            println!(
                "Received archives wait: {} is not beside this program",
                unpack::PROGRAM
            );
            jobs
        }
    };
    let artwork = Artwork::new(db.clone(), Some(app_data), anilist);
    let season_info = Seasons::over(db.clone(), &artwork);
    let anissia_client = Anissia::with_defaults(db.clone(), anissia_config);
    let anissia = AnissiaQueue::new(anissia_client.clone(), AnissiaStore::new(db.clone()));
    let captions = CaptionObserver::new(anissia_client, AnissiaStore::new(db.clone()));
    let mut worker = Worker::new(db, &worker_env, lock_path_for(&db_path))
        .map_err(|e| e.to_string())?
        .with_captions(captions.clone())
        .with_jobs(jobs)
        .with_season_info(season_info.stored())
        .with_wakes(wakes);
    if let Some(pool) = browser {
        println!("The server browser is on (a pool over the browser container)");
        worker = worker.with_browser(pool);
    } else if browser_lock.is_some() {
        worker = worker.with_unused_browser_lock();
    }

    // Work covers: AniList searches and image fetches, one at a time, beside
    // the collection loop and outside its lock.
    let queue = tokio::spawn({
        let cancel = cancel.clone();
        let lock = LockFile::Artwork.path_for(&db_path);
        async move { artwork.run_queue(lock, cancel).await }
    });

    // Season info: linking a first season by its folder name and the daily
    // refresh of entries that are not finished, on the same pace.
    let season_queue = tokio::spawn({
        let cancel = cancel.clone();
        let lock = LockFile::Seasons.path_for(&db_path);
        async move { season_info.run_queue(lock, cancel).await }
    });

    // Subscribed anime: the daily refresh of their schedule snapshots, on
    // Anissia's own pace.
    let anissia_queue = tokio::spawn({
        let cancel = cancel.clone();
        let lock = LockFile::Anissia.path_for(&db_path);
        async move { anissia.run_queue(lock, cancel).await }
    });

    // The subtitle lines of every Anissia anime: the recent list every 30
    // minutes, whatever the collection cycle is doing, under its own lock.
    let caption_queue = tokio::spawn({
        let cancel = cancel.clone();
        let lock = LockFile::AnissiaCaptions.path_for(&db_path);
        async move { captions.run_queue(lock, cancel).await }
    });

    println!(
        "trss started: a cycle every {}s (database: {})",
        worker_env.interval.as_secs(),
        db_path.display()
    );

    worker.run(cancel).await;
    let _ = queue.await;
    let _ = season_queue.await;
    let _ = anissia_queue.await;
    let _ = caption_queue.await;
    let served = serving.await.map_err(|e| e.to_string())?;
    sweeping.abort();

    println!("trss stopped");
    served
}

/// Removes the socket next to the database on which releases before one
/// process heard the web's wake-ups (`<database>.wake`): nothing listens on it
/// now.
fn remove_old_wake_socket(db_path: &std::path::Path) {
    let mut socket = db_path.as_os_str().to_owned();
    socket.push(".wake");
    let socket = PathBuf::from(socket);
    match std::fs::remove_file(&socket) {
        Ok(()) => println!(
            "trss: removed the wake socket of an earlier release ({})",
            socket.display()
        ),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => eprintln!(
            "trss: cannot remove the wake socket of an earlier release ({}): {e}",
            socket.display()
        ),
    }
}

/// Cancels `cancel` on SIGTERM or SIGINT, so a cycle in progress can wind down
/// and the web stops taking connections.
async fn shutdown_on_signal(cancel: CancellationToken) {
    let mut sigterm = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        Ok(signal) => signal,
        Err(err) => {
            eprintln!("trss: cannot listen for SIGTERM: {err}");
            return;
        }
    };

    tokio::select! {
        _ = sigterm.recv() => println!("SIGTERM received, shutting down"),
        _ = tokio::signal::ctrl_c() => println!("SIGINT received, shutting down"),
    }
    cancel.cancel();
}
