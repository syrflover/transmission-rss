//! One remote screen of a browser run: the web's own DevTools connection to
//! the page that shows a job's check, shared by every socket open on it (see
//! the protocol in [`super`]).
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
//! # Seats
//!
//! At most [`MAX_SOCKETS`] sockets are open on a hub. A new one always gets
//! a seat: when they are all taken, the oldest socket's seat is given to it
//! and that socket ends (`replaced`). The newest socket is the device the
//! person is using; the oldest may be a phone that slept with the screen
//! open and left a dead connection behind.

use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{
    broadcast::{
        self,
        error::{RecvError, TryRecvError},
    },
    mpsc, oneshot, Notify,
};
use tokio_util::sync::{CancellationToken, DropGuard};
use trss_browser::cdp::{Connection, Event};
use trss_jobs::ScreenStore;

use super::nav::{tabs_message, History};
use crate::commands_api::now_millis;

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
        Incoming::Viewport(_) | Incoming::Reload | Incoming::Back | Incoming::Forward => None,
    }
}

/// The binding a hub shows: the run, its page and when it was bound to the
/// job. Another check of the job in the same run is another binding.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Binding {
    pub run: String,
    pub target: String,
    pub bound_at: i64,
}

/// The most sockets open on one hub; one more takes the oldest one's seat
/// (`replaced`).
pub const MAX_SOCKETS: usize = 4;

/// A mouse button or a touch the page holds down.
#[derive(Debug, Default)]
struct Held {
    mouse: Option<(String, f64, f64)>,
    /// Fingers are on the page: the last touch event sent left points.
    touch: bool,
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
    input_every: Duration,
    last_input: Mutex<Option<Instant>>,
    /// Taken to admit, send and record an input, and to move the generation
    /// on and let go of what is held (see the module docs).
    input: tokio::sync::Mutex<Held>,
    seats: Mutex<Seats>,
    /// Cancelled when the page or the connection is gone, or the binding
    /// changed; `reason` says which (`browser`, `run`).
    ended: CancellationToken,
    reason: Mutex<&'static str>,
    /// The `nav` and `tabs` messages as last sent.
    nav: Mutex<Option<Arc<str>>>,
    tabs: Mutex<Option<Arc<str>>>,
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
        self.conn.close();
    }
}

impl Hub {
    /// Attaches to the page of `binding` through the launcher's DevTools
    /// proxy at `cdp_url` and starts sending its frames. Every `check_every`
    /// it reads whether the binding is still the job's, and ends (`run`)
    /// when it is not.
    #[allow(clippy::too_many_arguments)]
    pub async fn open(
        cdp_url: &str,
        token: &str,
        job: &str,
        binding: Binding,
        screens: ScreenStore,
        input_every: Duration,
        check_every: Duration,
    ) -> Result<Arc<Hub>, String> {
        let conn = Connection::connect(cdp_url, token)
            .await
            .map_err(|e| format!("connect: {e}"))?;
        let events = conn.events();
        let attached = conn
            .command(
                None,
                "Target.attachToTarget",
                json!({ "targetId": binding.target, "flatten": true }),
            )
            .await
            .map_err(|e| format!("attach: {e}"))?;
        let session = attached["sessionId"]
            .as_str()
            .ok_or("attach: no session")?
            .to_owned();
        conn.command(Some(&session), "Page.enable", json!({}))
            .await
            .map_err(|e| format!("Page.enable: {e}"))?;
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
            input_every,
            last_input: Mutex::new(None),
            input: tokio::sync::Mutex::default(),
            seats: Mutex::default(),
            ended: CancellationToken::new(),
            reason: Mutex::new("browser"),
            nav: Mutex::default(),
            tabs: Mutex::default(),
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
        tokio::spawn(watch_binding(Arc::downgrade(&hub), check_every, stop));
        // Before the first frame: a socket that connects finds the state.
        hub.read_state().await;
        hub.start_screencast(None)
            .await
            .map_err(|e| format!("screencast: {e}"))?;
        Ok(hub)
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

    /// The `nav` and `tabs` messages as last sent, for a socket that
    /// connects or skipped some.
    pub fn states(&self) -> Vec<Arc<str>> {
        [&self.nav, &self.tabs]
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

    async fn send(&self, method: &str, params: Value) -> Result<Value, String> {
        self.conn
            .command(Some(&self.session), method, params)
            .await
            .map_err(|e| e.to_string())
    }

    /// Starts the screencast; how many events came before the start was
    /// answered.
    async fn start_screencast(&self, viewport: Option<Viewport>) -> Result<u64, String> {
        let mut params = json!({ "format": "jpeg", "quality": FRAME_QUALITY, "everyNthFrame": 1 });
        if let Some(v) = viewport {
            params["maxWidth"] = json!(v.width);
            params["maxHeight"] = json!(v.height);
        }
        self.conn
            .command_marked(Some(&self.session), "Page.startScreencast", params)
            .await
            .map(|(_, after)| after)
            .map_err(|e| e.to_string())
    }

    /// Takes one message of a socket.
    pub async fn handle(&self, text: &str) -> Handled {
        let Ok(message) = serde_json::from_str::<Incoming>(text) else {
            return Handled::Invalid;
        };
        match message {
            Incoming::Viewport(viewport) if viewport.valid() => {
                self.resize_to(viewport).await;
                Handled::Done
            }
            Incoming::Viewport(_) => Handled::Invalid,
            Incoming::Reload => {
                if self
                    .send("Page.reload", json!({ "ignoreCache": false }))
                    .await
                    .is_err()
                {
                    self.end();
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
                if self.send(method, params).await.is_err() {
                    self.end();
                    return Handled::Done;
                }
                note_held(&mut held, &message);
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
                let _ = self
                    .send("Page.navigateToHistoryEntry", json!({ "entryId": entry }))
                    .await;
            }
        }
        self.input_came().await;
        // A step that was refused leaves the buttons as the page says.
        self.changed.notify_one();
    }

    async fn history(&self) -> Result<History, String> {
        let answer = self.send("Page.getNavigationHistory", json!({})).await?;
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

/// What a sent input leaves held down on the page.
fn note_held(held: &mut Held, message: &Incoming) {
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
            held.touch = event != "touchCancel" && !points.is_empty();
        }
        _ => {}
    }
}

impl Hub {
    /// Lets go of what the page holds down (see the module docs). A
    /// release that does not click.
    async fn release_held(&self, held: &mut Held) {
        let held = std::mem::take(held);
        if let Some((button, x, y)) = held.mouse {
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
        if held.touch {
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
        let _one = self.resize.lock().await;
        let gen = {
            let mut held = self.input.lock().await;
            let gen = {
                let mut view = self.view.lock().expect("view lock");
                view.gen += 1;
                view.resizing = true;
                view.viewport = Some(viewport);
                view.gen
            };
            self.release_held(&mut held).await;
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
            Err(err) => {
                self.view.lock().expect("view lock").resizing = false;
                eprintln!(
                    "trss-web: the remote screen of job {} could not change its size: {err}",
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
    fn take(&self, event: Event, kept: &mut Option<Event>) {
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
    /// at most every `input_every`.
    async fn input_came(&self) {
        let due = {
            let mut last = self.last_input.lock().expect("input lock");
            let due = last.is_none_or(|at| at.elapsed() >= self.input_every);
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
/// connection is gone), `run` (the job's binding changed), `unreachable` or
/// `replaced` (a newer screen of the binding took this one's seat).
pub fn ended_message(reason: &str) -> String {
    json!({ "type": "ended", "reason": reason }).to_string()
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
        let b = &hub.binding;
        let still = screen.as_ref().is_some_and(|s| {
            s.waiting
                && s.run_id.as_deref() == Some(b.run.as_str())
                && s.target_id.as_deref() == Some(b.target.as_str())
                && s.bound_at == Some(b.bound_at)
        });
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
        );
        assert!(held.touch);
        // One finger lifts; the other stays down.
        note_held(
            &mut held,
            &parse(&format!(
                r#"{{"type":"touch","gen":1,"event":"touchEnd","points":{one}}}"#
            )),
        );
        assert!(held.touch);
        note_held(
            &mut held,
            &parse(r#"{"type":"touch","gen":1,"event":"touchEnd","points":[]}"#),
        );
        assert!(!held.touch);
        note_held(
            &mut held,
            &parse(&format!(
                r#"{{"type":"touch","gen":1,"event":"touchMove","points":{one}}}"#
            )),
        );
        assert!(held.touch);
        note_held(
            &mut held,
            &parse(r#"{"type":"touch","gen":1,"event":"touchCancel","points":[]}"#),
        );
        assert!(!held.touch);
    }
}
