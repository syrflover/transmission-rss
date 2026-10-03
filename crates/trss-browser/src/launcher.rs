//! The launcher (`trss-browserd`): what runs in the browser container.
//!
//! The container is always up. It runs a virtual display and this launcher
//! ([`xvfb`] keeps the display; [`api`] is the HTTP side). Chromium exists
//! only while a *run* does: the worker asks the launcher over the private
//! compose network to start a run, drives it through the DevTools socket the
//! launcher proxies, and asks it to end the run. The launcher starts each
//! Chromium windowed on the display with a profile of its own under
//! [`Config::runs_dir`] and deletes the profile when the run ends, so no
//! cookie or site storage outlives a run. There is no Docker socket anywhere:
//! starting and ending a browser is this process's job inside its own
//! container.
//!
//! The DevTools port of each Chromium listens on loopback of the container
//! only. The one way to reach it is the proxy at `GET /runs/{id}/cdp`, behind
//! the token ([`api`]).
//!
//! A run is over when the worker ends it, when [`Launcher::reset`] ends all
//! of them, or when its Chromium exits by itself. In each case the proxied
//! sockets close, the process group is stopped (SIGTERM, then SIGKILL after
//! [`Config::kill_grace`]), and the profile and the downloads folder are
//! removed. The worker moves what it wants out of the downloads folder while
//! the run lives.
//!
//! A start goes on in a task of its own, so a request that is dropped (the
//! worker gave up waiting) cannot leave a Chromium or its folders behind
//! halfway; ending the run, or a reset, makes a start under way give up and
//! is answered when it has.

pub mod api;
mod devtools;
pub mod xvfb;

use std::{
    collections::HashMap,
    io,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rustix::process::{
    kill_process_group, test_kill_process, test_kill_process_group, Pid, Signal,
};
use tokio::process::{Child, Command};
use tokio_util::sync::CancellationToken;

use crate::protocol::{is_valid_run_id, RunInfo};

pub const TOKEN_VAR: &str = "TRSS_BROWSER_TOKEN";
pub const BIND_VAR: &str = "TRSS_BROWSER_BIND";
pub const MAX_RUNS_VAR: &str = "TRSS_BROWSER_MAX_RUNS";
pub const CHROMIUM_VAR: &str = "TRSS_BROWSER_CHROMIUM";
pub const CHROMIUM_ARGS_VAR: &str = "TRSS_BROWSER_CHROMIUM_ARGS";
pub const RUNS_DIR_VAR: &str = "TRSS_BROWSER_RUNS_DIR";
pub const DOWNLOADS_VAR: &str = "TRSS_BROWSER_DOWNLOADS";
pub const DISPLAY_VAR: &str = "TRSS_BROWSER_DISPLAY";

pub const DEFAULT_BIND: &str = "0.0.0.0:9230";
pub const DEFAULT_MAX_RUNS: usize = 2;
/// The size of the virtual display.
pub const SCREEN: &str = "1920x1200x24";

/// How the launcher is set up.
#[derive(Clone)]
pub struct Config {
    /// What every request must bring as `Authorization: Bearer <token>`.
    pub token: String,
    /// Where the HTTP API listens (the binary's; tests bind their own).
    pub bind: SocketAddr,
    /// The Chromium executable.
    pub chromium: PathBuf,
    /// Arguments put before the launcher's own. The image sets `--no-sandbox`:
    /// the container is the sandbox, and Docker's default seccomp profile
    /// keeps Chromium's own from starting.
    pub chromium_args: Vec<String>,
    /// Where each run's profile is made, and removed from.
    pub runs_dir: PathBuf,
    /// The folder shared with the worker; a run's downloads go in a folder of
    /// its id inside.
    pub downloads_dir: PathBuf,
    /// The X display Chromium opens on (`:99`).
    pub display: String,
    /// The display's socket. When set, a start waits for it to exist.
    pub display_socket: Option<PathBuf>,
    /// How many runs may be alive at once.
    pub max_runs: usize,
    /// How long a start waits for Chromium's DevTools to answer.
    pub ready_timeout: Duration,
    /// How long Chromium has to exit on SIGTERM before SIGKILL.
    pub kill_grace: Duration,
}

impl std::fmt::Debug for Config {
    /// Without the token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("bind", &self.bind)
            .field("chromium", &self.chromium)
            .field("chromium_args", &self.chromium_args)
            .field("runs_dir", &self.runs_dir)
            .field("downloads_dir", &self.downloads_dir)
            .field("display", &self.display)
            .field("max_runs", &self.max_runs)
            .finish_non_exhaustive()
    }
}

/// A bad setting. Messages name the variable, never its value.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error(
        "environment variable {TOKEN_VAR} is not set: the launcher does not start without a token"
    )]
    MissingToken,
    #[error("environment variable {0} has an invalid value")]
    Invalid(&'static str),
}

impl Config {
    /// The defaults, for `token`.
    pub fn new(token: impl Into<String>) -> Config {
        Config {
            token: token.into(),
            bind: DEFAULT_BIND.parse().expect("a socket address"),
            chromium: PathBuf::from("chromium"),
            chromium_args: Vec::new(),
            runs_dir: PathBuf::from("/tmp/trss-runs"),
            downloads_dir: PathBuf::from("/downloads"),
            display: ":99".to_owned(),
            display_socket: None,
            max_runs: DEFAULT_MAX_RUNS,
            ready_timeout: Duration::from_secs(30),
            kill_grace: Duration::from_secs(5),
        }
    }

    pub fn from_env() -> Result<Config, ConfigError> {
        Config::from_lookup(|key| std::env::var(key).ok())
    }

    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Config, ConfigError> {
        let token = get(TOKEN_VAR)
            .filter(|t| !t.trim().is_empty())
            .ok_or(ConfigError::MissingToken)?;
        let mut config = Config::new(token);
        if let Some(bind) = get(BIND_VAR) {
            config.bind = bind.parse().map_err(|_| ConfigError::Invalid(BIND_VAR))?;
        }
        if let Some(max) = get(MAX_RUNS_VAR) {
            config.max_runs = max
                .parse()
                .ok()
                .filter(|n| *n > 0)
                .ok_or(ConfigError::Invalid(MAX_RUNS_VAR))?;
        }
        if let Some(chromium) = get(CHROMIUM_VAR).filter(|c| !c.is_empty()) {
            config.chromium = chromium.into();
        }
        if let Some(args) = get(CHROMIUM_ARGS_VAR) {
            config.chromium_args = args.split_whitespace().map(str::to_owned).collect();
        }
        if let Some(dir) = get(RUNS_DIR_VAR).filter(|d| !d.is_empty()) {
            config.runs_dir = dir.into();
        }
        if let Some(dir) = get(DOWNLOADS_VAR).filter(|d| !d.is_empty()) {
            config.downloads_dir = dir.into();
        }
        if let Some(display) = get(DISPLAY_VAR).filter(|d| !d.is_empty()) {
            config.display = display;
        }
        config.display_socket =
            Some(xvfb::socket_path(&config.display).ok_or(ConfigError::Invalid(DISPLAY_VAR))?);
        Ok(config)
    }
}

/// Why a start was refused.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("the run id is not valid")]
    InvalidId,
    #[error("{0} runs are alive already")]
    TooManyRuns(usize),
    #[error("a run with that id is being ended")]
    Ending,
    /// An end (or a reset) came for the run while it was being started, or
    /// the start another request began for it did not complete.
    #[error("the start was cancelled or did not complete")]
    Aborted,
    #[error("the display is not up")]
    DisplayDown,
    #[error("Chromium did not start: {0}")]
    Spawn(String),
    #[error("Chromium exited before its DevTools answered ({0})")]
    Exited(String),
    #[error("Chromium's DevTools did not answer in time")]
    NotReady,
    #[error("cannot prepare the run's folders: {0}")]
    Io(#[from] io::Error),
}

/// A run: one Chromium with its profile.
pub struct Run {
    pub id: String,
    /// Chromium's process id, which is also its process group.
    pub pid: u32,
    /// Unix milliseconds.
    pub started_at: i64,
    pub profile: PathBuf,
    /// Where Chromium saves downloads, in this container.
    pub downloads: PathBuf,
    /// The browser-level DevTools socket, on loopback.
    pub(crate) ws_url: String,
    /// Cancelled when the run is over (or on its way out): proxies close.
    pub(crate) ended: CancellationToken,
    /// Cancelled to ask the supervisor to end the run.
    stop: CancellationToken,
    /// Cancelled when the process group is gone and the profile removed.
    done: CancellationToken,
}

impl Run {
    pub fn info(&self) -> RunInfo {
        RunInfo {
            run: self.id.clone(),
            started_at: self.started_at,
            pid_alive: pid(self.pid).is_some_and(|p| test_kill_process(p).is_ok()),
            downloads: self.downloads.to_string_lossy().into_owned(),
        }
    }

    /// Whether the run is being or has been ended.
    pub fn is_ending(&self) -> bool {
        self.stop.is_cancelled() || self.ended.is_cancelled()
    }
}

struct Inner {
    config: Config,
    runs: Mutex<HashMap<String, Arc<Run>>>,
    /// One start (or reset) at a time, so the cap is counted exactly.
    start_lock: tokio::sync::Mutex<()>,
    /// The starts under way (those queued for `start_lock` included), by run
    /// id: an end or a reset finds them here.
    starting: Mutex<HashMap<String, Starting>>,
}

/// A start under way.
struct Starting {
    /// Cancelled by an end of the run, or a reset: the start gives up.
    cancel: CancellationToken,
    /// Cancelled when the start is over, whatever its outcome: a run it made
    /// is in the table by then.
    done: CancellationToken,
}

/// Takes a start out of `Inner::starting` when its task is over, in any way.
struct StartGuard {
    inner: Arc<Inner>,
    id: String,
    done: CancellationToken,
}

impl Drop for StartGuard {
    fn drop(&mut self) {
        self.inner
            .starting
            .lock()
            .expect("starting lock")
            .remove(&self.id);
        self.done.cancel();
    }
}

/// The launcher's state: the live runs. Cheap to clone.
#[derive(Clone)]
pub struct Launcher {
    inner: Arc<Inner>,
}

fn pid(raw: u32) -> Option<Pid> {
    i32::try_from(raw).ok().and_then(Pid::from_raw)
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

fn free_port() -> io::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

impl Launcher {
    /// A launcher over `config`. The folders it needs are made, and what an
    /// earlier launcher left in the runs folder is removed: no process of
    /// that one is alive in this container.
    pub async fn open(config: Config) -> io::Result<Launcher> {
        tokio::fs::create_dir_all(&config.runs_dir).await?;
        let launcher = Launcher {
            inner: Arc::new(Inner {
                config,
                runs: Mutex::default(),
                start_lock: tokio::sync::Mutex::new(()),
                starting: Mutex::default(),
            }),
        };
        launcher.clear_runs_dir().await;
        Ok(launcher)
    }

    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    async fn clear_runs_dir(&self) {
        let Ok(mut entries) = tokio::fs::read_dir(&self.inner.config.runs_dir).await else {
            return;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            remove_path(&entry.path()).await;
        }
    }

    /// The live run `id`; `None` for an unknown one and for one on its way out.
    pub fn get(&self, id: &str) -> Option<Arc<Run>> {
        let runs = self.inner.runs.lock().expect("runs lock");
        runs.get(id).filter(|r| !r.is_ending()).cloned()
    }

    /// The live runs, oldest first. Changes nothing.
    pub fn list(&self) -> Vec<RunInfo> {
        let runs = self.inner.runs.lock().expect("runs lock");
        let mut list: Vec<RunInfo> = runs
            .values()
            .filter(|r| !r.is_ending())
            .map(|r| r.info())
            .collect();
        list.sort_by(|a, b| (a.started_at, &a.run).cmp(&(b.started_at, &b.run)));
        list
    }

    /// Starts the run `id`, or answers the one that exists.
    ///
    /// The start goes on in a task of its own, so a caller that gives up (the
    /// worker's request timing out) cannot cut it halfway and leave a
    /// Chromium and its folders behind. What the caller can do is
    /// [`Launcher::end`] the run, which makes a start under way give up and
    /// waits until it has.
    pub async fn start(&self, id: &str) -> Result<Arc<Run>, StartError> {
        if !is_valid_run_id(id) {
            return Err(StartError::InvalidId);
        }
        let registered = {
            let mut starting = self.inner.starting.lock().expect("starting lock");
            match starting.get(id) {
                Some(other) => Err(other.done.clone()),
                None => {
                    let (cancel, done) = (CancellationToken::new(), CancellationToken::new());
                    starting.insert(
                        id.to_owned(),
                        Starting {
                            cancel: cancel.clone(),
                            done: done.clone(),
                        },
                    );
                    Ok((cancel, done))
                }
            }
        };
        let (cancel, done) = match registered {
            Ok(registered) => registered,
            Err(other) => {
                // Another request is starting this run: its outcome is ours.
                other.cancelled().await;
                let run = self.inner.runs.lock().expect("runs lock").get(id).cloned();
                return match run {
                    Some(run) if run.is_ending() => Err(StartError::Ending),
                    Some(run) => Ok(run),
                    None => Err(StartError::Aborted),
                };
            }
        };
        let guard = StartGuard {
            inner: self.inner.clone(),
            id: id.to_owned(),
            done,
        };
        let launcher = self.clone();
        let id = id.to_owned();
        let task = tokio::spawn(async move {
            let _guard = guard;
            launcher.start_registered(&id, cancel).await
        });
        match task.await {
            Ok(result) => result,
            Err(err) => Err(StartError::Spawn(format!("the start task failed: {err}"))),
        }
    }

    async fn start_registered(
        &self,
        id: &str,
        cancel: CancellationToken,
    ) -> Result<Arc<Run>, StartError> {
        let config = &self.inner.config;
        let _one_start = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(StartError::Aborted),
            lock = self.inner.start_lock.lock() => lock,
        };
        if cancel.is_cancelled() {
            return Err(StartError::Aborted);
        }
        {
            let runs = self.inner.runs.lock().expect("runs lock");
            if let Some(run) = runs.get(id) {
                return if run.is_ending() {
                    Err(StartError::Ending)
                } else {
                    Ok(run.clone())
                };
            }
            if runs.len() >= config.max_runs {
                return Err(StartError::TooManyRuns(config.max_runs));
            }
        }
        if let Some(socket) = &config.display_socket {
            tokio::select! {
                shown = self.wait_for_display(socket) => shown?,
                _ = cancel.cancelled() => return Err(StartError::Aborted),
            }
        }

        let profile = config.runs_dir.join(id);
        let downloads = config.downloads_dir.join(id);
        let prepared = self.prepare_folders(&profile, &downloads).await;
        if let Err(err) = prepared {
            self.discard(&profile, &downloads).await;
            return Err(err.into());
        }
        let port = match free_port() {
            Ok(port) => port,
            Err(err) => {
                self.discard(&profile, &downloads).await;
                return Err(err.into());
            }
        };
        let mut child = match self.spawn_chromium(&profile, port) {
            Ok(child) => child,
            Err(err) => {
                self.discard(&profile, &downloads).await;
                return Err(StartError::Spawn(err.to_string()));
            }
        };
        let pid_raw = child.id().unwrap_or(0);
        let ready = tokio::select! {
            ready = self.wait_ready(&mut child, port) => ready,
            _ = cancel.cancelled() => Err(StartError::Aborted),
        };
        let ws_url = match ready {
            Ok(url) => url,
            Err(err) => {
                terminate(&mut child, pid(pid_raw), config.kill_grace).await;
                self.discard(&profile, &downloads).await;
                return Err(err);
            }
        };

        let run = Arc::new(Run {
            id: id.to_owned(),
            pid: pid_raw,
            started_at: now_millis(),
            profile,
            downloads,
            ws_url,
            ended: CancellationToken::new(),
            stop: CancellationToken::new(),
            done: CancellationToken::new(),
        });
        self.inner
            .runs
            .lock()
            .expect("runs lock")
            .insert(id.to_owned(), run.clone());
        tokio::spawn(supervise(self.inner.clone(), run.clone(), child));
        Ok(run)
    }

    async fn prepare_folders(&self, profile: &Path, downloads: &Path) -> io::Result<()> {
        // A folder of an earlier run of the same id has nothing to give.
        remove_path(profile).await;
        tokio::fs::create_dir_all(profile).await?;
        tokio::fs::create_dir_all(downloads).await?;
        // The worker moves the files out, and may run as another user.
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(downloads, std::fs::Permissions::from_mode(0o777)).await
    }

    async fn discard(&self, profile: &Path, downloads: &Path) {
        remove_path(profile).await;
        remove_path(downloads).await;
    }

    async fn wait_for_display(&self, socket: &Path) -> Result<(), StartError> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !socket.exists() {
            if Instant::now() >= deadline {
                return Err(StartError::DisplayDown);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }

    fn spawn_chromium(&self, profile: &Path, port: u16) -> io::Result<Child> {
        self.chromium_command(profile, port).spawn()
    }

    /// The command that starts Chromium for a run. The page code of a site
    /// runs in Chromium, so what the launcher is configured with (its token
    /// above all) is not passed on in the environment.
    fn chromium_command(&self, profile: &Path, port: u16) -> Command {
        let config = &self.inner.config;
        let mut command = Command::new(&config.chromium);
        command
            .env_remove(TOKEN_VAR)
            .args(&config.chromium_args)
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg(format!("--remote-debugging-port={port}"))
            .args([
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-dev-shm-usage",
                "--window-size=1280,800",
                "--force-device-scale-factor=1",
                "about:blank",
            ])
            .env("DISPLAY", &config.display)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // Its own process group, so that the whole tree can be stopped.
            .process_group(0)
            .kill_on_drop(true);
        command
    }

    async fn wait_ready(&self, child: &mut Child, port: u16) -> Result<String, StartError> {
        let deadline = Instant::now() + self.inner.config.ready_timeout;
        loop {
            if let Some(status) = child.try_wait()? {
                return Err(StartError::Exited(status.to_string()));
            }
            if let Some(path) = devtools::browser_socket_path(port).await {
                return Ok(format!("ws://127.0.0.1:{port}{path}"));
            }
            if Instant::now() >= deadline {
                return Err(StartError::NotReady);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Ends the run `id` and waits until its process group is gone and its
    /// profile and downloads folder removed. A start of the run under way is
    /// cancelled and waited for first, so the run it would have made does not
    /// outlive this call. An unknown id has nothing left behind either.
    pub async fn end(&self, id: &str) {
        if !is_valid_run_id(id) {
            return;
        }
        let starting = self
            .inner
            .starting
            .lock()
            .expect("starting lock")
            .get(id)
            .map(|s| (s.cancel.clone(), s.done.clone()));
        if let Some((cancel, done)) = starting {
            cancel.cancel();
            done.cancelled().await;
        }
        let run = self.inner.runs.lock().expect("runs lock").get(id).cloned();
        match run {
            Some(run) => {
                run.stop.cancel();
                run.done.cancelled().await;
            }
            None => {
                self.discard(
                    &self.inner.config.runs_dir.join(id),
                    &self.inner.config.downloads_dir.join(id),
                )
                .await
            }
        }
    }

    /// Ends every run and removes every profile. No start runs meanwhile;
    /// the ones under way give up.
    pub async fn reset(&self) {
        let starting: Vec<CancellationToken> = self
            .inner
            .starting
            .lock()
            .expect("starting lock")
            .values()
            .map(|s| s.cancel.clone())
            .collect();
        for cancel in starting {
            cancel.cancel();
        }
        let _no_start = self.inner.start_lock.lock().await;
        let runs: Vec<Arc<Run>> = self
            .inner
            .runs
            .lock()
            .expect("runs lock")
            .values()
            .cloned()
            .collect();
        for run in &runs {
            run.stop.cancel();
        }
        for run in &runs {
            run.done.cancelled().await;
        }
        self.clear_runs_dir().await;
    }
}

/// Owns the process: waits until it exits by itself or the run is asked to
/// end, then closes the proxies, stops what is left of the group and removes
/// the profile.
async fn supervise(inner: Arc<Inner>, run: Arc<Run>, mut child: Child) {
    tokio::select! {
        _ = child.wait() => println!("Browser run {}: Chromium exited by itself", run.id),
        _ = run.stop.cancelled() => {}
    }
    run.ended.cancel();
    terminate(&mut child, pid(run.pid), inner.config.kill_grace).await;
    remove_path(&run.profile).await;
    // What the worker did not move out is of no use to anyone now, and must
    // not pile up in the shared disk.
    remove_path(&run.downloads).await;
    inner.runs.lock().expect("runs lock").remove(&run.id);
    run.done.cancel();
}

/// Stops the process group `group` led by `child`: SIGTERM, then SIGKILL for
/// what is still there `grace` later. Returns when the leader is reaped and
/// the group is empty (or, for a group that will not die, after a last wait).
async fn terminate(child: &mut Child, group: Option<Pid>, grace: Duration) {
    let Some(group) = group else {
        let _ = child.kill().await;
        return;
    };
    let _ = kill_process_group(group, Signal::TERM);
    let deadline = Instant::now() + grace;
    if tokio::time::timeout(grace, child.wait()).await.is_err() {
        let _ = kill_process_group(group, Signal::KILL);
        let _ = child.wait().await;
    }
    // The leader is gone; its children usually follow it at once.
    while test_kill_process_group(group).is_ok() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if test_kill_process_group(group).is_ok() {
        let _ = kill_process_group(group, Signal::KILL);
        let last = Instant::now() + Duration::from_secs(2);
        while test_kill_process_group(group).is_ok() && Instant::now() < last {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// Removes a file or folder tree, trying again once for a process that is
/// still writing into it. A missing path is not an error.
async fn remove_path(path: &Path) {
    for attempt in 0..3 {
        let removed = match tokio::fs::symlink_metadata(path).await {
            Ok(meta) if meta.is_dir() => tokio::fs::remove_dir_all(path).await,
            Ok(_) => tokio::fs::remove_file(path).await,
            Err(_) => return,
        };
        match removed {
            Ok(()) => return,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return,
            Err(err) if attempt == 2 => {
                eprintln!("Browser launcher: cannot remove {}: {err}", path.display())
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    #[test]
    fn it_does_not_start_without_a_token() {
        assert_eq!(
            Config::from_lookup(lookup(&[])).unwrap_err(),
            ConfigError::MissingToken
        );
        assert_eq!(
            Config::from_lookup(lookup(&[(TOKEN_VAR, "  ")])).unwrap_err(),
            ConfigError::MissingToken
        );
    }

    #[test]
    fn the_defaults_and_the_overrides() {
        let config = Config::from_lookup(lookup(&[(TOKEN_VAR, "t")])).unwrap();
        assert_eq!(config.max_runs, 2);
        assert_eq!(config.bind.port(), 9230);
        assert_eq!(config.display, ":99");
        assert_eq!(
            config.display_socket.as_deref(),
            Some(Path::new("/tmp/.X11-unix/X99"))
        );

        let config = Config::from_lookup(lookup(&[
            (TOKEN_VAR, "t"),
            (MAX_RUNS_VAR, "3"),
            (CHROMIUM_ARGS_VAR, "--no-sandbox  --lang=ko"),
        ]))
        .unwrap();
        assert_eq!(config.max_runs, 3);
        assert_eq!(config.chromium_args, ["--no-sandbox", "--lang=ko"]);

        assert_eq!(
            Config::from_lookup(lookup(&[(TOKEN_VAR, "t"), (MAX_RUNS_VAR, "0")])).unwrap_err(),
            ConfigError::Invalid(MAX_RUNS_VAR)
        );
    }

    #[test]
    fn chromium_does_not_get_the_launchers_token() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::new("t");
        config.runs_dir = dir.path().join("runs");
        let launcher = Launcher {
            inner: Arc::new(Inner {
                config,
                runs: Mutex::default(),
                start_lock: tokio::sync::Mutex::new(()),
                starting: Mutex::default(),
            }),
        };
        let command = launcher.chromium_command(Path::new("/profile"), 9222);
        let envs: HashMap<_, _> = command.as_std().get_envs().collect();
        // `None` is a variable taken out of the environment.
        assert_eq!(envs.get(std::ffi::OsStr::new(TOKEN_VAR)), Some(&None));
        assert!(envs
            .get(std::ffi::OsStr::new("DISPLAY"))
            .is_some_and(Option::is_some));
    }

    #[test]
    fn debug_output_has_no_token() {
        let shown = format!("{:?}", Config::new("s3cret-token"));
        assert!(!shown.contains("s3cret"), "{shown}");
    }
}
