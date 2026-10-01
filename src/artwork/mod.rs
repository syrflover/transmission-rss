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
//! The image limits hold for uploads and AniList images alike. They are set
//! for the containers' 128M memory limit: before an image is decoded, what the
//! decode would allocate (output, the JPEG decoder's input copy and a
//! progressive JPEG's coefficients, the WebP decoder's frame) is added up from
//! its headers and must stay under [`DECODE_MAX_ALLOC`] (see [`image`]), and
//! one decode runs at a time in a process, keeping its turn until it ends even
//! when its caller went away. The bytes are shared ([`Bytes`]), never copied,
//! from the upload's body to the decode and the file, and at most
//! [`UPLOAD_SLOTS`] uploads are taken in at once.

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
        artwork::{ArtworkError, ArtworkStore, Format, ImageRef, Selection, Source, UserChange},
        Db,
    },
    worker::{system_clock, Clock},
};

/// The largest image file accepted, uploaded or fetched: 10 MiB.
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
/// The longest side of an accepted image, in pixels.
pub const MAX_IMAGE_SIDE: u32 = 8192;
/// The most pixels an accepted image may have: 12 million (a 4000 × 3000
/// photo). Within it, [`DECODE_MAX_ALLOC`] still refuses the kinds that cost
/// more to decode (16 bits a channel, large progressive JPEGs).
pub const MAX_IMAGE_PIXELS: u64 = 12_000_000;
/// The most memory one decode may allocate, all buffers counted: 64 MiB. A
/// 12 MP baseline JPEG (36 MB of RGB) or 8-bit RGBA PNG (48 MB) fits; a 12 MP
/// 16-bit RGBA PNG (96 MB) or progressive JPEG (72 MB and more) does not.
pub const DECODE_MAX_ALLOC: u64 = 64 * 1024 * 1024;
/// How long fetching one image may take in total.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a user's AniList request may wait for its turn before the web
/// answers that AniList is busy.
pub const USER_MAX_WAIT: Duration = Duration::from_secs(10);
/// How many uploads a process takes in at once, from reading the body to the
/// published file: at most this many bodies of [`MAX_IMAGE_BYTES`] are held.
pub const UPLOAD_SLOTS: usize = 2;

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
    decoding: Arc<Semaphore>,
    uploads: Arc<Semaphore>,
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
            decoding: Arc::new(Semaphore::new(1)),
            uploads: Arc::new(Semaphore::new(UPLOAD_SLOTS)),
        }
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

    /// Verifies `bytes` on a blocking thread, one decode at a time. The decode
    /// keeps its turn until it ends, even when the caller stops waiting.
    pub async fn verify(&self, bytes: Bytes) -> Result<Format, Rejected> {
        let permit = self
            .decoding
            .clone()
            .acquire_owned()
            .await
            .expect("never closed");
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            image::verify(&bytes)
        })
        .await
        .unwrap_or(Err(Rejected::Damaged))
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

    /// Verifies and publishes `bytes`, returning the reference a selection
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
            let format = this
                .verify(bytes.clone())
                .await
                .map_err(ActionError::Rejected)?;
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
        let bytes = self.anilist.fetch_image(&url).await?;
        let image = self
            .store_image(bytes.into(), Source::Anilist, None)
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

    /// The bytes of the work's current image, checked (see
    /// [`files::read_verified`]).
    pub async fn image(&self, image: ImageRef) -> Result<Vec<u8>, Unavailable> {
        let Some(app) = self.app_data.clone() else {
            return Err(Unavailable::Unverified);
        };
        tokio::task::spawn_blocking(move || files::read_verified(&app, &image))
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
