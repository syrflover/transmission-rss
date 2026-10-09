//! One remote screen of a browser run: the web's own DevTools connection to
//! the page that shows a job's check, shared by every socket open on it (see
//! the protocol in [`super`]). The browser draws only the tab in front, so
//! the hub brings its page there when it opens: a page a popup was opened in
//! front of (a person who went back to the earlier tab) would send no frames.
//!
//! # Frames after a change of size
//!
//! A change of size stops the screencast, lays the page out anew and starts
//! the screencast again; the frames of the old layout must not be sent as
//! the new generation's. The browser sends the answers and the events of one
//! connection in order, and the connection numbers its events in that order
//! and tells how many came before an answer
//! ([`Connection::command_marked`]). So the frames sent before the answer to
//! the new `Page.startScreencast` are of the old layout, and those after it
//! of the new one. While the size changes the pump (which alone reads the
//! events and sends frames) acknowledges every frame and keeps the newest
//! one aside. Once the start is answered, the pump is told its mark
//! ([`Barrier`]): it reads the events that came meanwhile in the same way,
//! sends the new size, lets the frames through again, and sends the frame it
//! kept if that came after the answer (the page may draw nothing more for a
//! while). A frame of another size than the device's is dropped as well,
//! whenever it comes.
//!
//! A mouse button or a touch held when the size starts to change is let go
//! on the page first (`mouseReleased` with no click, `touchCancel`): its
//! release, made on the old frames, would be dropped. An input is admitted,
//! sent and recorded as held under the same lock ([`Hub::input`]) under which
//! a change of size moves the generation on and lets go: an input either
//! went to the page before the change (and is let go by it) or is judged
//! against the new generation (and dropped).
//!
//! # What the screen says about its pages
//!
//! Besides frames, a hub tells its sockets the state of the page shown (the
//! back and forward buttons, the host: the `nav` message) and the run's tabs
//! (the `tabs` message), both built in [`super::nav`] and sent only when they
//! change, and again to a socket that connects or skipped messages. They are
//! read again when the browser says a page navigated or a target changed
//! (the events are told to the refresher, which reads at most every
//! [`REFRESH_GAP`]) and when the worker's list of the run's pages changes.
//! Back and forward are judged here against the page's own history each time
//! (a step back never goes before the first page that is not blank), whatever
//! a socket believes.
//!
//! # A page that does not answer
//!
//! An input, a reload or a change of size that the page does not answer
//! within [`Times::answer_within`] does not end the hub: the page is
//! stalled. The sockets are told (`{"type":"page","responding":false}`),
//! and the inputs that come are not sent: they would wait in the browser and
//! reach the page all at once when it moves again. A size asked for meanwhile
//! is kept for later. The prober asks the page for a trifle
//! (`Runtime.evaluate`) every [`Times::probe_every`]; once it answers, the
//! sockets are told (`responding: true`) and the last size is applied anew,
//! which starts the screencast again as a new generation (a change of size
//! may have stopped halfway). Only a connection or a page that is gone ends
//! the hub (`browser`). A page that does not answer `Page.enable` within
//! [`Times::open_within`] has no hub at all ([`OpenError::Stuck`]).
//!
//! # Dialogs
//!
//! The page's dialogs (`alert`, `confirm`, `prompt`, `beforeunload`) are
//! not drawn in the frames, and with `Page.enable` on, the browser leaves
//! them to the hub: one no one answers holds the page until the run ends,
//! even after the hub's connection closed. So the hub tells its sockets the
//! dialog the page shows and answers it as a person chooses
//! ([`Hub::answer_dialog`]). While it shows, inputs, reloads and steps are
//! not sent (the page takes none), a size is kept until it closed, and then
//! what a press that opened it holds down is let go. An
//! input that opened it (a click on a button that asks to confirm) is
//! answered only once it closed: its wait ends when the dialog opens, and
//! neither that wait nor the dialog is a page that does not answer (a
//! stall ends when one opens, and none starts while it shows). A person's
//! answer the page refuses means it shows none: a dialog whose close the hub
//! missed is closed then, so it does not hold the screen. A `beforeunload`
//! within [`LEAVE_WITHIN`] of a person's reload or step is theirs: the hub
//! answers it `leave` and does not send it. A hub that goes while a dialog
//! shows dismisses it first (an `alert` is closed, a `beforeunload` stays).
//!
//! # Seats
//!
//! At most [`MAX_SOCKETS`] sockets are open on a hub. A new one always gets
//! a seat: when they are all taken, the oldest socket's seat is given to it
//! and that socket ends (`replaced`). The newest socket is the device the
//! person is using; the oldest may be a phone that slept with the screen
//! open and left a dead connection behind.

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{
    broadcast::{
        self,
        error::{RecvError, TryRecvError},
    },
    mpsc, oneshot, watch, Notify,
};
use tokio_util::sync::{CancellationToken, DropGuard};
use trss_browser::cdp::{CdpError, Connection, Event};
use trss_jobs::ScreenStore;

use super::nav::{host_of, tabs_message, History};
use crate::commands_api::now_millis;

/// The times a hub keeps (see the module docs and `super`).
#[derive(Debug, Clone, Copy)]
pub struct Times {
    /// How often a person's input is recorded for the worker's idle end.
    pub input_every: Duration,
    /// How often the hub reads whether its binding is still the job's.
    pub check_every: Duration,
    /// How long a command to the page may wait for its answer before the
    /// page is taken as stalled.
    pub answer_within: Duration,
    /// How long `Page.enable` may wait when the hub opens.
    pub open_within: Duration,
    /// How often a stalled page is asked whether it answers again.
    pub probe_every: Duration,
}

/// Why no hub could be opened on a page.
#[derive(Debug)]
pub enum OpenError {
    /// The page did not answer within [`Times::open_within`].
    Stuck,
    /// The run or its page could not be reached.
    Unreachable(String),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::Stuck => f.write_str("the page does not answer"),
            OpenError::Unreachable(why) => f.write_str(why),
        }
    }
}

/// Why a command to the page had no answer.
#[derive(Debug)]
enum Fail {
    /// The connection or the page is gone.
    Gone(String),
    /// No answer within [`Times::answer_within`].
    Late(String),
    /// The browser refused it.
    Refused(String),
}

impl From<CdpError> for Fail {
    fn from(err: CdpError) -> Fail {
        match err {
            CdpError::Timeout(_) => Fail::Late(err.to_string()),
            CdpError::Command { .. } => Fail::Refused(err.to_string()),
            CdpError::Closed | CdpError::Connect(_) | CdpError::NotFound => {
                Fail::Gone(err.to_string())
            }
        }
    }
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fail::Gone(why) | Fail::Late(why) | Fail::Refused(why) => f.write_str(why),
        }
    }
}

/// How many frames and notes wait for a slow socket before it skips some.
const OUT_BACKLOG: usize = 8;
/// JPEG quality of the frames.
const FRAME_QUALITY: u32 = 70;
/// How far a frame's size may be from the device's and still be its (the
/// browser rounds).
const SIZE_SLACK: f64 = 1.5;
/// The least time between two readings of the page's history and the run's
/// targets, however many events ask for one.
const REFRESH_GAP: Duration = Duration::from_millis(150);
/// How long after a person's reload or step a `beforeunload` dialog is
/// taken as theirs and answered `leave` (see the module docs).
const LEAVE_WITHIN: Duration = Duration::from_secs(3);
/// The most characters of a dialog's message that are sent.
const DIALOG_MESSAGE: usize = 1000;
/// The most characters of a prompt's text, sent or answered.
const DIALOG_TEXT: usize = 2000;
/// How long the dismissal of a dialog no one answers may take when its hub
/// goes.
const DISMISS_WITHIN: Duration = Duration::from_secs(2);

/// What a device reports when it opens the screen or changes its size.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    pub dpr: f64,
    pub touch: bool,
}

impl Viewport {
    fn valid(&self) -> bool {
        (200..=4096).contains(&self.width)
            && (200..=4096).contains(&self.height)
            && self.dpr.is_finite()
            && (0.5..=4.0).contains(&self.dpr)
    }
}

/// The screen as the inputs are judged against it.
#[derive(Debug, Clone, Copy, Default)]
pub struct View {
    /// Bumped by every change of size.
    pub gen: u64,
    /// A change of size is under way.
    pub resizing: bool,
    pub viewport: Option<Viewport>,
}

/// Whether an input made on generation `gen` may go to the page: only one
/// made on the frames of the current size, and not while the size changes.
pub fn admits(view: &View, gen: u64) -> bool {
    !view.resizing && view.gen == gen
}

/// A message of a socket (see the protocol in [`super`]).
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Incoming {
    Viewport(Viewport),
    Mouse {
        gen: u64,
        event: String,
        x: f64,
        y: f64,
        #[serde(default)]
        button: Option<String>,
        #[serde(default)]
        buttons: Option<u32>,
        #[serde(default, rename = "clickCount")]
        click_count: Option<u32>,
        #[serde(default)]
        modifiers: Option<u32>,
        #[serde(default, rename = "deltaX")]
        delta_x: Option<f64>,
        #[serde(default, rename = "deltaY")]
        delta_y: Option<f64>,
    },
    Touch {
        gen: u64,
        event: String,
        points: Vec<TouchPoint>,
        #[serde(default)]
        modifiers: Option<u32>,
    },
    Key {
        gen: u64,
        event: String,
        #[serde(default)]
        key: Option<String>,
        #[serde(default)]
        code: Option<String>,
        #[serde(default)]
        text: Option<String>,
        #[serde(default, rename = "keyCode")]
        key_code: Option<u32>,
        #[serde(default)]
        modifiers: Option<u32>,
    },
    Text {
        gen: u64,
        text: String,
    },
    Reload,
    Back,
    Forward,
    /// A person's answer to the dialog `id` the page shows: `accept` (`확인`,
    /// `떠나기`) or not, and for a prompt the text.
    Dialog {
        id: u64,
        accept: bool,
        #[serde(default)]
        text: Option<String>,
    },
}

impl Incoming {
    /// A move of the pointer or of a finger, or a turn of the wheel: one of
    /// a stream in which the next says where the pointer is as well. The
    /// only messages a socket lets go of when they pile up: a press, a
    /// release, a key, a size, a reload or a step that is lost would leave
    /// the page or the screen in another state than the device's.
    pub fn is_motion(&self) -> bool {
        match self {
            Incoming::Mouse { event, .. } => event == "mouseMoved" || event == "mouseWheel",
            Incoming::Touch { event, .. } => event == "touchMove",
            _ => false,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct TouchPoint {
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub id: Option<u32>,
}

/// What became of a socket's message.
#[derive(Debug, PartialEq)]
pub enum Handled {
    /// Applied (or nothing to do).
    Done,
    /// An input of another generation, or made during a resize: dropped.
    Dropped { gen: u64 },
    /// Not a message the protocol knows, or out of its bounds.
    Invalid,
}

/// The DevTools command an input is, when it is within bounds.
fn command_of(message: &Incoming, view: &View) -> Option<(&'static str, Value)> {
    let on_screen = |x: f64, y: f64| {
        x.is_finite()
            && y.is_finite()
            && x >= 0.0
            && y >= 0.0
            && view
                .viewport
                .is_none_or(|v| x <= f64::from(v.width) && y <= f64::from(v.height))
    };
    let modifiers = |m: &Option<u32>| m.unwrap_or(0).min(15);
    let short = |s: &Option<String>| s.as_ref().is_none_or(|s| s.chars().count() <= 32);
    match message {
        Incoming::Mouse {
            event,
            x,
            y,
            button,
            buttons,
            click_count,
            modifiers: m,
            delta_x,
            delta_y,
            ..
        } => {
            let event = match event.as_str() {
                e @ ("mousePressed" | "mouseReleased" | "mouseMoved" | "mouseWheel") => e,
                _ => return None,
            };
            let button = match button.as_deref().unwrap_or("none") {
                b @ ("none" | "left" | "middle" | "right" | "back" | "forward") => b,
                _ => return None,
            };
            let delta = |d: &Option<f64>| d.unwrap_or(0.0);
            if !on_screen(*x, *y)
                || !delta(delta_x).is_finite()
                || !delta(delta_y).is_finite()
                || click_count.unwrap_or(0) > 3
            {
                return None;
            }
            let mut params = json!({
                "type": event, "x": x, "y": y, "button": button,
                "buttons": buttons.unwrap_or(0).min(31),
                "clickCount": click_count.unwrap_or(0),
                "modifiers": modifiers(m),
            });
            if event == "mouseWheel" {
                params["deltaX"] = json!(delta(delta_x));
                params["deltaY"] = json!(delta(delta_y));
            }
            Some(("Input.dispatchMouseEvent", params))
        }
        Incoming::Touch {
            event,
            points,
            modifiers: m,
            ..
        } => {
            let event = match event.as_str() {
                e @ ("touchStart" | "touchMove" | "touchEnd" | "touchCancel") => e,
                _ => return None,
            };
            if points.len() > 10 || points.iter().any(|p| !on_screen(p.x, p.y)) {
                return None;
            }
            let points: Vec<Value> = points
                .iter()
                .enumerate()
                .map(|(n, p)| json!({ "x": p.x, "y": p.y, "id": p.id.unwrap_or(n as u32).min(1000) }))
                .collect();
            Some((
                "Input.dispatchTouchEvent",
                json!({ "type": event, "touchPoints": points, "modifiers": modifiers(m) }),
            ))
        }
        Incoming::Key {
            event,
            key,
            code,
            text,
            key_code,
            modifiers: m,
            ..
        } => {
            let event = match event.as_str() {
                e @ ("keyDown" | "keyUp" | "rawKeyDown" | "char") => e,
                _ => return None,
            };
            if !short(key) || !short(code) || !short(text) || key_code.unwrap_or(0) > 255 {
                return None;
            }
            let mut params = json!({ "type": event, "modifiers": modifiers(m) });
            for (name, value) in [("key", key), ("code", code), ("text", text)] {
                if let Some(value) = value {
                    params[name] = json!(value);
                }
            }
            if let Some(code) = key_code {
                params["windowsVirtualKeyCode"] = json!(code);
            }
            Some(("Input.dispatchKeyEvent", params))
        }
        Incoming::Text { text, .. } => {
            let length = text.chars().count();
            (length > 0 && length <= 2000).then(|| ("Input.insertText", json!({ "text": text })))
        }
        Incoming::Viewport(_)
        | Incoming::Reload
        | Incoming::Back
        | Incoming::Forward
        | Incoming::Dialog { .. } => None,
    }
}

/// The binding a hub shows: the run, its page and when it was bound to the
/// job. Another check of the job in the same run is another binding. It is
/// the job's seat ([`trss_jobs::screen::Screen::seat`]), which says whether it
/// is still the job's.
pub use trss_jobs::screen::Seat as Binding;

/// The most sockets open on one hub; one more takes the oldest one's seat
/// (`replaced`).
pub const MAX_SOCKETS: usize = 4;

/// A mouse button or a touch the page holds down.
#[derive(Debug, Default)]
struct Held {
    mouse: Option<(String, f64, f64)>,
    /// Fingers are on the page: the last touch event sent left points.
    touch: bool,
    /// How many dialogs the page had shown when the button, or the first
    /// finger, went down: a dialog that closed lets go only of what went
    /// down before it opened.
    mouse_from: u64,
    touch_from: u64,
}

/// The sockets seated on a hub, oldest first.
#[derive(Default)]
struct Seats {
    next: u64,
    open: VecDeque<(u64, CancellationToken)>,
}

/// A change of size the pump finishes (see the module docs): `viewport` is
/// the message of the new size, `after` how many events came before the new
/// screencast's start was answered.
struct Barrier {
    viewport: Arc<str>,
    after: u64,
    done: oneshot::Sender<()>,
}

/// A dialog the page shows (see the module docs).
#[derive(Debug, Clone, Copy)]
struct Shown {
    id: u64,
    kind: &'static str,
    /// A `beforeunload` the hub answers itself (`leave`): never sent to the
    /// sockets.
    theirs: bool,
}

/// `text` cut to its first `most` characters.
fn cut(text: &str, most: usize) -> String {
    text.chars().take(most).collect()
}

/// The remote screen of one run's page.
pub struct Hub {
    job: String,
    binding: Binding,
    conn: Connection,
    session: String,
    view: Mutex<View>,
    /// One change of size at a time.
    resize: tokio::sync::Mutex<()>,
    barriers: mpsc::UnboundedSender<Barrier>,
    out: broadcast::Sender<Arc<str>>,
    screens: ScreenStore,
    times: Times,
    last_input: Mutex<Option<Instant>>,
    /// The page did not answer a command in time and has not answered the
    /// prober since (see the module docs).
    stalled: AtomicBool,
    /// Told when the page stalls: the prober starts asking.
    stalls: Arc<Notify>,
    /// Taken to admit, send and record an input, and to move the generation
    /// on and let go of what is held (see the module docs).
    input: tokio::sync::Mutex<Held>,
    seats: Mutex<Seats>,
    /// Cancelled when the page or the connection is gone, or the binding
    /// changed; `reason` says which (`browser`, `run`).
    ended: CancellationToken,
    reason: Mutex<&'static str>,
    /// The dialog the page shows: inputs, reloads, steps and changes of
    /// size wait while it does (see the module docs).
    dialog: watch::Sender<Option<Shown>>,
    /// The last dialog's number.
    dialogs: AtomicU64,
    /// When a person last reloaded the page or stepped in its history.
    left_at: Mutex<Option<Instant>>,
    /// The dialog a person's answer is on its way to (0 when none): one
    /// answer to a dialog at a time.
    answering: AtomicU64,
    /// A size was kept while the page could not take it (stalled, or a
    /// dialog showed): it is applied once the page can.
    size_waits: AtomicBool,
    /// The `nav`, `tabs`, `page` and `dialog` messages as last sent.
    nav: Mutex<Option<Arc<str>>>,
    tabs: Mutex<Option<Arc<str>>>,
    page: Mutex<Option<Arc<str>>>,
    dialog_message: Mutex<Option<Arc<str>>>,
    /// The pages the worker listed when the hub last read the screen.
    listed: Mutex<Vec<String>>,
    /// Told that `nav` or `tabs` may be out of date.
    changed: Arc<Notify>,
    /// One reading of the page and the targets at a time.
    refresh: tokio::sync::Mutex<()>,
    _stop_tasks: DropGuard,
}

/// A socket's place on a hub, given back when it is dropped.
pub struct Seat {
    hub: Arc<Hub>,
    id: u64,
    replaced: CancellationToken,
}

impl Seat {
    /// Resolves when a newer socket took this seat.
    pub async fn replaced(&self) {
        self.replaced.cancelled().await
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        let mut seats = self.hub.seats.lock().expect("seats lock");
        seats.open.retain(|(id, _)| *id != self.id);
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        // A dialog no one answers stays on the page and holds it until the
        // run ends: it is dismissed first (see the module docs).
        let shows = self.dialog.borrow().is_some();
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) if shows => {
                let (conn, session) = (self.conn.clone(), self.session.clone());
                runtime.spawn(async move {
                    let _ = conn
                        .command_within(
                            Some(&session),
                            "Page.handleJavaScriptDialog",
                            json!({ "accept": false }),
                            DISMISS_WITHIN,
                        )
                        .await;
                    conn.close();
                });
            }
            _ => self.conn.close(),
        }
    }
}

impl Hub {
    /// Attaches to the page of `binding` through the launcher's DevTools
    /// proxy at `cdp_url` and starts sending its frames. Every
    /// [`Times::check_every`] it reads whether the binding is still the
    /// job's, and ends (`run`) when it is not.
    pub async fn open(
        cdp_url: &str,
        token: &str,
        job: &str,
        binding: Binding,
        screens: ScreenStore,
        times: Times,
    ) -> Result<Arc<Hub>, OpenError> {
        let unreachable = |what: &str, e: CdpError| OpenError::Unreachable(format!("{what}: {e}"));
        let conn = Connection::connect(cdp_url, token)
            .await
            .map_err(|e| unreachable("connect", e))?;
        let events = conn.events();
        let attached = conn
            .command(
                None,
                "Target.attachToTarget",
                json!({ "targetId": binding.target, "flatten": true }),
            )
            .await
            .map_err(|e| unreachable("attach", e))?;
        let session = attached["sessionId"]
            .as_str()
            .ok_or_else(|| OpenError::Unreachable("attach: no session".to_owned()))?
            .to_owned();
        // The page answers this itself: one that is held (a dialog no one
        // answers, a script that never yields) does not.
        match conn
            .command_within(Some(&session), "Page.enable", json!({}), times.open_within)
            .await
        {
            Ok(_) => {}
            Err(CdpError::Timeout(_)) => return Err(OpenError::Stuck),
            Err(e) => return Err(unreachable("Page.enable", e)),
        }
        // The browser draws only the tab in front: a page another one was
        // opened in front of (a person who went back to an earlier tab)
        // sends no frames until it is brought there.
        let _ = conn
            .command_within(
                Some(&session),
                "Page.bringToFront",
                json!({}),
                times.answer_within,
            )
            .await;
        // The tabs follow the run's other targets: their titles and
        // addresses change as they load. Without it they are read when the
        // worker's list or the page changes.
        let _ = conn
            .command(
                None,
                "Target.setDiscoverTargets",
                json!({ "discover": true }),
            )
            .await;
        let stop = CancellationToken::new();
        let (out, _) = broadcast::channel(OUT_BACKLOG);
        let (barriers, barrier_feed) = mpsc::unbounded_channel();
        let hub = Arc::new(Hub {
            job: job.to_owned(),
            binding,
            conn,
            session,
            view: Mutex::default(),
            resize: tokio::sync::Mutex::new(()),
            barriers,
            out,
            screens,
            times,
            last_input: Mutex::new(None),
            stalled: AtomicBool::new(false),
            stalls: Arc::new(Notify::new()),
            input: tokio::sync::Mutex::default(),
            seats: Mutex::default(),
            ended: CancellationToken::new(),
            reason: Mutex::new("browser"),
            dialog: watch::Sender::new(None),
            dialogs: AtomicU64::new(0),
            left_at: Mutex::new(None),
            answering: AtomicU64::new(0),
            size_waits: AtomicBool::new(false),
            nav: Mutex::default(),
            tabs: Mutex::default(),
            page: Mutex::default(),
            dialog_message: Mutex::default(),
            listed: Mutex::default(),
            changed: Arc::new(Notify::new()),
            refresh: tokio::sync::Mutex::new(()),
            _stop_tasks: stop.clone().drop_guard(),
        });
        tokio::spawn(pump(
            Arc::downgrade(&hub),
            hub.conn.clone(),
            events,
            barrier_feed,
            stop.clone(),
        ));
        tokio::spawn(refresher(Arc::downgrade(&hub), stop.clone()));
        tokio::spawn(prober(Arc::downgrade(&hub), stop.clone()));
        tokio::spawn(watch_binding(Arc::downgrade(&hub), times.check_every, stop));
        // Before the first frame: a socket that connects finds the state.
        hub.read_state().await;
        match hub.start_screencast(None).await {
            Ok(_) => Ok(hub),
            Err(Fail::Late(_)) => Err(OpenError::Stuck),
            Err(e) => Err(OpenError::Unreachable(format!("screencast: {e}"))),
        }
    }

    /// A place for one more socket. When [`MAX_SOCKETS`] are taken, the
    /// oldest is given up: its socket is told through [`Seat::replaced`].
    pub fn seat(self: &Arc<Self>) -> Seat {
        let mut seats = self.seats.lock().expect("seats lock");
        while seats.open.len() >= MAX_SOCKETS {
            if let Some((_, oldest)) = seats.open.pop_front() {
                oldest.cancel();
            }
        }
        let id = seats.next;
        seats.next += 1;
        let replaced = CancellationToken::new();
        seats.open.push_back((id, replaced.clone()));
        Seat {
            hub: self.clone(),
            id,
            replaced,
        }
    }

    pub fn view(&self) -> View {
        *self.view.lock().expect("view lock")
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<str>> {
        self.out.subscribe()
    }

    /// The `nav`, `tabs`, `page` and `dialog` messages as last sent, for a
    /// socket that connects or skipped some.
    pub fn states(&self) -> Vec<Arc<str>> {
        [&self.nav, &self.tabs, &self.page, &self.dialog_message]
            .into_iter()
            .filter_map(|slot| slot.lock().expect("state lock").clone())
            .collect()
    }

    pub fn is_ended(&self) -> bool {
        self.ended.is_cancelled()
    }

    pub async fn ended(&self) {
        self.ended.cancelled().await
    }

    /// Why the hub ended: `browser` or `run`.
    pub fn end_reason(&self) -> &'static str {
        *self.reason.lock().expect("reason lock")
    }

    /// The page or the connection is gone: every socket of the hub ends.
    fn end(&self) {
        self.end_because("browser");
    }

    /// Every socket of the hub ends, for `reason` unless it had ended.
    fn end_because(&self, reason: &'static str) {
        let mut why = self.reason.lock().expect("reason lock");
        if !self.ended.is_cancelled() {
            *why = reason;
            self.ended.cancel();
        }
    }

    /// A command in the page's session, waiting for its answer for as long
    /// as [`Times::answer_within`].
    async fn send(&self, method: &str, params: Value) -> Result<Value, Fail> {
        Ok(self
            .conn
            .command_within(
                Some(&self.session),
                method,
                params,
                self.times.answer_within,
            )
            .await?)
    }

    /// Starts the screencast; how many events came before the start was
    /// answered.
    async fn start_screencast(&self, viewport: Option<Viewport>) -> Result<u64, Fail> {
        let mut params = json!({ "format": "jpeg", "quality": FRAME_QUALITY, "everyNthFrame": 1 });
        if let Some(v) = viewport {
            params["maxWidth"] = json!(v.width);
            params["maxHeight"] = json!(v.height);
        }
        Ok(self
            .conn
            .command_marked_within(
                Some(&self.session),
                "Page.startScreencast",
                params,
                self.times.answer_within,
            )
            .await
            .map(|(_, after)| after)?)
    }

    pub fn is_stalled(&self) -> bool {
        self.stalled.load(Ordering::SeqCst)
    }

    /// What a command to the page that was not answered means: a page or
    /// connection that is gone ends the hub, one that did not answer in time
    /// is stalled (see the module docs), and a refusal is that command's
    /// alone.
    fn failed(&self, fail: &Fail) {
        match fail {
            Fail::Gone(_) => self.end(),
            Fail::Late(_) => self.stall(),
            Fail::Refused(_) => {}
        }
    }

    /// An input to the page: [`Hub::send`], but the wait ends when the page
    /// shows a dialog. The input went to the page, which answers it only
    /// once the dialog closes (a click that opened it): it is not a page
    /// that does not answer (see the module docs).
    async fn send_input(&self, method: &str, params: Value) -> Result<Value, Fail> {
        let mut dialog = self.dialog.subscribe();
        tokio::select! {
            // First, so that the input is sent before the dialog is looked at.
            biased;
            answer = self.send(method, params) => answer,
            _ = dialog.wait_for(Option::is_some) => Ok(Value::Null),
        }
    }

    pub fn shows_dialog(&self) -> bool {
        self.dialog.borrow().is_some()
    }

    /// A person reloads the page or steps in its history: a `beforeunload`
    /// dialog it brings is theirs.
    fn leaving(&self) {
        *self.left_at.lock().expect("left lock") = Some(Instant::now());
    }

    /// A person's answer to the dialog `id`: only the one the page shows
    /// now, and not one the hub answers itself, and one answer at a time.
    /// The text goes only with a prompt that is accepted. Counts as input.
    pub async fn answer_dialog(self: &Arc<Self>, id: u64, accept: bool, text: Option<String>) {
        let kind = match *self.dialog.borrow() {
            Some(shown) if shown.id == id && !shown.theirs => shown.kind,
            _ => return,
        };
        if self.answering.swap(id, Ordering::SeqCst) == id {
            return;
        }
        let mut params = json!({ "accept": accept });
        if accept && kind == "prompt" {
            params["promptText"] = json!(cut(text.as_deref().unwrap_or(""), DIALOG_TEXT));
        }
        let answer = self.send("Page.handleJavaScriptDialog", params).await;
        let _ = self
            .answering
            .compare_exchange(id, 0, Ordering::SeqCst, Ordering::SeqCst);
        // Refused: the page shows no dialog. One that closed meanwhile was
        // closed by its own event; one whose close the hub missed (its
        // events lagged) would hold the screen until every socket closed, so
        // it is closed now.
        if let Err(Fail::Refused(_)) = answer {
            let mut gone = None;
            self.dialog.send_if_modified(|shown| match *shown {
                Some(was) if was.id == id => {
                    gone = shown.take();
                    true
                }
                _ => false,
            });
            if let Some(gone) = gone {
                self.after_dialog(gone);
            }
        }
        self.input_came().await;
    }

    /// The page shows a dialog. A `beforeunload` that comes soon after a
    /// person's reload or step is answered `leave` at once; any other is
    /// told to the sockets. Waiting for a person is not a page that does not
    /// answer: a stall ends, and the size is applied once the dialog closed.
    fn dialog_opening(self: &Arc<Self>, params: &Value) {
        let kind = match params["type"].as_str() {
            Some("confirm") => "confirm",
            Some("prompt") => "prompt",
            Some("beforeunload") => "beforeunload",
            _ => "alert",
        };
        let theirs = kind == "beforeunload"
            && self
                .left_at
                .lock()
                .expect("left lock")
                .is_some_and(|at| at.elapsed() <= LEAVE_WITHIN);
        let id = self.dialogs.fetch_add(1, Ordering::SeqCst) + 1;
        // Shown first: a stall that comes meanwhile sees it ([`Hub::mark_stalled`]).
        self.dialog.send_replace(Some(Shown { id, kind, theirs }));
        if self.mark_stalled(false) {
            self.size_waits.store(true, Ordering::SeqCst);
        }
        if theirs {
            let hub = self.clone();
            tokio::spawn(async move {
                let _ = hub
                    .send("Page.handleJavaScriptDialog", json!({ "accept": true }))
                    .await;
            });
            return;
        }
        // The message is what the page shows the person, as the frames are:
        // it is sent and never logged. Of the frame's address only the host.
        let message = json!({
            "type": "dialog",
            "dialog": {
                "id": id,
                "kind": kind,
                "message": cut(params["message"].as_str().unwrap_or(""), DIALOG_MESSAGE),
                "host": params["url"].as_str().and_then(host_of),
                "prompt": cut(params["defaultPrompt"].as_str().unwrap_or(""), DIALOG_TEXT),
            },
        });
        self.publish(&self.dialog_message, message.to_string());
    }

    /// The dialog closed (answered, or the page went).
    fn dialog_closed(self: &Arc<Self>) {
        if let Some(shown) = self.dialog.send_replace(None) {
            self.after_dialog(shown);
        }
    }

    /// The dialog `shown` no longer shows: the sockets are told, what a
    /// press that opened it holds down is let go (its release came while it
    /// showed and was not sent; a press that came after it closed is not),
    /// and a size kept meanwhile is applied.
    fn after_dialog(self: &Arc<Self>, shown: Shown) {
        if !shown.theirs {
            self.publish(
                &self.dialog_message,
                json!({ "type": "dialog", "dialog": null }).to_string(),
            );
        }
        let hub = self.clone();
        tokio::spawn(async move {
            {
                let mut held = hub.input.lock().await;
                hub.release_held(&mut held, shown.id).await;
            }
            hub.apply_waiting_size().await;
        });
    }

    /// The page did not answer in time: the sockets are told, and the prober
    /// asks until it answers again.
    fn stall(&self) {
        if self.mark_stalled(true) {
            self.stalls.notify_one();
        }
    }

    /// The stalled page answered: the sockets are told, and the last size
    /// is applied anew, which starts the screencast again.
    async fn recover(&self) {
        if self.mark_stalled(false) {
            self.reapply_size().await;
        }
    }

    /// Sets whether the page is stalled and tells the sockets when that
    /// changed, both under the lock of the `page` message: a stall and a
    /// recovery that cross are told in the order the flag changed, and the
    /// last message sent is the flag's value. A page that shows a dialog
    /// does not stall: a command it did not answer waits for the person (a
    /// reload sent just before the dialog opened). Whether it changed.
    fn mark_stalled(&self, stalled: bool) -> bool {
        let mut last = self.page.lock().expect("state lock");
        if stalled && self.shows_dialog() {
            return false;
        }
        if self.stalled.swap(stalled, Ordering::SeqCst) == stalled {
            return false;
        }
        let message: Arc<str> = json!({ "type": "page", "responding": !stalled })
            .to_string()
            .into();
        *last = Some(message.clone());
        let _ = self.out.send(message);
        true
    }

    /// Takes one message of a socket.
    pub async fn handle(self: &Arc<Self>, message: Incoming) -> Handled {
        match message {
            Incoming::Viewport(viewport) if viewport.valid() => {
                self.resize_to(viewport).await;
                Handled::Done
            }
            Incoming::Viewport(_) => Handled::Invalid,
            // Not while a dialog shows: the person answers it first.
            Incoming::Reload | Incoming::Back | Incoming::Forward if self.shows_dialog() => {
                Handled::Done
            }
            Incoming::Reload => {
                self.leaving();
                // Also when the page is stalled: the browser itself answers
                // it, and a page that moves again may need it.
                if let Err(fail) = self
                    .send("Page.reload", json!({ "ignoreCache": false }))
                    .await
                {
                    self.failed(&fail);
                }
                self.input_came().await;
                Handled::Done
            }
            Incoming::Back => {
                self.step(true).await;
                Handled::Done
            }
            Incoming::Forward => {
                self.step(false).await;
                Handled::Done
            }
            Incoming::Dialog { id, accept, text } => {
                self.answer_dialog(id, accept, text).await;
                Handled::Done
            }
            Incoming::Mouse { gen, .. }
            | Incoming::Touch { gen, .. }
            | Incoming::Key { gen, .. }
            | Incoming::Text { gen, .. } => {
                // Held until the input is recorded: a change of size waits
                // for it (see the module docs).
                let mut held = self.input.lock().await;
                let view = self.view();
                if !admits(&view, gen) {
                    return Handled::Dropped { gen: view.gen };
                }
                let Some((method, params)) = command_of(&message, &view) else {
                    return Handled::Invalid;
                };
                // Before the input goes: a dialog it opens is a later one.
                let dialogs = self.dialogs.load(Ordering::SeqCst);
                // Not sent to a stalled page, nor while a dialog shows (see
                // the module docs).
                if self.is_stalled() || self.shows_dialog() {
                    return Handled::Done;
                }
                match self.send_input(method, params).await {
                    Ok(_) => {}
                    // Sent, and the page may take it later.
                    Err(fail @ Fail::Late(_)) => self.failed(&fail),
                    Err(fail) => {
                        self.failed(&fail);
                        return Handled::Done;
                    }
                }
                note_held(&mut held, &message, dialogs);
                drop(held);
                self.input_came().await;
                Handled::Done
            }
        }
    }
}

impl Hub {
    /// One step back (or forward) in the page's history, when its own history
    /// allows it now ([`History::back`]): a step back never goes to the blank
    /// page the server passed through, whatever the socket believes. Counts as
    /// a person's input. A step the browser refuses (its history changed
    /// between the read and the step) is only not taken: a page or connection
    /// that is gone ends the hub through its own events, not through one
    /// press of a button.
    async fn step(&self, back: bool) {
        if let Ok(history) = self.history().await {
            let entry = if back {
                history.back()
            } else {
                history.forward()
            };
            if let Some(entry) = entry {
                self.leaving();
                let step = self
                    .send("Page.navigateToHistoryEntry", json!({ "entryId": entry }))
                    .await;
                // Not taken: a `beforeunload` the page brings now is its own.
                if let Err(Fail::Refused(_)) = step {
                    *self.left_at.lock().expect("left lock") = None;
                }
            }
        }
        self.input_came().await;
        // A step that was refused leaves the buttons as the page says.
        self.changed.notify_one();
    }

    async fn history(&self) -> Result<History, String> {
        let answer = self
            .send("Page.getNavigationHistory", json!({}))
            .await
            .map_err(|e| e.to_string())?;
        History::parse(&answer).ok_or_else(|| "no history".to_owned())
    }

    /// Reads the state of the page shown and the run's tabs, and tells the
    /// sockets what changed.
    async fn read_state(&self) {
        let _one = self.refresh.lock().await;
        if let Ok(history) = self.history().await {
            self.publish(&self.nav, history.nav().message());
        }
        let screen = match self.screens.screen(&self.job).await {
            Ok(Some(screen)) if screen.run_id.as_deref() == Some(self.binding.run.as_str()) => {
                screen
            }
            _ => return,
        };
        *self.listed.lock().expect("listed lock") = screen.pages.clone();
        let Ok(answer) = self
            .conn
            .command(None, "Target.getTargets", json!({}))
            .await
        else {
            return;
        };
        let message = tabs_message(
            &screen.pages,
            &answer["targetInfos"],
            &self.binding.target,
            screen.first_target_id.as_deref(),
        );
        self.publish(&self.tabs, message);
    }

    /// Sends `message` to the sockets when it is not what `slot` last held.
    fn publish(&self, slot: &Mutex<Option<Arc<str>>>, message: String) {
        let message: Arc<str> = message.into();
        {
            let mut last = slot.lock().expect("state lock");
            if last.as_deref() == Some(&*message) {
                return;
            }
            *last = Some(message.clone());
        }
        let _ = self.out.send(message);
    }
}

/// What a sent input leaves held down on the page, `dialogs` having shown.
fn note_held(held: &mut Held, message: &Incoming, dialogs: u64) {
    match message {
        Incoming::Mouse {
            event,
            x,
            y,
            button,
            ..
        } => match event.as_str() {
            "mousePressed" => {
                let button = button.clone().unwrap_or_else(|| "left".to_owned());
                held.mouse = Some((button, *x, *y));
                held.mouse_from = dialogs;
            }
            "mouseReleased" => held.mouse = None,
            "mouseMoved" => {
                if let Some((_, hx, hy)) = &mut held.mouse {
                    (*hx, *hy) = (*x, *y);
                }
            }
            _ => {}
        },
        // The points are the fingers left on the page, also after a
        // `touchEnd` of one of them.
        Incoming::Touch { event, points, .. } => {
            let touch = event != "touchCancel" && !points.is_empty();
            if touch && !held.touch {
                held.touch_from = dialogs;
            }
            held.touch = touch;
        }
        _ => {}
    }
}

/// What of `held` went down before `before` dialogs had shown, taken out of
/// it: the button, and whether fingers were down.
fn let_go(held: &mut Held, before: u64) -> (Option<(String, f64, f64)>, bool) {
    let mouse = if held.mouse_from < before {
        held.mouse.take()
    } else {
        None
    };
    let touch = held.touch_from < before && std::mem::take(&mut held.touch);
    (mouse, touch)
}

impl Hub {
    /// Lets go of what the page holds down that went down before `before`
    /// dialogs had shown (all of it with `u64::MAX`; see the module docs).
    /// A release that does not click.
    async fn release_held(&self, held: &mut Held, before: u64) {
        let (mouse, touch) = let_go(held, before);
        if let Some((button, x, y)) = mouse {
            let _ = self
                .send(
                    "Input.dispatchMouseEvent",
                    json!({
                        "type": "mouseReleased", "x": x, "y": y, "button": button,
                        "buttons": 0, "clickCount": 0, "modifiers": 0,
                    }),
                )
                .await;
        }
        if touch {
            let _ = self
                .send(
                    "Input.dispatchTouchEvent",
                    json!({ "type": "touchCancel", "touchPoints": [], "modifiers": 0 }),
                )
                .await;
        }
    }

    /// The device's size (the last one to open or change wins): a new
    /// generation, the page laid out for it, and the frames of its size
    /// (see the module docs).
    async fn resize_to(&self, viewport: Viewport) {
        let one = self.resize.lock().await;
        self.apply_size(viewport, one).await;
    }

    /// The last size, read once no other change of size is under way, is
    /// applied anew: one a socket asked for while the page was stalled, or
    /// meanwhile, is not undone by an older one.
    async fn reapply_size(&self) {
        let one = self.resize.lock().await;
        if let Some(viewport) = self.view().viewport {
            self.apply_size(viewport, one).await;
        }
    }

    /// A size kept while a dialog showed is applied once it closed. Under
    /// the lock of a change of size, which a size kept meanwhile was kept
    /// under too.
    async fn apply_waiting_size(&self) {
        let one = self.resize.lock().await;
        if self.size_waits.load(Ordering::SeqCst) {
            if let Some(viewport) = self.view().viewport {
                self.apply_size(viewport, one).await;
            }
        }
    }

    /// [`Hub::resize_to`], with its change of size under way (`_one`).
    async fn apply_size(&self, viewport: Viewport, _one: tokio::sync::MutexGuard<'_, ()>) {
        // A stalled page, or one that shows a dialog, would not answer: the
        // size is applied when it answers again ([`Hub::recover`]) or the
        // dialog closed.
        if self.is_stalled() || self.shows_dialog() {
            self.view.lock().expect("view lock").viewport = Some(viewport);
            self.size_waits.store(true, Ordering::SeqCst);
            return;
        }
        self.size_waits.store(false, Ordering::SeqCst);
        let gen = {
            let mut held = self.input.lock().await;
            let gen = {
                let mut view = self.view.lock().expect("view lock");
                view.gen += 1;
                view.resizing = true;
                view.viewport = Some(viewport);
                view.gen
            };
            self.release_held(&mut held, u64::MAX).await;
            gen
        };
        let applied = async {
            self.send("Page.stopScreencast", json!({})).await?;
            self.send(
                "Emulation.setDeviceMetricsOverride",
                json!({
                    "width": viewport.width,
                    "height": viewport.height,
                    "deviceScaleFactor": viewport.dpr,
                    "mobile": viewport.touch,
                }),
            )
            .await?;
            self.send(
                "Emulation.setTouchEmulationEnabled",
                json!({ "enabled": viewport.touch, "maxTouchPoints": if viewport.touch { 5 } else { 1 } }),
            )
            .await?;
            self.start_screencast(Some(viewport)).await
        }
        .await;
        match applied {
            Ok(after) => {
                let viewport: Arc<str> = json!({
                    "type": "viewport", "gen": gen,
                    "width": viewport.width, "height": viewport.height,
                    "dpr": viewport.dpr, "touch": viewport.touch,
                })
                .to_string()
                .into();
                let (done, finished) = oneshot::channel();
                if self
                    .barriers
                    .send(Barrier {
                        viewport,
                        after,
                        done,
                    })
                    .is_ok()
                {
                    // Not answered: the pump is gone with the connection.
                    let _ = finished.await;
                }
            }
            // The new size is sent once the page answers again and it is
            // applied anew: the devices' inputs wait for it.
            Err(fail @ Fail::Late(_)) => {
                self.view.lock().expect("view lock").resizing = false;
                // A dialog that opened meanwhile holds the page: the size is
                // applied once it closed.
                if self.shows_dialog() {
                    self.size_waits.store(true, Ordering::SeqCst);
                } else {
                    self.failed(&fail);
                }
            }
            Err(fail) => {
                self.view.lock().expect("view lock").resizing = false;
                eprintln!(
                    "trss-web: the remote screen of job {} could not change its size: {fail}",
                    self.job
                );
                self.end();
            }
        }
    }

    /// The pump read every frame of the old size: the new size is sent and
    /// its frames go through from here.
    fn finish_resize(&self, viewport: Arc<str>) {
        let _ = self.out.send(viewport);
        self.view.lock().expect("view lock").resizing = false;
    }

    /// One event of the page's connection. A frame that comes while the
    /// size changes is kept aside in `kept` (the newest one).
    fn take(self: &Arc<Self>, event: Event, kept: &mut Option<Event>) {
        let ours = event.session_id.as_deref() == Some(self.session.as_str());
        match event.method.as_str() {
            "Page.screencastFrame" if ours => {
                self.ack(&event.params);
                if self.view().resizing {
                    *kept = Some(event);
                } else {
                    self.frame(&event.params);
                }
            }
            "Target.detachedFromTarget"
                if event.params["sessionId"].as_str() == Some(self.session.as_str()) =>
            {
                self.end()
            }
            "Inspector.detached" if ours => self.end(),
            // The page shown went to another address (the main frame, or a
            // change of the address within its document), or a target of the
            // run changed its title or address, came or went.
            "Page.frameNavigated" if ours && event.params["frame"]["parentId"].is_null() => {
                self.changed.notify_one()
            }
            "Page.navigatedWithinDocument" if ours => self.changed.notify_one(),
            "Page.javascriptDialogOpening" if ours => self.dialog_opening(&event.params),
            "Page.javascriptDialogClosed" if ours => self.dialog_closed(),
            "Target.targetCreated" | "Target.targetDestroyed" | "Target.targetInfoChanged" => {
                self.changed.notify_one()
            }
            _ => {}
        }
    }

    /// Frames that were not seen are not acknowledged, which stalls the
    /// screencast: it starts again (at the current size).
    fn restart_screencast(self: &Arc<Self>) {
        let hub = self.clone();
        tokio::spawn(async move {
            let _one = hub.resize.lock().await;
            let viewport = hub.view().viewport;
            let _ = hub.send("Page.stopScreencast", json!({})).await;
            let _ = hub.start_screencast(viewport).await;
        });
    }

    /// Acknowledges a frame: the browser sends the next one.
    fn ack(&self, params: &Value) {
        if let Some(id) = params["sessionId"].as_i64() {
            let conn = self.conn.clone();
            let session = self.session.clone();
            tokio::spawn(async move {
                let _ = conn
                    .command(
                        Some(&session),
                        "Page.screencastFrameAck",
                        json!({ "sessionId": id }),
                    )
                    .await;
            });
        }
    }

    /// A person's input reached the page: recorded for the worker's idle end,
    /// at most every [`Times::input_every`].
    async fn input_came(&self) {
        let due = {
            let mut last = self.last_input.lock().expect("input lock");
            let due = last.is_none_or(|at| at.elapsed() >= self.times.input_every);
            if due {
                *last = Some(Instant::now());
            }
            due
        };
        if due {
            if let Err(err) = self
                .screens
                .record_input(&self.job, &self.binding.run, now_millis())
                .await
            {
                eprintln!(
                    "trss-web: cannot record the input of job {}: {err}",
                    self.job
                );
            }
        }
    }

    /// A frame of the page, sent to the sockets when it is of the current
    /// size and no change of size is under way.
    fn frame(&self, params: &Value) {
        let view = self.view();
        let meta = &params["metadata"];
        let (width, height) = (
            meta["deviceWidth"].as_f64().unwrap_or(0.0),
            meta["deviceHeight"].as_f64().unwrap_or(0.0),
        );
        if view.resizing {
            return;
        }
        if let Some(v) = view.viewport {
            if (width - f64::from(v.width)).abs() > SIZE_SLACK
                || (height - f64::from(v.height)).abs() > SIZE_SLACK
            {
                return;
            }
        }
        let Some(data) = params["data"].as_str() else {
            return;
        };
        let message = json!({
            "type": "frame", "gen": view.gen,
            "width": width.round(), "height": height.round(), "data": data,
        });
        let _ = self.out.send(message.to_string().into());
    }
}

/// The message that ends a socket; `reason` is `browser` (the run's page or
/// connection is gone), `run` (the job's binding changed), `unreachable`,
/// `stuck` (the page did not answer when the screen opened) or `replaced` (a
/// newer screen of the binding took this one's seat).
pub fn ended_message(reason: &str) -> String {
    json!({ "type": "ended", "reason": reason }).to_string()
}

/// Asks a stalled page every [`Times::probe_every`] whether it answers again
/// (see the module docs), until the hub is dropped.
async fn prober(hub: std::sync::Weak<Hub>, stop: CancellationToken) {
    loop {
        let Some((stalls, every)) = hub
            .upgrade()
            .map(|hub| (hub.stalls.clone(), hub.times.probe_every))
        else {
            return;
        };
        tokio::select! {
            _ = stop.cancelled() => return,
            _ = stalls.notified() => {}
        }
        loop {
            tokio::select! {
                _ = stop.cancelled() => return,
                _ = tokio::time::sleep(every) => {}
            }
            let Some(hub) = hub.upgrade() else {
                return;
            };
            if hub.is_ended() {
                return;
            }
            if !hub.is_stalled() {
                break;
            }
            match hub
                .send("Runtime.evaluate", json!({ "expression": "0" }))
                .await
            {
                // A refusal is the page's answer too.
                Ok(_) | Err(Fail::Refused(_)) => {
                    hub.recover().await;
                    break;
                }
                Err(Fail::Gone(_)) => {
                    hub.end();
                    return;
                }
                Err(Fail::Late(_)) => {}
            }
        }
    }
}

/// Reads the events of the hub's connection until the hub is dropped (which
/// closes the connection) or the connection closes, and finishes the
/// changes of size (see the module docs).
async fn pump(
    hub: std::sync::Weak<Hub>,
    conn: Connection,
    mut events: broadcast::Receiver<Event>,
    mut barriers: mpsc::UnboundedReceiver<Barrier>,
    stop: CancellationToken,
) {
    // The newest frame that came while the size changes.
    let mut kept: Option<Event> = None;
    loop {
        let event = tokio::select! {
            _ = stop.cancelled() => return,
            _ = conn.closed() => {
                if let Some(hub) = hub.upgrade() {
                    hub.end();
                }
                return;
            }
            Some(barrier) = barriers.recv() => {
                let Some(hub) = hub.upgrade() else {
                    return;
                };
                // What came meanwhile is taken as during the change: every
                // frame the browser sent before it answered the start is in
                // the feed by now.
                let mut lagged = false;
                loop {
                    match events.try_recv() {
                        Ok(event) => hub.take(event, &mut kept),
                        Err(TryRecvError::Lagged(_)) => lagged = true,
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Closed) => {
                            hub.end();
                            return;
                        }
                    }
                }
                hub.finish_resize(barrier.viewport);
                if let Some(frame) = kept.take().filter(|f| f.seq > barrier.after) {
                    hub.frame(&frame.params);
                }
                let _ = barrier.done.send(());
                if lagged {
                    hub.restart_screencast();
                }
                continue;
            }
            event = events.recv() => event,
        };
        let Some(hub) = hub.upgrade() else {
            return;
        };
        match event {
            Ok(event) => hub.take(event, &mut kept),
            Err(RecvError::Lagged(_)) => hub.restart_screencast(),
            Err(RecvError::Closed) => {
                hub.end();
                return;
            }
        }
    }
}

/// Reads the page's state and the run's tabs whenever they may have changed
/// (see the module docs), at most every [`REFRESH_GAP`], until the hub is
/// dropped.
async fn refresher(hub: std::sync::Weak<Hub>, stop: CancellationToken) {
    loop {
        let Some(changed) = hub.upgrade().map(|hub| hub.changed.clone()) else {
            return;
        };
        tokio::select! {
            _ = stop.cancelled() => return,
            _ = changed.notified() => {}
        }
        {
            let Some(hub) = hub.upgrade() else {
                return;
            };
            if hub.is_ended() {
                return;
            }
            hub.read_state().await;
        }
        tokio::select! {
            _ = stop.cancelled() => return,
            _ = tokio::time::sleep(REFRESH_GAP) => {}
        }
    }
}

/// Ends the hub (`run`) once its binding is no longer the job's: the job no
/// longer waits for its check, or another run, page or binding took its
/// place. One reading every `every` for all the sockets of the hub.
async fn watch_binding(hub: std::sync::Weak<Hub>, every: Duration, stop: CancellationToken) {
    let mut ticks = tokio::time::interval(every);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticks.tick().await;
    loop {
        tokio::select! {
            _ = stop.cancelled() => return,
            _ = ticks.tick() => {}
        }
        let Some(hub) = hub.upgrade() else {
            return;
        };
        if hub.is_ended() {
            return;
        }
        let Ok(screen) = hub.screens.screen(&hub.job).await else {
            continue;
        };
        let still = screen
            .as_ref()
            .and_then(|s| s.seat())
            .is_some_and(|seat| seat == hub.binding);
        if !still {
            hub.end_because("run");
            return;
        }
        // The worker changed the run's list of pages.
        let pages = screen.map(|s| s.pages).unwrap_or_default();
        if *hub.listed.lock().expect("listed lock") != pages {
            hub.changed.notify_one();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_input_of_an_older_size_or_made_while_it_changes_is_dropped() {
        let mut view = View::default();
        assert!(admits(&view, 0));
        view.gen = 3;
        assert!(admits(&view, 3));
        assert!(!admits(&view, 2), "an older generation");
        assert!(!admits(&view, 4), "a generation not made yet");
        view.resizing = true;
        assert!(!admits(&view, 3), "during a resize");
    }

    #[test]
    fn inputs_out_of_bounds_are_not_sent() {
        let view = View {
            gen: 1,
            resizing: false,
            viewport: Some(Viewport {
                width: 400,
                height: 600,
                dpr: 3.0,
                touch: true,
            }),
        };
        let parse = |text: &str| serde_json::from_str::<Incoming>(text).unwrap();
        let (method, params) = command_of(
            &parse(r#"{"type":"mouse","gen":1,"event":"mousePressed","x":10,"y":20,"button":"left","clickCount":1}"#),
            &view,
        )
        .unwrap();
        assert_eq!(method, "Input.dispatchMouseEvent");
        assert_eq!(params["button"], "left");
        assert!(command_of(
            &parse(r#"{"type":"mouse","gen":1,"event":"mousePressed","x":401,"y":20}"#),
            &view
        )
        .is_none());
        assert!(command_of(
            &parse(r#"{"type":"mouse","gen":1,"event":"dragIntercepted","x":1,"y":2}"#),
            &view
        )
        .is_none());
        let (method, params) = command_of(
            &parse(r#"{"type":"touch","gen":1,"event":"touchStart","points":[{"x":5,"y":6}]}"#),
            &view,
        )
        .unwrap();
        assert_eq!(method, "Input.dispatchTouchEvent");
        assert_eq!(params["touchPoints"][0]["id"], 0);
        let (method, params) = command_of(
            &parse(r#"{"type":"key","gen":1,"event":"keyDown","key":"Enter","code":"Enter","keyCode":13}"#),
            &view,
        )
        .unwrap();
        assert_eq!(method, "Input.dispatchKeyEvent");
        assert_eq!(params["windowsVirtualKeyCode"], 13);
        assert!(command_of(&parse(r#"{"type":"text","gen":1,"text":""}"#), &view).is_none());
        let long = format!(
            r#"{{"type":"text","gen":1,"text":"{}"}}"#,
            "가".repeat(2001)
        );
        assert!(command_of(&parse(&long), &view).is_none());
        assert!(!Viewport {
            width: 100,
            height: 600,
            dpr: 1.0,
            touch: false
        }
        .valid());
    }

    #[test]
    fn a_closed_dialog_lets_go_only_of_what_went_down_before_it_opened() {
        let parse = |text: &str| serde_json::from_str::<Incoming>(text).unwrap();
        let press =
            parse(r#"{"type":"mouse","gen":1,"event":"mousePressed","x":5,"y":6,"button":"left"}"#);
        let touch =
            parse(r#"{"type":"touch","gen":1,"event":"touchStart","points":[{"x":1,"y":1}]}"#);

        // The press that opened dialog 1 went down with none shown yet.
        let mut held = Held::default();
        note_held(&mut held, &press, 0);
        note_held(&mut held, &touch, 0);
        assert_eq!(
            let_go(&mut held, 1),
            (Some(("left".to_owned(), 5.0, 6.0)), true)
        );
        assert!(held.mouse.is_none() && !held.touch);

        // A press and a touch after dialog 1 closed are the person's own: its
        // release, coming late, leaves them down. A change of size lets go of
        // all.
        note_held(&mut held, &press, 1);
        note_held(&mut held, &touch, 1);
        assert_eq!(let_go(&mut held, 1), (None, false));
        assert!(held.mouse.is_some() && held.touch);
        assert_eq!(
            let_go(&mut held, u64::MAX),
            (Some(("left".to_owned(), 5.0, 6.0)), true)
        );
    }

    #[test]
    fn fingers_are_held_while_a_touch_event_leaves_points() {
        let parse = |text: &str| serde_json::from_str::<Incoming>(text).unwrap();
        let mut held = Held::default();
        let two = r#"[{"x":1,"y":1,"id":0},{"x":2,"y":2,"id":1}]"#;
        let one = r#"[{"x":2,"y":2,"id":1}]"#;
        note_held(
            &mut held,
            &parse(&format!(
                r#"{{"type":"touch","gen":1,"event":"touchStart","points":{two}}}"#
            )),
            0,
        );
        assert!(held.touch);
        // One finger lifts; the other stays down, held since before the
        // dialog that showed meanwhile.
        note_held(
            &mut held,
            &parse(&format!(
                r#"{{"type":"touch","gen":1,"event":"touchEnd","points":{one}}}"#
            )),
            1,
        );
        assert!(held.touch);
        assert_eq!(held.touch_from, 0);
        note_held(
            &mut held,
            &parse(r#"{"type":"touch","gen":1,"event":"touchEnd","points":[]}"#),
            0,
        );
        assert!(!held.touch);
        // A new touch, after that dialog.
        note_held(
            &mut held,
            &parse(&format!(
                r#"{{"type":"touch","gen":1,"event":"touchMove","points":{one}}}"#
            )),
            1,
        );
        assert!(held.touch);
        assert_eq!(held.touch_from, 1);
        note_held(
            &mut held,
            &parse(r#"{"type":"touch","gen":1,"event":"touchCancel","points":[]}"#),
            0,
        );
        assert!(!held.touch);
    }
}
