//! The messages the server sends on a remote screen's socket (see the
//! protocol in [`super`]). The web's `ServerMessage` is generated from
//! [`Outgoing`], so a message the server cannot send is not one the web
//! knows.

use serde::Serialize;

use super::{hub::Viewport, nav::Nav};

/// Why the server ended a socket: the page or its connection is gone
/// (`browser`), the job's binding changed (`run`), the run could not be
/// reached (`unreachable`), the page did not answer when the screen opened
/// (`stuck`), or a newer screen of the binding took this one's seat
/// (`replaced`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum EndedReason {
    Browser,
    Run,
    Unreachable,
    Stuck,
    Replaced,
}

/// The kinds of dialog a page shows: `alert("…")`, `confirm("…")`,
/// `prompt("…")`, and the ask before leaving the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum DialogKind {
    Alert,
    Confirm,
    Prompt,
    Beforeunload,
}

impl DialogKind {
    /// The kind the browser names (`Page.javascriptDialogOpening`'s `type`);
    /// one it does not name is an `alert`.
    pub fn of(browser: Option<&str>) -> Self {
        match browser {
            Some("confirm") => Self::Confirm,
            Some("prompt") => Self::Prompt,
            Some("beforeunload") => Self::Beforeunload,
            _ => Self::Alert,
        }
    }
}

/// A dialog the remote page shows, which no frame draws: the person answers
/// it on the screen.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct PageDialog {
    /// What the answer names the dialog by.
    pub id: u64,
    pub kind: DialogKind,
    /// The page's text, cut by the server; empty for `beforeunload`, whose
    /// text browsers no longer show.
    pub message: String,
    /// Only the host of the frame that opened it; `null` for one with none.
    pub host: Option<String>,
    /// A prompt's default text.
    pub prompt: String,
}

/// One tab of the run: a page the server browser has open.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Tab {
    /// What a switch or close request names the page by.
    pub id: String,
    /// The page's title, already cut by the server; empty when the page has
    /// none.
    pub title: String,
    /// Only the host of the page's address, never a path or query; `null` for
    /// a page with none (`about:blank`).
    pub host: Option<String>,
    /// The page the screen shows.
    pub shown: bool,
    /// The run's first page cannot be closed.
    pub closable: bool,
}

/// A message the server sends on the socket.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "ServerMessage"))]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Outgoing<'a> {
    /// A change of size is done (or, on connecting, the size the page has
    /// now); `gen` names the frames of it.
    Viewport {
        gen: u64,
        width: u32,
        height: u32,
        dpr: f64,
        touch: bool,
    },
    /// A JPEG frame of generation `gen`; `width` and `height` are the remote
    /// page's CSS pixels.
    Frame {
        gen: u64,
        width: f64,
        height: f64,
        data: &'a str,
    },
    /// An input was dropped; `gen` is the generation the page is at.
    Dropped {
        gen: u64,
    },
    /// The state of the page shown; sent on connecting and whenever it
    /// changes.
    Nav(Nav),
    /// The run's tabs in the order the worker listed them; sent on
    /// connecting and whenever they change.
    Tabs {
        tabs: Vec<Tab>,
    },
    /// Whether the page answers: `false` once it did not answer an input in
    /// time (inputs are not sent to it), `true` once it answers again. Sent
    /// on connecting and whenever it changes; never while the page never
    /// stalled.
    Page {
        responding: bool,
    },
    /// The dialog the page shows, or `null` once it closed. Inputs, reloads
    /// and steps are not sent to the page meanwhile. Sent on connecting and
    /// whenever it changes.
    Dialog {
        dialog: Option<PageDialog>,
    },
    Ended {
        reason: EndedReason,
    },
}

impl Outgoing<'_> {
    /// The viewport message for `viewport` at generation `gen`.
    pub fn viewport(gen: u64, viewport: Viewport) -> Self {
        Self::Viewport {
            gen,
            width: viewport.width,
            height: viewport.height,
            dpr: viewport.dpr,
            touch: viewport.touch,
        }
    }

    /// The text frame.
    pub fn text(&self) -> String {
        serde_json::to_string(self).expect("a message of the protocol is JSON")
    }
}
