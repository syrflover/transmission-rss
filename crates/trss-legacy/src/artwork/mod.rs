//! Work artwork (`docs/specs/library.md`, 작품 표지): a cover for each work,
//! selected automatically from AniList when the work's folder name names
//! exactly one AniList entry, or chosen by the user from AniList's results or
//! as an uploaded file.
//!
//! - [`title`]: the automatic decision.
//! - [`anilist`]: the AniList client and its pace.
//! - [`image`]: judging image bytes.
//! - [`files`]: the image files in the app data folder.
//! - [`queue`]: the worker's automatic searches and fetches.
//!
//! [`Artwork`] puts them together for both processes. The web carries out the
//! user's choices itself: the user waits for them and needs the reason when a
//! file or an entry is refused, an upload's bytes arrive at the web, and the
//! worker's command loop waits behind a running cycle. Both processes write
//! the app data folder and the database safely side by side: files get fresh
//! names and never replace anything, and every selection change is checked
//! against the selection's version in its transaction. The worker runs only
//! the automatic queue, outside the cycle lock (it touches neither the media
//! nor Transmission).
//!
//! # Limits
//!
//! The image limits hold for uploads and AniList images alike: at most
//! [`MAX_IMAGE_BYTES`] bytes, [`MAX_IMAGE_SIDE`] pixels a side and
//! [`MAX_IMAGE_PIXELS`] pixels, judged from the bytes' own header (see
//! [`image`]). Nothing is decoded, so the memory an image takes is its bytes
//! and not its pixels: a 12 MP image is no more than 10 MiB here.
//!
//! An image's bytes are held in one buffer from where they arrive to where
//! the file is published, and that buffer is shared ([`Bytes`]) with the
//! check and the file, never copied. An upload's body is collected into a
//! buffer reserved from `Content-Length` within [`MAX_IMAGE_BYTES`], and a
//! body that takes longer than [`UPLOAD_BODY_TIMEOUT`] to arrive is dropped
//! and gives its slot back. The cover of a picked AniList entry is fetched
//! into a buffer reserved once the same way, and must be as long as the
//! response said. At most [`UPLOAD_SLOTS`] uploads and picks are taken in at
//! once: each holds its slot from before the first byte arrives until the
//! file is published ([`Artwork::pick`], [`Artwork::upload`]).
//!
//! Serving is bounded by bytes. Before a file is read to serve or check it,
//! its recorded size is taken from [`SERVING_BUDGET`], and the bytes keep it
//! until the last [`Bytes`] made from them is dropped, which is when the
//! server has written the last byte out (even into a slow client's socket
//! buffer: the connection's write queue holds a clone) or dropped the
//! response (a client that went away). A file of 10 MiB waits for 10 MiB of
//! room; covers of a few hundred KB are served dozens at a time. A file
//! checked before is read again only when it changed ([`files::Verified`]).
//!
//! # Memory of the web process
//!
//! Counted together, the web process holds at most
//!
//! | what | bound | MiB |
//! | --- | --- | --- |
//! | uploads and picks | [`UPLOAD_SLOTS`] × [`MAX_IMAGE_BYTES`] | 20 |
//! | cover files read or being sent | [`SERVING_BUDGET`] | 32 |
//!
//! 52 MiB, plus about 14 MiB for an idle web process with an empty database:
//! 66 MiB against the container's 128 MiB limit, leaving about 62 MiB for the
//! rest (the database's pages, rule previews, requests in flight).
//!
//! Not counted: the kernel's socket buffers of a response, and the bytes an
//! upload body holds beyond a declared length (an upload without
//! `Content-Length` grows its buffer by doubling, up to about twice its size
//! while it does).
//!
//! # Memory of the worker process
//!
//! The worker takes no uploads and serves no files. Its artwork queue runs one
//! job at a time and holds the bytes it fetched ([`MAX_IMAGE_BYTES`], 10 MiB)
//! until the file is published. The cycle (feeds, Transmission), the command
//! loop and the directory watches come on top of it: a library of about 1,500
//! folders measured 29 MB resident with its watches (ticket 0016), so about
//! 40 MiB with a cover, far from 128 MiB.

pub mod anilist;
pub mod files;
pub mod image;
pub mod queue;
pub mod title;

use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub use anilist::{Anilist, AnilistConfig, AnilistError, ImageFetchError};
pub use files::{AppData, Unavailable};
pub use image::Rejected;

use crate::{
    store::{
        artwork::{ArtworkError, ArtworkStore, ImageRef, Selection, Source, UserChange},
        Db,
    },
    worker::{system_clock, Clock},
};

/// The largest image file accepted, uploaded or fetched: 10 MiB.
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
/// The longest side of an accepted image, in pixels.
pub const MAX_IMAGE_SIDE: u32 = 8192;
/// The most pixels an accepted image may have: 12 million (a 4000 × 3000
/// photo), as its header states.
pub const MAX_IMAGE_PIXELS: u64 = 12_000_000;
/// How long fetching one image may take in total.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a user's AniList request may wait for its turn before the web
/// answers that AniList is busy.
pub const USER_MAX_WAIT: Duration = Duration::from_secs(10);
/// How many uploads and picked covers a process takes in at once, from the
/// first byte to the published file: at most this many buffers of
/// [`MAX_IMAGE_BYTES`] are held.
pub const UPLOAD_SLOTS: usize = 2;
/// How long reading one upload's body may take in total, while it holds an
/// upload slot: a body that stalls is dropped and gives the slot back.
pub const UPLOAD_BODY_TIMEOUT: Duration = Duration::from_secs(60);
/// Bytes of image files a process holds at once to serve or check them: 32
/// MiB. A file takes its own size from the budget before it is read and gives
/// it back when its last byte has gone out of the process or the response was
/// dropped (see [`Artwork::image`]). Covers of a few hundred KB are served
/// dozens at a time; a file of [`MAX_IMAGE_BYTES`] waits for room.
pub const SERVING_BUDGET: usize = 32 * 1024 * 1024;

/// A file's bytes with the room they took from [`SERVING_BUDGET`]: the room
/// goes back when the last [`Bytes`] made from it is dropped.
struct Held {
    data: Vec<u8>,
    _room: OwnedSemaphorePermit,
}

impl AsRef<[u8]> for Held {
    fn as_ref(&self) -> &[u8] {
        &self.data
    }
}

/// Why a user's artwork action did not happen. Nothing was changed.
#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    #[error(transparent)]
    Store(#[from] ArtworkError),
    #[error(transparent)]
    Anilist(#[from] AnilistError),
    #[error(transparent)]
    Fetch(#[from] ImageFetchError),
    /// The bytes are not an accepted image.
    #[error("rejected: {0}")]
    Rejected(Rejected),
    /// AniList has no entry with this ID.
    #[error("no such AniList entry")]
    NoEntry,
    /// The entry has no usable cover.
    #[error("the AniList entry has no cover")]
    NoCover,
    /// The app data folder is not configured (only in tests and tools).
    #[error("no app data folder")]
    NoAppData,
    #[error("cannot store the image: {0}")]
    Publish(String),
}

impl From<files::PublishError> for ActionError {
    fn from(e: files::PublishError) -> Self {
        match e {
            files::PublishError::Store(e) => ActionError::Store(e),
            other => ActionError::Publish(other.to_string()),
        }
    }
}

/// The artwork services of one process. Cheap to clone.
#[derive(Clone)]
pub struct Artwork {
    pub store: ArtworkStore,
    pub anilist: Anilist,
    app_data: Option<AppData>,
    clock: Clock,
    uploads: Arc<Semaphore>,
    serving: Arc<Semaphore>,
    verified: Arc<files::Verified>,
    body_timeout: Duration,
}

impl Artwork {
    pub fn new(db: Db, app_data: Option<AppData>, config: AnilistConfig) -> Self {
        Self::with_clock(db, app_data, config, system_clock())
    }

    pub fn with_clock(
        db: Db,
        app_data: Option<AppData>,
        config: AnilistConfig,
        clock: Clock,
    ) -> Self {
        let store = ArtworkStore::new(db);
        Artwork {
            anilist: Anilist::new(config, store.clone(), clock.clone()),
            store,
            app_data,
            clock,
            uploads: Arc::new(Semaphore::new(UPLOAD_SLOTS)),
            serving: Arc::new(Semaphore::new(SERVING_BUDGET)),
            verified: Arc::default(),
            body_timeout: UPLOAD_BODY_TIMEOUT,
        }
    }

    /// Overrides how long an upload's body may take to arrive (tests).
    pub fn with_body_timeout(mut self, timeout: Duration) -> Self {
        self.body_timeout = timeout;
        self
    }

    /// How long reading one upload's body may take ([`UPLOAD_BODY_TIMEOUT`]).
    pub fn body_timeout(&self) -> Duration {
        self.body_timeout
    }

    /// Overrides the time between AniList requests (tests).
    pub fn with_spacing(mut self, spacing: Duration) -> Self {
        self.anilist = self.anilist.with_spacing(spacing);
        self
    }

    pub fn app_data(&self) -> Option<&AppData> {
        self.app_data.as_ref()
    }

    pub fn now(&self) -> i64 {
        (self.clock)()
    }

    /// A turn to take in one upload ([`UPLOAD_SLOTS`]); the web takes it before
    /// it reads the body and hands it to [`Artwork::upload`], which keeps it
    /// until the file is stored.
    pub async fn upload_slot(&self) -> OwnedSemaphorePermit {
        self.uploads
            .clone()
            .acquire_owned()
            .await
            .expect("never closed")
    }

    /// Judges (header only, see [`image::verify`]) and publishes `bytes`, returning the reference a selection
    /// takes. The file is `staging` until then.
    ///
    /// It runs as its own task to the end, holding `hold` (an upload's slot)
    /// until then: a caller that stops waiting (a client that went away) leaves
    /// no half-made file, only a published one no selection takes, which the
    /// recovery hands to the cleanup.
    pub async fn store_image(
        &self,
        bytes: Bytes,
        origin: Source,
        hold: Option<OwnedSemaphorePermit>,
    ) -> Result<ImageRef, ActionError> {
        let app = self.app_data.clone().ok_or(ActionError::NoAppData)?;
        let this = self.clone();
        tokio::spawn(async move {
            let _hold = hold;
            let format = image::verify(&bytes).map_err(ActionError::Rejected)?;
            let path = files::publish(&app, &this.store, bytes.clone(), format, this.now()).await?;
            Ok(files::image_ref(origin, path, &bytes, format))
        })
        .await
        .unwrap_or_else(|e| Err(ActionError::Publish(e.to_string())))
    }

    /// Fails with a conflict before any work when the selection is not at
    /// `expected` any more.
    async fn check_version(&self, work_id: &str, expected: i64) -> Result<(), ActionError> {
        let current = self.store.selection(work_id).await?;
        if current.version != expected {
            return Err(ArtworkError::Conflict(Box::new(current)).into());
        }
        Ok(())
    }

    /// Takes a just stored image as the user's choice, or leaves it to the
    /// cleanup when the selection moved on meanwhile.
    async fn take(
        &self,
        work_id: &str,
        expected: i64,
        anilist_media_id: Option<i64>,
        image: ImageRef,
    ) -> Result<Selection, ActionError> {
        let path = image.relative_path.clone();
        match self
            .store
            .select_manual(work_id, expected, anilist_media_id, image)
            .await
        {
            Ok(selection) => {
                self.tidy().await;
                Ok(selection)
            }
            Err(e) => {
                let _ = self.store.abandon_file(&path).await;
                self.tidy().await;
                Err(e.into())
            }
        }
    }

    /// The user's uploaded file becomes the work's cover (`manual`, upload).
    /// `slot` is the upload's turn ([`Artwork::upload_slot`]), taken before the
    /// body was read; without one, one is taken here.
    pub async fn upload(
        &self,
        work_id: &str,
        expected: i64,
        bytes: impl Into<Bytes>,
        slot: Option<OwnedSemaphorePermit>,
    ) -> Result<Selection, ActionError> {
        let slot = match slot {
            Some(slot) => slot,
            None => self.upload_slot().await,
        };
        self.check_version(work_id, expected).await?;
        let image = self
            .store_image(bytes.into(), Source::Upload, Some(slot))
            .await?;
        self.take(work_id, expected, None, image).await
    }

    /// The AniList entry the user picked becomes the work's cover (`manual`,
    /// AniList). The image URL comes from AniList's own answer for the ID.
    pub async fn pick(
        &self,
        work_id: &str,
        expected: i64,
        anilist_media_id: i64,
    ) -> Result<Selection, ActionError> {
        self.check_version(work_id, expected).await?;
        if self.app_data.is_none() {
            return Err(ActionError::NoAppData);
        }
        let entry = self
            .anilist
            .media(anilist_media_id, Some(USER_MAX_WAIT))
            .await?
            .ok_or(ActionError::NoEntry)?;
        let url = entry.cover_url.ok_or(ActionError::NoCover)?;
        // Like an upload, the cover is held from the first byte fetched to the
        // published file, so picks and uploads together stay within the slots.
        let slot = self.upload_slot().await;
        let bytes = self.anilist.fetch_image(&url).await?;
        let image = self
            .store_image(bytes.into(), Source::Anilist, Some(slot))
            .await?;
        self.take(work_id, expected, Some(anilist_media_id), image)
            .await
    }

    /// Clears the cover, goes back to `auto`, or asks for the image again.
    pub async fn change(
        &self,
        work_id: &str,
        expected: i64,
        change: UserChange,
    ) -> Result<Selection, ActionError> {
        let selection = self
            .store
            .change(work_id, expected, change, self.now())
            .await?;
        self.tidy().await;
        Ok(selection)
    }

    /// The bytes of the work's current image, checked (see [`files::check`]).
    ///
    /// The bytes keep their size of the [`SERVING_BUDGET`] for as long as any
    /// clone of the returned [`Bytes`] lives: a response body made from them
    /// gives the room back when the server has written the last byte out, or
    /// drops the body (a client that went away, a body never polled).
    pub async fn image(&self, image: ImageRef) -> Result<Bytes, Unavailable> {
        self.check(image, true).await.map(Option::unwrap_or_default)
    }

    /// The part of the [`SERVING_BUDGET`] not taken now (tests).
    #[cfg(test)]
    pub(crate) fn serving_free(&self) -> usize {
        self.serving.available_permits()
    }

    /// Whether the image's file is the one recorded, without reading it again
    /// when it did not change since it was last checked.
    pub async fn image_state(&self, image: ImageRef) -> Result<(), Unavailable> {
        self.check(image, false).await.map(|_| ())
    }

    /// Checks on a blocking thread. The file's recorded size is taken from the
    /// [`SERVING_BUDGET`] first (a file larger than the budget takes all of
    /// it), so only so many bytes of files are read or held at once. A check
    /// keeps its room until it ends, even when the caller stops waiting; the
    /// bytes it reads carry the room on to the caller.
    async fn check(&self, image: ImageRef, read: bool) -> Result<Option<Bytes>, Unavailable> {
        let Some(app) = self.app_data.clone() else {
            return Err(Unavailable::Unverified);
        };
        let room = image.byte_size.clamp(1, SERVING_BUDGET as u64) as u32;
        let permit = self
            .serving
            .clone()
            .acquire_many_owned(room)
            .await
            .expect("never closed");
        let verified = self.verified.clone();
        tokio::task::spawn_blocking(move || {
            files::check(&app, &image, &verified, read).map(|bytes| {
                bytes.map(|data| {
                    Bytes::from_owner(Held {
                        data,
                        _room: permit,
                    })
                })
            })
        })
        .await
        .unwrap_or(Err(Unavailable::Unverified))
    }

    /// Removes the image files nothing refers to any more; failures are logged.
    pub async fn tidy(&self) {
        let Some(app) = &self.app_data else { return };
        match files::cleanup(app, &self.store).await {
            Ok(cleaned) if cleaned.unsure => {
                eprintln!("Artwork cleanup left files it could not judge");
            }
            Ok(_) => {}
            Err(e) => eprintln!("Artwork cleanup failed: {e}"),
        }
    }
}

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod tests;
