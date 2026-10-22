//! The cover of a subscription that is not connected to a season yet
//! (`docs/requirements/library.md`, 작품 표지; the table is
//! `subscription_artwork`). Until its rule receives a video the subscription
//! has no work, so the schedule's card and the rule's poster show this cover
//! instead.
//!
//! - The subscription's maker asks for it in its own transaction
//!   ([`request_in`]): a `search` of the Anissia anime's original title, which
//!   becomes a `fetch` of the one entry it selects, as a work's does. Imports
//!   ask for none.
//! - It ends with the subscription (`ON DELETE CASCADE`; the cleanup then
//!   removes its file), or when the subscription is connected to a season
//!   ([`adopt_in`]), which hands the image to the work whose cover has none
//!   yet.
//! - A job's result is kept only while the row still waits for that same job
//!   (its kind and `job_requested_at`): there is no user change to compare a
//!   version with, only the row ending.

use std::collections::HashMap;

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use super::{repo, ArtworkError, ArtworkStore, Format, ImageRef, JobKind, Note, Searched, Source};
use trss_core::Millis;

/// A subscription's cover job the worker took, with what it needs to run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionJob {
    pub rule_id: String,
    /// The Anissia anime's original title (`originalSubject`): what the search
    /// looks up and compares.
    pub original: Option<String>,
    /// Its start date as Anissia writes it (`YYYY-MM-DD`).
    pub start_date: Option<String>,
    /// When the job was asked for: which job of the row this is.
    pub requested_at: Millis,
    pub kind: JobKind,
    pub anilist_media_id: Option<i64>,
    /// The cover URL the verified search answer gave (fetch only).
    pub image_url: Option<String>,
    pub attempts: u32,
}

/// Asks for rule `rule_id`'s subscription cover: a search, to run after the
/// caller's transaction (which made the subscription) commits.
pub fn request_in(tx: &Transaction<'_>, rule_id: &str, now: Millis) -> rusqlite::Result<()> {
    tx.prepare_cached(
        "INSERT INTO subscription_artwork (rule_id, job, job_requested_at) VALUES (?1, 'search', ?2)
         ON CONFLICT (rule_id) DO NOTHING",
    )?
    .execute(params![rule_id, now])?;
    Ok(())
}

/// Ends rule `rule_id`'s subscription cover inside the transaction that
/// connects the subscription to a season, handing it to the season's work
/// `work_id` (`None` when the library has no such work; a work of an
/// unregistered watch folder takes nothing, as it is not changed) when that
/// work's cover is `auto` with nothing selected and the subscription has an
/// image: the
/// image becomes the work's, selecting its AniList entry (a new version, so
/// the work's own search still running drops its result). A cover not handed
/// over leaves its file to the cleanup. Whether the work took the image.
pub fn adopt_in(
    tx: &Transaction<'_>,
    rule_id: &str,
    work_id: Option<&str>,
) -> rusqlite::Result<bool> {
    let work_id = match work_id {
        Some(work_id) if repo::in_library(tx, work_id)? => Some(work_id),
        _ => None,
    };
    let mut adopted = false;
    if let Some(work_id) = work_id {
        // A work recorded before its row could exist gets the unselected
        // `auto` row, as the store's own reads do.
        tx.prepare_cached(
            "INSERT OR IGNORE INTO work_artwork (work_id, mode)
             SELECT id, 'auto' FROM works WHERE id = ?1",
        )?
        .execute([work_id])?;
        adopted = tx
            .prepare_cached(
                "UPDATE work_artwork SET source = 'anilist', anilist_media_id = s.anilist_media_id,
                     image_id = s.image_id, image_origin = 'anilist', image_path = s.image_path,
                     image_size = s.image_size, image_sha256 = s.image_sha256,
                     image_format = s.image_format, version = version + 1, job = NULL,
                     job_requested_at = NULL, job_attempts = 0, job_not_before = NULL,
                     job_image_url = NULL, note = NULL, note_at = NULL
                   FROM (SELECT * FROM subscription_artwork
                          WHERE rule_id = ?1 AND image_id IS NOT NULL) AS s
                  WHERE work_artwork.work_id = ?2 AND work_artwork.mode = 'auto'
                    AND work_artwork.source IS NULL AND work_artwork.image_id IS NULL",
            )?
            .execute(params![rule_id, work_id])?
            > 0;
    }
    tx.prepare_cached("DELETE FROM subscription_artwork WHERE rule_id = ?1")?
        .execute([rule_id])?;
    Ok(adopted)
}

fn next_job(conn: &Connection, now: Millis) -> rusqlite::Result<Option<SubscriptionJob>> {
    conn.prepare_cached(
        "SELECT s.rule_id, a.original_subject, a.start_date, s.job, s.anilist_media_id,
                s.job_image_url, s.job_attempts, s.job_requested_at
           FROM subscription_artwork s
           JOIN rule_subscriptions r ON r.rule_id = s.rule_id
           JOIN anissia_anime a ON a.anime_no = r.anissia_anime_no
          WHERE s.job IS NOT NULL AND (s.job_not_before IS NULL OR s.job_not_before <= ?1)
          ORDER BY s.job = 'search', s.job_requested_at, s.rule_id
          LIMIT 1",
    )?
    .query_row([now], |row| {
        let kind: String = row.get(3)?;
        Ok(SubscriptionJob {
            rule_id: row.get(0)?,
            original: row.get(1)?,
            start_date: row.get(2)?,
            kind: JobKind::from_code(&kind).ok_or_else(|| repo::bad(3, "job"))?,
            anilist_media_id: row.get(4)?,
            image_url: row.get(5)?,
            attempts: row.get::<_, i64>(6)?.max(0) as u32,
            requested_at: row.get(7)?,
        })
    })
    .optional()
}

/// Whether rule `rule_id`'s cover still waits for the job of `kind` asked for
/// at `requested_at`, and the entry that job is for.
fn job_stands(
    tx: &Transaction<'_>,
    rule_id: &str,
    kind: JobKind,
    requested_at: Millis,
) -> rusqlite::Result<Option<Option<i64>>> {
    tx.prepare_cached(
        "SELECT anilist_media_id FROM subscription_artwork
          WHERE rule_id = ?1 AND job = ?2 AND job_requested_at = ?3",
    )?
    .query_row(params![rule_id, kind.code(), requested_at], |r| r.get(0))
    .optional()
}

fn searched(
    conn: &mut Connection,
    rule_id: &str,
    requested_at: Millis,
    outcome: &Searched,
    now: Millis,
) -> rusqlite::Result<bool> {
    let tx = repo::begin(conn)?;
    if job_stands(&tx, rule_id, JobKind::Search, requested_at)?.is_none() {
        return Ok(false);
    }
    match outcome {
        Searched::Selected {
            anilist_media_id,
            image_url,
        } => {
            tx.prepare_cached(
                "UPDATE subscription_artwork SET anilist_media_id = ?2, job = 'fetch',
                     job_requested_at = ?3, job_attempts = 0, job_not_before = NULL,
                     job_image_url = ?4, note = NULL, note_at = NULL
                 WHERE rule_id = ?1",
            )?
            .execute(params![rule_id, anilist_media_id, now, image_url])?;
        }
        Searched::Left(note) => {
            tx.prepare_cached(
                "UPDATE subscription_artwork SET job = NULL, job_requested_at = NULL,
                     job_attempts = 0, job_not_before = NULL, job_image_url = NULL,
                     note = ?2, note_at = ?3
                 WHERE rule_id = ?1",
            )?
            .execute(params![rule_id, note.code(), now])?;
        }
    }
    tx.commit()?;
    Ok(true)
}

fn fetched(
    conn: &mut Connection,
    rule_id: &str,
    requested_at: Millis,
    anilist_media_id: i64,
    image: &ImageRef,
) -> rusqlite::Result<bool> {
    let tx = repo::begin(conn)?;
    let applies = job_stands(&tx, rule_id, JobKind::Fetch, requested_at)?
        == Some(Some(anilist_media_id))
        && repo::staged(&tx, &image.relative_path)?;
    if applies {
        tx.prepare_cached(
            "UPDATE subscription_artwork SET image_id = ?2, image_path = ?3, image_size = ?4,
                 image_sha256 = ?5, image_format = ?6, job = NULL, job_requested_at = NULL,
                 job_attempts = 0, job_not_before = NULL, job_image_url = NULL, note = NULL,
                 note_at = NULL
             WHERE rule_id = ?1",
        )?
        .execute(params![
            rule_id,
            image.id,
            image.relative_path,
            image.byte_size as i64,
            image.sha256,
            image.format.code(),
        ])?;
    }
    // Taken or not, the file is published: one nobody refers to (the
    // subscription ended meanwhile) is the cleanup's to remove.
    tx.prepare_cached("UPDATE artwork_files SET state = 'published' WHERE relative_path = ?1")?
        .execute([&image.relative_path])?;
    tx.commit()?;
    Ok(applies)
}

#[allow(clippy::too_many_arguments)]
fn job_later(
    conn: &mut Connection,
    rule_id: &str,
    kind: JobKind,
    requested_at: Millis,
    retry_at: Option<Millis>,
    failed: bool,
    note: Note,
    now: Millis,
) -> rusqlite::Result<()> {
    let tx = repo::begin(conn)?;
    if job_stands(&tx, rule_id, kind, requested_at)?.is_none() {
        return Ok(());
    }
    match retry_at {
        Some(at) => {
            tx.prepare_cached(
                "UPDATE subscription_artwork SET job_not_before = ?2,
                     job_attempts = job_attempts + ?3
                 WHERE rule_id = ?1",
            )?
            .execute(params![rule_id, at, failed as i64])?;
        }
        None => {
            tx.prepare_cached(
                "UPDATE subscription_artwork SET job = NULL, job_requested_at = NULL,
                     job_attempts = 0, job_not_before = NULL, job_image_url = NULL,
                     note = ?2, note_at = ?3
                 WHERE rule_id = ?1",
            )?
            .execute(params![rule_id, note.code(), now])?;
        }
    }
    tx.commit()
}

fn image(conn: &Connection, rule_id: &str) -> rusqlite::Result<Option<ImageRef>> {
    conn.prepare_cached(
        "SELECT image_id, image_path, image_size, image_sha256, image_format
           FROM subscription_artwork WHERE rule_id = ?1 AND image_id IS NOT NULL",
    )?
    .query_row([rule_id], |row| {
        let format: String = row.get(4)?;
        Ok(ImageRef {
            id: row.get(0)?,
            origin: Source::Anilist,
            relative_path: row.get(1)?,
            byte_size: row.get::<_, i64>(2)?.max(0) as u64,
            sha256: row.get(3)?,
            format: Format::from_code(&format).ok_or_else(|| repo::bad(4, "format"))?,
        })
    })
    .optional()
}

fn image_ids_of(
    conn: &Connection,
    rule_ids: &[String],
) -> rusqlite::Result<HashMap<String, String>> {
    let mut stmt = conn.prepare_cached(
        "SELECT image_id FROM subscription_artwork WHERE rule_id = ?1 AND image_id IS NOT NULL",
    )?;
    let mut out = HashMap::new();
    for id in rule_ids {
        if let Some(image_id) = stmt.query_row([id], |r| r.get(0)).optional()? {
            out.insert(id.clone(), image_id);
        }
    }
    Ok(out)
}

impl ArtworkStore {
    /// The next subscription cover job that may run at `now`: fetches first,
    /// then the oldest request.
    pub async fn next_subscription_job(
        &self,
        now: Millis,
    ) -> Result<Option<SubscriptionJob>, ArtworkError> {
        self.db.run(move |c| Ok(next_job(c, now)?)).await
    }

    /// Records a subscription search's outcome, unless the subscription's
    /// cover no longer waits for that search (the subscription ended or was
    /// connected to a season). Whether it was recorded.
    pub async fn subscription_searched(
        &self,
        rule_id: &str,
        requested_at: Millis,
        outcome: Searched,
        now: Millis,
    ) -> Result<bool, ArtworkError> {
        let id = rule_id.to_owned();
        self.db
            .run(move |c| Ok(searched(c, &id, requested_at, &outcome, now)?))
            .await
    }

    /// Makes `image`, the staged file recorded at `image.relative_path`, the
    /// subscription's cover, unless its cover no longer waits for that fetch of
    /// `anilist_media_id`. Whether it was taken; when not, the file is left to
    /// the cleanup.
    pub async fn subscription_fetched(
        &self,
        rule_id: &str,
        requested_at: Millis,
        anilist_media_id: i64,
        image: ImageRef,
    ) -> Result<bool, ArtworkError> {
        let id = rule_id.to_owned();
        self.db
            .run(move |c| Ok(fetched(c, &id, requested_at, anilist_media_id, &image)?))
            .await
    }

    /// The subscription job of `kind` asked for at `requested_at` did not
    /// finish: try again at `retry_at` (counting a failed attempt when
    /// `failed`), or, with `retry_at` `None`, give up and leave `note`.
    #[allow(clippy::too_many_arguments)]
    pub async fn subscription_job_later(
        &self,
        rule_id: &str,
        kind: JobKind,
        requested_at: Millis,
        retry_at: Option<Millis>,
        failed: bool,
        note: Note,
        now: Millis,
    ) -> Result<(), ArtworkError> {
        let id = rule_id.to_owned();
        self.db
            .run(move |c| {
                Ok(job_later(
                    c,
                    &id,
                    kind,
                    requested_at,
                    retry_at,
                    failed,
                    note,
                    now,
                )?)
            })
            .await
    }

    /// Rule `rule_id`'s subscription cover, if it has one.
    pub async fn subscription_image(
        &self,
        rule_id: &str,
    ) -> Result<Option<ImageRef>, ArtworkError> {
        let id = rule_id.to_owned();
        self.db.run(move |c| Ok(image(c, &id)?)).await
    }

    /// The image ID of each of `rule_ids` whose subscription has a cover, by
    /// rule ID.
    pub async fn subscription_image_ids_of(
        &self,
        rule_ids: Vec<String>,
    ) -> Result<HashMap<String, String>, ArtworkError> {
        self.db.run(move |c| Ok(image_ids_of(c, &rule_ids)?)).await
    }
}
