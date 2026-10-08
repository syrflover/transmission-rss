//! Google Drive fonts that are not received again when they did not change
//! (`docs/specs/subtitles.md`, 폰트; `migrations/jobs/unchanged_fonts.sql` in
//! `trss-core`).
//!
//! # The record
//!
//! Only an individual Google Drive file (key `drive:<id>`, not one a server
//! browser took or downloaded) is ever left unreceived. Before a receipt asks
//! for its bytes, the runner looks for the font the work keeps of it
//! ([`Placer::unchanged_font`]): the latest receipt of the key, in any job,
//! that is the file itself (not an archive's member) and whose plan row kept
//! it as a font not removed, in the creator folder this job stores into. That
//! receipt's size and the `Last-Modified` of its snapshot are the record; no
//! other column holds them. A later receipt of the key with the same bytes
//! reuses the font, so it becomes the record by itself, and so does a receipt
//! that was not received (its size and snapshot are what the `HEAD` said). A
//! key never kept as a font, a record with no `Last-Modified`, a work folder
//! that is not there, and a font whose file is missing or holds other bytes
//! than recorded give none: the file is received as before, with no `HEAD`.
//!
//! With a record, one `HEAD` of the file goes through the Drive client's pace
//! (`Source::recheck`). When its size and `Last-Modified` are the record's,
//! the receipt is `done` with no path and names the font
//! ([`crate::store::FileRow::unchanged_asset`]); a `HEAD` that fails, gives
//! no `Last-Modified` or other values, receives the file.
//!
//! # Using the font
//!
//! The analysis plans the receipt as any other, from its name, size and
//! SHA-256 (the font's). Its row is stored by using the font as a reuse does
//! ([`Placer::store_unchanged`]): a package entry, the row's `asset_id`, and
//! the links of the package's subtitles. A cleanup keeps the font for a job
//! not ended whose receipt names it and whose row is not stored yet, and
//! after that as it keeps any reuse ([`super::cleanup`]).
//!
//! # When the font went away
//!
//! The font is looked at again when its row is stored, which may be a later
//! run (a restart, a work folder back after a wait). One a cleanup removed,
//! one whose file is gone or changed, or one now outside the creator folder
//! is never published from nothing: in one transaction ([`revoke`]) the
//! receipt and those that share it are `abandoned`, its rows not kept yet
//! go, and those of their items that were received go back to `pending`. The job goes back in line, and
//! its next run receives the file (the font fails the look above, so with no
//! `HEAD`). Until then, no subtitle of those posts is linked
//! ([`Placer::link`]).
//!
//! # What the job's detail says
//!
//! Each kept font says how it was received ([`font_receipt`]): not received
//! (`unchanged`, a receipt that asked for no bytes), received with the bytes
//! of a font kept before, which it uses (`same`: a Naver or Tistory font, a
//! Drive font whose `Last-Modified` changed, a font in an archive), or kept
//! as a new file (`new`, its row's own store published it). Keeping one copy
//! of the same bytes is never told as a download not made. An archive comes
//! whole; [`new_assets`] counts the new files its members became.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use trss_core::Millis;

use super::{
    blocking, files,
    records::{self, durable, Asset, JobFacts, Kept, PlanRow},
    row_label, Placer,
};
use crate::{
    model::{AssetKind, PlanAction},
    store::{FileRow, ItemRow, JobError},
};

/// What a font row whose receipt was not received logs when its font went
/// away or changed and the file is received again.
pub const RECEIVE_AGAIN: &str = "받지 않은 폰트의 보관본이 없어졌거나 바뀌어 다시 받아요";

/// The stored font a job may use instead of receiving a Drive file, with the
/// record it was kept from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptFont {
    pub asset_id: String,
    /// The name the record's receipt had (the one Drive gave).
    pub name: String,
    /// The record's size and `Last-Modified`, which a `HEAD` must give.
    pub size: u64,
    pub last_modified: String,
    /// The font's SHA-256.
    pub sha256: String,
}

/// How a kept font was received (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontReceipt {
    /// Not received: its Google Drive file did not change (`받지 않음(바뀌지
    /// 않음)`).
    Unchanged,
    /// Received, with the bytes of a font kept before, which it uses (`받아서
    /// 같음`).
    Same,
    /// Received and kept as a new file (`새로 받음`).
    New,
}

impl FontReceipt {
    /// Its code in the API.
    pub fn code(self) -> &'static str {
        match self {
            FontReceipt::Unchanged => "unchanged",
            FontReceipt::Same => "same",
            FontReceipt::New => "new",
        }
    }
}

/// How the font of `row` was received, when the row kept a font:
/// `unchanged` when its receipt (`receipt`, the file itself, not an
/// archive's) asked for no bytes, `new` when the row's own store published
/// its file (`made`, [`records::RowPaths::made`]), `same` otherwise.
pub fn font_receipt(row: &PlanRow, receipt: Option<&FileRow>, made: bool) -> Option<FontReceipt> {
    if row.kind != AssetKind::Font || row.asset_id.is_none() {
        return None;
    }
    let unchanged = row.member.is_none() && receipt.is_some_and(|f| f.unchanged_asset.is_some());
    Some(match (unchanged, made) {
        (true, _) => FontReceipt::Unchanged,
        (false, true) => FontReceipt::New,
        (false, false) => FontReceipt::Same,
    })
}

/// By archive receipt: how many of its members became new stored files
/// (`made`, by plan position), for an archive none of whose rows to keep is
/// still under way.
pub fn new_assets<'a>(plan: &'a [PlanRow], made: &HashMap<i64, bool>) -> HashMap<&'a str, usize> {
    let mut counts: HashMap<&str, Option<usize>> = HashMap::new();
    for row in plan.iter().filter(|r| r.member.is_some()) {
        let count = counts.entry(row.file_id.as_str()).or_insert(Some(0));
        let under_way = row.action != PlanAction::Drop && !row.kept() && row.outcome.is_none();
        let new = row.kept() && made.get(&row.position).copied().unwrap_or(false);
        *count = match (*count, under_way) {
            (Some(n), false) => Some(n + usize::from(new)),
            _ => None,
        };
    }
    counts
        .into_iter()
        .filter_map(|(id, n)| Some((id, n?)))
        .collect()
}

/// The last `last_modified` of a receipt's snapshot (a JSON array of `[name,
/// value]` pairs): the answer's, which comes after the post's own values.
pub fn last_modified(snapshot: &str) -> Option<String> {
    let pairs: Vec<[String; 2]> = serde_json::from_str(snapshot).ok()?;
    pairs
        .into_iter()
        .rev()
        .find(|[name, _]| name == trss_subtitles::http::LAST_MODIFIED)
        .map(|[_, value]| value)
        .filter(|v| !v.is_empty())
}

/// The latest receipt of a Drive file kept as a font ([`record`]).
struct Record {
    name: String,
    size: Option<i64>,
    snapshot: Option<String>,
    /// The font it is kept as.
    asset_id: String,
}

/// The latest receipt of `key` kept as a font of `work_id` under `dir/` (not
/// in a folder within it) that is not removed.
fn record(c: &Connection, work_id: &str, key: &str, dir: &str) -> rusqlite::Result<Option<Record>> {
    c.prepare_cached(
        "SELECT f.name, f.size, f.snapshot, a.id
           FROM subtitle_job_files f
           JOIN subtitle_job_plan p
             ON p.file_id = f.id AND p.member IS NULL AND p.kind = 'font'
           JOIN subtitle_assets a ON a.id = p.asset_id
          WHERE f.file_key = ?1 AND f.state = 'done' AND f.same_as IS NULL
            AND a.work_id = ?2 AND a.base = 'work' AND a.kind = 'font'
            AND a.removed_at IS NULL
            AND lower(substr(a.relative_path, 1, length(?3))) = lower(?3)
            AND instr(substr(a.relative_path, length(?3) + 1), '/') = 0
          ORDER BY f.created_at DESC, f.updated_at DESC, f.id
          LIMIT 1",
    )?
    .query_row(params![key, work_id, format!("{dir}/")], |r| {
        Ok(Record {
            name: r.get(0)?,
            size: r.get(1)?,
            snapshot: r.get(2)?,
            asset_id: r.get(3)?,
        })
    })
    .optional()
}

/// The receipt `receipt` that was not received, and the receipts sharing it,
/// are `abandoned` with `reason`; its plan rows not kept yet (with no effect)
/// go; and the items of those receipts that were received go back to
/// `pending`, to receive the file anew. An item held, failed or waiting keeps
/// its state and reason: it was never planned, and receives the file when it
/// is taken up again. One synced transaction.
pub fn revoke(
    c: &mut Connection,
    receipt: &str,
    reason: &str,
    now: Millis,
) -> Result<(), JobError> {
    durable(c, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let items: Vec<i64> = {
            let mut stmt = tx.prepare_cached(
                "SELECT DISTINCT item_id FROM subtitle_job_files
                  WHERE (id = ?1 OR same_as = ?1) AND state = 'done'
                    AND unchanged_asset IS NOT NULL",
            )?;
            let rows = stmt.query_map([receipt], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        tx.prepare_cached(
            "UPDATE subtitle_job_files SET state = 'abandoned', reason = ?2, updated_at = ?3
              WHERE (id = ?1 OR same_as = ?1) AND state = 'done'
                AND unchanged_asset IS NOT NULL",
        )?
        .execute(params![receipt, reason, now])?;
        tx.prepare_cached(
            "DELETE FROM subtitle_job_plan
              WHERE file_id = ?1 AND stored_id IS NULL AND asset_id IS NULL AND outcome IS NULL
                AND NOT EXISTS (SELECT 1 FROM subtitle_file_effects e
                                 WHERE e.job_id = subtitle_job_plan.job_id
                                   AND e.position = subtitle_job_plan.position)",
        )?
        .execute([receipt])?;
        for item in items {
            tx.prepare_cached(
                "UPDATE subtitle_job_items
                    SET state = 'pending', wait = NULL, reason = NULL, failure = NULL,
                        unchanged_from = NULL, updated_at = ?2
                  WHERE id = ?1 AND state = 'done'",
            )?
            .execute(params![item, now])?;
        }
        tx.commit()?;
        Ok(())
    })
}

impl Placer {
    /// The work folder, the creator folder (relative to it) and the font
    /// `asset_id` when the job may use it: the work folder is there, the font
    /// is not removed, is in the creator folder the job stores into, and its
    /// file has its recorded length and SHA-256.
    async fn usable_font(
        &self,
        facts: &JobFacts,
        asset_id: &str,
    ) -> Result<Option<(PathBuf, String, Asset)>, JobError> {
        let Some(work) = facts.work_id.clone() else {
            return Ok(None);
        };
        let folder = self.read(move |c| records::work_folder(c, &work)).await?;
        let Some(folder) = folder.map(PathBuf::from).filter(|f| f.is_dir()) else {
            return Ok(None);
        };
        let dir = self
            .creator_dir(facts, "work", files::SUBTITLES_DIR.to_owned())
            .await?;
        let id = asset_id.to_owned();
        let Some(asset) = self.read(move |c| records::asset(c, &id)).await? else {
            return Ok(None);
        };
        let inside = asset
            .relative_path
            .to_lowercase()
            .strip_prefix(&format!("{}/", dir.to_lowercase()))
            .is_some_and(|rest| !rest.is_empty() && !rest.contains('/'));
        if asset.kind != AssetKind::Font || !inside {
            return Ok(None);
        }
        let path = files::within(&folder, &asset.relative_path);
        let (size, sha) = (asset.size, asset.sha256.clone());
        let same = blocking(move || files::facts(&path))
            .await
            .ok()
            .flatten()
            .is_some_and(|(n, s, _)| n == size && s == sha);
        Ok(same.then_some((folder, dir, asset)))
    }

    /// The font the job may use instead of receiving the Drive file `key`
    /// (see the module docs), or `None` when it receives it as before.
    pub(crate) async fn unchanged_font(
        &self,
        job: &str,
        key: &str,
    ) -> Result<Option<KeptFont>, JobError> {
        let id = job.to_owned();
        let Some(facts) = self.read(move |c| records::job_facts(c, &id)).await? else {
            return Ok(None);
        };
        let Some(work) = facts.work_id.clone() else {
            return Ok(None);
        };
        let dir = self
            .creator_dir(&facts, "work", files::SUBTITLES_DIR.to_owned())
            .await?;
        let key = key.to_owned();
        let found = self.read(move |c| record(c, &work, &key, &dir)).await?;
        let Some(Record {
            name,
            size: Some(size),
            snapshot: Some(snapshot),
            asset_id,
        }) = found
        else {
            return Ok(None);
        };
        let (Ok(size), Some(last_modified)) = (u64::try_from(size), last_modified(&snapshot))
        else {
            return Ok(None);
        };
        let Some((_, _, asset)) = self.usable_font(&facts, &asset_id).await? else {
            return Ok(None);
        };
        if asset.size != size {
            return Ok(None);
        }
        Ok(Some(KeptFont {
            asset_id,
            name,
            size,
            last_modified,
            sha256: asset.sha256,
        }))
    }

    /// Stores the row of a receipt that was not received by using its font,
    /// or, when the font went away or changed, takes the receipt back so the
    /// file is received anew (see the module docs).
    pub(super) async fn store_unchanged(
        &self,
        facts: &JobFacts,
        items: &[ItemRow],
        row: &PlanRow,
        receipt: &FileRow,
        asset_id: &str,
    ) -> Result<(), JobError> {
        let usable = match row.kind {
            AssetKind::Font => self.usable_font(facts, asset_id).await?,
            _ => None,
        };
        let usable = usable.filter(|(_, _, a)| a.size == row.size && a.sha256 == row.sha256);
        let Some((folder, _, asset)) = usable else {
            let (id, now) = (receipt.id.clone(), self.now());
            self.write(move |c| revoke(c, &id, RECEIVE_AGAIN, now))
                .await?;
            return self
                .event(&row.job_id, format!("{}: {RECEIVE_AGAIN}", row.name), None)
                .await;
        };
        let at: PathBuf = files::within(Path::new(&folder), &asset.relative_path);
        let relative = asset.relative_path.clone();
        self.record_stored(facts, items, row, Kept::Reused(asset.id), &at)
            .await?;
        self.event(
            &row.job_id,
            format!(
                "{}: 바뀌지 않은 폰트라 보관한 것을 그대로 써요",
                row_label(row)
            ),
            Some(relative),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answer_s_last_modified_is_the_last_one_of_the_snapshot() {
        assert_eq!(
            last_modified(r#"[["last_modified","post"],["date","x"],["last_modified","file"]]"#),
            Some("file".to_owned())
        );
        assert_eq!(last_modified(r#"[["content_length","5"]]"#), None);
        assert_eq!(last_modified(r#"[["last_modified",""]]"#), None);
        assert_eq!(last_modified("not json"), None);
    }
}
