//! How the web starts what it stored for the [`worker`](crate::worker) part,
//! in the same process.
//!
//! The worker part runs a loop each for the commands, the subtitle jobs and
//! the remote screens of the jobs. A handler that stored work for one of them
//! rings that loop, which then looks at once. A ring while the loop is busy is
//! kept until its next look, and rings in between count once: a look takes
//! everything that is waiting. The commands' loop looks only when it is rung,
//! when a command ends or when one it left for later is due; the jobs' and the
//! screens' loops also look on their own every few seconds.

use std::sync::Arc;

use tokio::sync::Notify;

/// The rings of the worker part's loops. Cheap to clone; clones ring the same
/// loops. One made with `default` rings loops that only a worker given it
/// ([`crate::worker::Worker::with_wakes`]) listens to.
#[derive(Clone, Default)]
pub struct Wakes {
    pub(crate) commands: Arc<Notify>,
    pub(crate) jobs: Arc<Notify>,
    pub(crate) screens: Arc<Notify>,
}

impl Wakes {
    /// A command was stored: its loop claims it now.
    pub fn commands(&self) {
        self.commands.notify_one();
    }

    /// A subtitle job was made, or a person's decision or cleanup on one was
    /// stored: the jobs' loop runs what is ready now.
    pub fn jobs(&self) {
        self.jobs.notify_one();
    }

    /// A person asked something of a job's remote screen, or asked a find job
    /// to finish: the screens' loop answers now.
    pub fn screens(&self) {
        self.screens.notify_one();
    }

    /// Returns once the commands' loop was rung (at once if it was rung since
    /// the last such wait), for a caller that listens in its place.
    pub async fn commands_rung(&self) {
        self.commands.notified().await;
    }

    /// [`Wakes::commands_rung`] for the jobs' loop.
    pub async fn jobs_rung(&self) {
        self.jobs.notified().await;
    }

    /// [`Wakes::commands_rung`] for the screens' loop.
    pub async fn screens_rung(&self) {
        self.screens.notified().await;
    }
}
