//! Work artwork: which cover each work has and the image files the app made
//! for them (`docs/specs/library.md`, 작품 표지; `docs/specs/settings.md`, 작품
//! 표지 필드). The rules that decide, fetch and verify live in
//! [`crate::artwork`]; this store keeps the state and its invariants.
//!
//! # The selection and its version
//!
//! Every work has one [`Selection`]: a [`Mode`] (`auto`, `manual`,
//! `disabled`), what is selected (an AniList ID or an uploaded file) and the
//! current image's reference ([`ImageRef`]). The table's checks hold the
//! invariants of the settings spec (no `manual` without an image, no `auto`
//! upload, nothing selected while `disabled`, ...).
//!
//! The selection's `version` grows with every change of what is selected (an
//! image arriving for the selection as it is, or a job being asked for, is no
//! such change). A
//! user change names the version it was made from and fails with
//! [`ArtworkError::Conflict`], changing nothing, when someone changed the
//! selection first. An automatic result names the version its job was taken
//! at and is dropped the same way, so a late search or image never undoes a
//! user's newer choice or `disabled`.
//!
//! # Jobs
//!
//! [`JobKind::Search`] and [`JobKind::Fetch`] are the automatic work the worker
//! still has to do for a work. A work recorded for the first time gets a
//! search (a trigger on `works` does it, so no scan code has to), and so does a
//! user going back to `auto`; a user's repair asks for the selected ID's image
//! again. Nothing else creates one: a rescan, a restart or opening a screen
//! does not. Jobs persist, so a restart resumes the queue.
//!
//! # Files
//!
//! The image files are recorded in `artwork_files` before they exist, so the
//! app removes only files it can show it made (see [`crate::artwork::files`]).

mod repo;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use rusqlite::Connection;

pub use repo::FileRow;

use super::{
    db::{Db, DbError},
    history::Millis,
};

/// How a work's cover is chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The app selects a clear AniList match by itself.
    Auto,
    /// The user selected an AniList entry or uploaded a file.
    Manual,
    /// The user cleared the cover: no cover and no automatic search.
    Disabled,
}

impl Mode {
    pub fn code(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::Manual => "manual",
            Mode::Disabled => "disabled",
        }
    }

    pub fn from_code(code: &str) -> Option<Mode> {
        match code {
            "auto" => Some(Mode::Auto),
            "manual" => Some(Mode::Manual),
            "disabled" => Some(Mode::Disabled),
            _ => None,
        }
    }
}

/// Where the selected cover comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Anilist,
    Upload,
}

impl Source {
    pub fn code(self) -> &'static str {
        match self {
            Source::Anilist => "anilist",
            Source::Upload => "upload",
        }
    }

    pub fn from_code(code: &str) -> Option<Source> {
        match code {
            "anilist" => Some(Source::Anilist),
            "upload" => Some(Source::Upload),
            _ => None,
        }
    }
}

/// An image format the app accepts, judged from the bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
    Webp,
}

impl Format {
    pub fn code(self) -> &'static str {
        match self {
            Format::Jpeg => "jpeg",
            Format::Png => "png",
            Format::Webp => "webp",
        }
    }

    pub fn from_code(code: &str) -> Option<Format> {
        match code {
            "jpeg" => Some(Format::Jpeg),
            "png" => Some(Format::Png),
            "webp" => Some(Format::Webp),
            _ => None,
        }
    }

    /// The `Content-Type` the image is served with.
    pub fn mime(self) -> &'static str {
        match self {
            Format::Jpeg => "image/jpeg",
            Format::Png => "image/png",
            Format::Webp => "image/webp",
        }
    }

    /// The extension of the files the app makes.
    pub fn extension(self) -> &'static str {
        match self {
            Format::Jpeg => "jpg",
            Format::Png => "png",
            Format::Webp => "webp",
        }
    }
}

/// The current image of a work: what the file must be, not a claim that it is
/// there. Serving checks the file against it every time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    /// Issued by the app; never a file name or a provider ID.
    pub id: String,
    pub origin: Source,
    /// Relative to the app data folder.
    pub relative_path: String,
    pub byte_size: u64,
    /// Lowercase hex.
    pub sha256: String,
    pub format: Format,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    /// Look the work's folder name up on AniList and select a clear match.
    Search,
    /// Receive the image of the selected AniList ID.
    Fetch,
}

impl JobKind {
    pub fn code(self) -> &'static str {
        match self {
            JobKind::Search => "search",
            JobKind::Fetch => "fetch",
        }
    }

    fn from_code(code: &str) -> Option<JobKind> {
        match code {
            "search" => Some(JobKind::Search),
            "fetch" => Some(JobKind::Fetch),
            _ => None,
        }
    }
}

/// Automatic work still to do for a work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub kind: JobKind,
    pub requested_at: Millis,
    /// Failed attempts so far.
    pub attempts: u32,
    /// Not to be tried before this time.
    pub not_before: Option<Millis>,
}

/// Why the last automatic search or fetch left the cover as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Note {
    /// No AniList title equals the folder name.
    NoMatch,
    /// More than one AniList entry has a title equal to the folder name.
    Ambiguous,
    /// The search had more results than the app reads, so a match could not
    /// be known to be the only one.
    Incomplete,
    /// AniList could not be reached or answered something unusable, again and
    /// again.
    Failed,
    /// AniList has no entry with the selected ID any more.
    Gone,
    /// The selected entry has no cover image.
    NoImage,
    /// The provider's image was not an image the app accepts.
    Rejected,
}

impl Note {
    pub fn code(self) -> &'static str {
        match self {
            Note::NoMatch => "no_match",
            Note::Ambiguous => "ambiguous",
            Note::Incomplete => "incomplete",
            Note::Failed => "failed",
            Note::Gone => "gone",
            Note::NoImage => "no_image",
            Note::Rejected => "rejected",
        }
    }

    fn from_code(code: &str) -> Option<Note> {
        [
            Note::NoMatch,
            Note::Ambiguous,
            Note::Incomplete,
            Note::Failed,
            Note::Gone,
            Note::NoImage,
            Note::Rejected,
        ]
        .into_iter()
        .find(|n| n.code() == code)
    }

    /// The note as a sentence for the screen.
    pub fn message(self) -> &'static str {
        match self {
            Note::NoMatch => "AniList에서 폴더 이름과 제목이 같은 작품을 찾지 못해 비워 뒀어요.",
            Note::Ambiguous => {
                "AniList에 폴더 이름과 제목이 같은 작품이 여럿이라 비워 뒀어요. 직접 골라 주세요."
            }
            Note::Incomplete => {
                "AniList 검색 결과가 너무 많아 하나뿐인지 확인하지 못해 비워 뒀어요. 직접 골라 주세요."
            }
            Note::Failed => "AniList에 여러 번 연결하지 못해 표지를 정하지 못했어요.",
            Note::Gone => "고른 AniList 작품을 더 이상 찾을 수 없어요.",
            Note::NoImage => "고른 AniList 작품에 표지 이미지가 없어요.",
            Note::Rejected => "AniList의 표지 이미지를 확인하지 못해 받지 않았어요.",
        }
    }
}

/// A work's cover selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub work_id: String,
    pub mode: Mode,
    pub source: Option<Source>,
    pub anilist_media_id: Option<i64>,
    pub image: Option<ImageRef>,
    pub version: i64,
    pub job: Option<Job>,
    pub note: Option<Note>,
}

/// A job the worker took, with what it needs to run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedJob {
    pub work_id: String,
    /// The work's folder name: the title observation the search compares.
    pub dir_name: String,
    /// The selection's version when the job was taken.
    pub version: i64,
    pub kind: JobKind,
    pub anilist_media_id: Option<i64>,
    /// The cover URL a verified search answer gave (fetch only).
    pub image_url: Option<String>,
    pub attempts: u32,
}

/// What a search came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Searched {
    /// One clear match: it is selected, and its image fetched next.
    Selected {
        anilist_media_id: i64,
        image_url: Option<String>,
    },
    /// Nothing selected, for this reason.
    Left(Note),
}

/// A change the user asks for that needs no new image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserChange {
    /// No cover, and no automatic search: `disabled`.
    Clear,
    /// Back to `auto` with nothing selected, and a new search.
    Auto,
    /// Receive the selected AniList ID's image again.
    Repair,
}

#[derive(Debug, thiserror::Error)]
pub enum ArtworkError {
    #[error(transparent)]
    Db(#[from] DbError),
    /// No work with this ID.
    #[error("no such work")]
    NotFound,
    /// The selection is not at the version the change was made from; nothing
    /// was changed. Carries the current selection.
    #[error("the artwork selection was changed by someone else")]
    Conflict(Box<Selection>),
    /// The change does not apply to the selection as it is.
    #[error("{0}")]
    Invalid(&'static str),
    /// The staged file this change would refer to is not on record any more
    /// (the app gave up on it); nothing was changed.
    #[error("the staged image is not on record any more")]
    Interrupted,
}

impl From<rusqlite::Error> for ArtworkError {
    fn from(e: rusqlite::Error) -> Self {
        ArtworkError::Db(DbError::Sqlite(e))
    }
}

/// Async access to the artwork state. Cheap to clone.
#[derive(Clone)]
pub struct ArtworkStore {
    db: Db,
}

impl ArtworkStore {
    pub fn new(db: Db) -> Self {
        ArtworkStore { db }
    }

    /// Runs `f` on the connection (for the file handling in
    /// [`crate::artwork::files`], which must hold a transaction while it
    /// removes a file).
    pub(crate) async fn run<T, F>(&self, f: F) -> Result<T, ArtworkError>
    where
        F: FnOnce(&mut Connection) -> Result<T, ArtworkError> + Send + 'static,
        T: Send + 'static,
    {
        self.db.run(f).await
    }

    /// The selection of work `work_id`; [`ArtworkError::NotFound`] when there
    /// is no such work.
    pub async fn selection(&self, work_id: &str) -> Result<Selection, ArtworkError> {
        let id = work_id.to_owned();
        self.db.run(move |c| repo::selection(c, &id)).await
    }

    /// The image ID of every work that has an image reference, by work ID.
    pub async fn image_ids(&self) -> Result<HashMap<String, String>, ArtworkError> {
        self.db.run(|c| Ok(repo::image_ids(c)?)).await
    }

    /// Applies a user's change made from `expected` (see [`UserChange`]).
    pub async fn change(
        &self,
        work_id: &str,
        expected: i64,
        change: UserChange,
        now: Millis,
    ) -> Result<Selection, ArtworkError> {
        let id = work_id.to_owned();
        self.db
            .run(move |c| repo::change(c, &id, expected, change, now))
            .await
    }

    /// Makes the user's choice the work's cover: `manual`, selecting
    /// `anilist_media_id` (`source` AniList) or the uploaded file, with
    /// `image`, whose file is the staged one recorded at `image.relative_path`.
    pub async fn select_manual(
        &self,
        work_id: &str,
        expected: i64,
        anilist_media_id: Option<i64>,
        image: ImageRef,
    ) -> Result<Selection, ArtworkError> {
        let id = work_id.to_owned();
        self.db
            .run(move |c| repo::select_manual(c, &id, expected, anilist_media_id, &image))
            .await
    }

    /// The next job that may run at `now`, oldest request first.
    pub async fn next_job(&self, now: Millis) -> Result<Option<ClaimedJob>, ArtworkError> {
        self.db.run(move |c| Ok(repo::next_job(c, now)?)).await
    }

    /// Records a search's outcome, unless the selection changed since the job
    /// was taken at `version` or the search is no longer wanted. Whether it
    /// was recorded.
    pub async fn searched(
        &self,
        work_id: &str,
        version: i64,
        outcome: Searched,
        now: Millis,
    ) -> Result<bool, ArtworkError> {
        let id = work_id.to_owned();
        self.db
            .run(move |c| Ok(repo::searched(c, &id, version, &outcome, now)?))
            .await
    }

    /// Makes `image` the image of the AniList selection the fetch job was
    /// taken for, unless the selection changed since `version`. Whether it was
    /// recorded; when not, the staged file is left to the cleanup.
    pub async fn fetched(
        &self,
        work_id: &str,
        version: i64,
        anilist_media_id: i64,
        image: ImageRef,
    ) -> Result<bool, ArtworkError> {
        let id = work_id.to_owned();
        self.db
            .run(move |c| repo::fetched(c, &id, version, anilist_media_id, &image))
            .await
    }

    /// The job taken at `version` did not finish: try again at `retry_at`
    /// (counting a failed attempt when `failed`), or, with `retry_at` `None`,
    /// give up and leave `note`.
    pub async fn job_later(
        &self,
        work_id: &str,
        version: i64,
        retry_at: Option<Millis>,
        failed: bool,
        note: Note,
        now: Millis,
    ) -> Result<(), ArtworkError> {
        let id = work_id.to_owned();
        self.db
            .run(move |c| {
                Ok(repo::job_later(
                    c, &id, version, retry_at, failed, note, now,
                )?)
            })
            .await
    }

    /// Records the files the app is about to make: `relative_path` (where it
    /// will be published) and `staging_path`. Must come before the staged
    /// file exists.
    pub async fn reserve_file(
        &self,
        relative_path: &str,
        staging_path: &str,
        now: Millis,
    ) -> Result<(), ArtworkError> {
        let (r, s) = (relative_path.to_owned(), staging_path.to_owned());
        self.db
            .run(move |c| Ok(repo::reserve_file(c, &r, &s, now)?))
            .await
    }

    /// Records the staged file's identity, which the rename to the published
    /// path keeps.
    pub async fn file_identity(
        &self,
        relative_path: &str,
        dev: u64,
        ino: u64,
    ) -> Result<(), ArtworkError> {
        let r = relative_path.to_owned();
        self.db
            .run(move |c| Ok(repo::file_identity(c, &r, dev, ino)?))
            .await
    }

    /// Forgets a reserved file that was never published (removed or never made).
    pub async fn forget_file(&self, relative_path: &str) -> Result<(), ArtworkError> {
        let r = relative_path.to_owned();
        self.db.run(move |c| Ok(repo::forget_file(c, &r)?)).await
    }

    /// Hands a published file that no selection took to the cleanup.
    pub async fn abandon_file(&self, relative_path: &str) -> Result<(), ArtworkError> {
        let r = relative_path.to_owned();
        self.db.run(move |c| Ok(repo::abandon_file(c, &r)?)).await
    }

    /// Takes the next slot for an AniList request at or after `now`, keeping
    /// `spacing_ms` between requests of both processes. `Err(wait)` without
    /// taking one when the slot is more than `max_wait_ms` away.
    pub async fn take_request_slot(
        &self,
        now: Millis,
        spacing_ms: i64,
        max_wait_ms: Option<i64>,
    ) -> Result<Result<Millis, i64>, ArtworkError> {
        self.db
            .run(move |c| Ok(repo::take_slot(c, now, spacing_ms, max_wait_ms)?))
            .await
    }

    /// AniList asked for no request before `until`.
    pub async fn block_requests(&self, until: Millis) -> Result<(), ArtworkError> {
        self.db.run(move |c| Ok(repo::block(c, until)?)).await
    }
}

pub(crate) use repo::{delete_file_row, files_of_state, new_id, referenced_paths};
