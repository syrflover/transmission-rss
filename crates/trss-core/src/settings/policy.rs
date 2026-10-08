//! The common policy (`docs/specs/settings.md`, 공통 정책): the order of the
//! subtitle formats, and the server browser's idle time and how many jobs may
//! use it at once. The three are one record with one version, saved together.
//!
//! Until the first save the policy is [`Policy::default`] at version 0. The
//! supported ranges ([`IDLE_TIMEOUT_SECONDS`], [`CONCURRENT_JOBS`]) are the
//! app's: a value outside them is refused and nothing is saved.
//!
//! A work may order the formats its own way ([`WorkFormatOrder`]); the work's
//! subtitles write that ([`SettingsStore::put_work_format_order`]), the
//! settings list the works that have one.

use std::{fmt, ops::RangeInclusive};

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

use super::{SettingsError, SettingsStore};
use crate::Millis;

/// The browser's idle time, in seconds: one minute to one hour.
pub const IDLE_TIMEOUT_SECONDS: RangeInclusive<u32> = 60..=3600;
/// How many jobs may use the server browser at once. The server is a small
/// machine and each job holds a browser of its own.
pub const CONCURRENT_JOBS: RangeInclusive<u32> = 1..=3;

/// A subtitle file format the app ranks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubtitleFormat {
    Ass,
    Srt,
    Smi,
}

impl SubtitleFormat {
    pub const ALL: [SubtitleFormat; 3] = [
        SubtitleFormat::Ass,
        SubtitleFormat::Srt,
        SubtitleFormat::Smi,
    ];

    /// The lower-case code of the API and the records (`ass`).
    pub fn code(self) -> &'static str {
        match self {
            SubtitleFormat::Ass => "ass",
            SubtitleFormat::Srt => "srt",
            SubtitleFormat::Smi => "smi",
        }
    }

    pub fn parse(code: &str) -> Option<SubtitleFormat> {
        SubtitleFormat::ALL.into_iter().find(|f| f.code() == code)
    }
}

/// The three formats, each once, the preferred first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatOrder([SubtitleFormat; 3]);

impl Default for FormatOrder {
    /// ASS → SRT → SMI.
    fn default() -> FormatOrder {
        FormatOrder(SubtitleFormat::ALL)
    }
}

impl FormatOrder {
    /// The order the codes give, when they name each format once.
    pub fn from_codes<S: AsRef<str>>(codes: &[S]) -> Option<FormatOrder> {
        let formats: Vec<SubtitleFormat> = codes
            .iter()
            .map(|c| SubtitleFormat::parse(c.as_ref()))
            .collect::<Option<_>>()?;
        let order: [SubtitleFormat; 3] = formats.try_into().ok()?;
        let each_once = SubtitleFormat::ALL.iter().all(|f| order.contains(f));
        each_once.then_some(FormatOrder(order))
    }

    pub fn formats(&self) -> [SubtitleFormat; 3] {
        self.0
    }

    fn parse_stored(text: &str) -> Option<FormatOrder> {
        let codes: Vec<&str> = text.split(',').collect();
        FormatOrder::from_codes(&codes)
    }
}

impl fmt::Display for FormatOrder {
    /// As stored: `ass,srt,smi`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let codes: Vec<&str> = self.0.iter().map(|x| x.code()).collect();
        f.write_str(&codes.join(","))
    }
}

/// The common policy as stored, or the defaults at version 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub format_order: FormatOrder,
    pub idle_timeout_seconds: u32,
    pub max_concurrent_jobs: u32,
    /// 0 until the first save; send it back to save.
    pub version: i64,
    /// When it was last saved; `None` before the first save.
    pub saved_at: Option<Millis>,
}

impl Default for Policy {
    fn default() -> Policy {
        Policy {
            format_order: FormatOrder::default(),
            idle_timeout_seconds: 300,
            max_concurrent_jobs: 1,
            version: 0,
            saved_at: None,
        }
    }
}

/// A work's own format order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkFormatOrder {
    pub work_id: String,
    /// The work's folder name, as the library names it.
    pub name: String,
    pub format_order: FormatOrder,
    pub updated_at: Millis,
}

/// A stored count as it reads: the table only keeps it positive, so a row
/// written around [`SettingsStore::put_policy`] may hold more than a `u32`. It
/// reads as the largest one rather than failing every reading and every save,
/// and the next save checks it against the supported range.
fn stored_count(value: i64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn read_policy(conn: &Connection) -> Result<Policy, SettingsError> {
    let row: Option<(String, i64, i64, i64, Millis)> = conn
        .prepare_cached(
            "SELECT format_order, idle_timeout_seconds, max_concurrent_jobs, version, saved_at
               FROM policy_settings WHERE id = 1",
        )?
        .query_row([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .optional()?;
    let Some((order, idle, jobs, version, saved_at)) = row else {
        return Ok(Policy::default());
    };
    Ok(Policy {
        format_order: FormatOrder::parse_stored(&order)
            .ok_or(SettingsError::Invalid("a stored format order is not one"))?,
        idle_timeout_seconds: stored_count(idle),
        max_concurrent_jobs: stored_count(jobs),
        version,
        saved_at: Some(saved_at),
    })
}

impl SettingsStore {
    /// The common policy: as last saved, or the defaults at version 0.
    pub async fn policy(&self) -> Result<Policy, SettingsError> {
        self.db.run(|c| read_policy(c)).await
    }

    /// Saves the policy if it is still at `expected_version` (0 before the
    /// first save) and returns it with its new version. Values outside the
    /// supported ranges are [`SettingsError::Invalid`]; a stale version is
    /// [`SettingsError::Conflict`]. Either way nothing is saved.
    pub async fn put_policy(
        &self,
        expected_version: i64,
        format_order: FormatOrder,
        idle_timeout_seconds: u32,
        max_concurrent_jobs: u32,
        now: Millis,
    ) -> Result<Policy, SettingsError> {
        if !IDLE_TIMEOUT_SECONDS.contains(&idle_timeout_seconds) {
            return Err(SettingsError::Invalid("the idle time is out of range"));
        }
        if !CONCURRENT_JOBS.contains(&max_concurrent_jobs) {
            return Err(SettingsError::Invalid(
                "the concurrent jobs are out of range",
            ));
        }
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let actual = read_policy(&tx)?.version;
                if actual != expected_version {
                    return Err(SettingsError::Conflict {
                        expected: expected_version,
                        actual,
                    });
                }
                tx.prepare_cached("INSERT INTO policy_settings
                         (id, format_order, idle_timeout_seconds, max_concurrent_jobs, version, saved_at)
                     VALUES (1, ?1, ?2, ?3, 1, ?4)
                     ON CONFLICT (id) DO UPDATE SET
                         format_order = excluded.format_order,
                         idle_timeout_seconds = excluded.idle_timeout_seconds,
                         max_concurrent_jobs = excluded.max_concurrent_jobs,
                         version = version + 1,
                         saved_at = excluded.saved_at")?.execute(
                    params![
                        format_order.to_string(),
                        idle_timeout_seconds,
                        max_concurrent_jobs,
                        now
                    ],
                )?;
                let stored = read_policy(&tx)?;
                tx.commit()?;
                Ok(stored)
            })
            .await
    }

    /// The work's own format order, `None` when it has none (or is no work).
    pub async fn work_format_order(
        &self,
        work_id: &str,
    ) -> Result<Option<FormatOrder>, SettingsError> {
        let work = work_id.to_owned();
        self.db
            .run(move |c| {
                let stored: Option<String> = c
                    .prepare_cached(
                        "SELECT format_order FROM work_subtitle_policy WHERE work_id = ?1",
                    )?
                    .query_row([work], |r| r.get(0))
                    .optional()?;
                stored
                    .map(|text| {
                        FormatOrder::parse_stored(&text)
                            .ok_or(SettingsError::Invalid("a stored format order is not one"))
                    })
                    .transpose()
            })
            .await
    }

    /// Gives the work its own format order, replacing the one it had, and
    /// puts it first of the works that have one. `false` when there is no such
    /// work, which nothing was written for.
    pub async fn put_work_format_order(
        &self,
        work_id: &str,
        format_order: FormatOrder,
        now: Millis,
    ) -> Result<bool, SettingsError> {
        let work = work_id.to_owned();
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let known: i64 = tx
                    .prepare_cached("SELECT count(*) FROM works WHERE id = ?1")?
                    .query_row([&work], |r| r.get(0))?;
                if known == 0 {
                    return Ok(false);
                }
                tx.prepare_cached(
                    "INSERT INTO work_subtitle_policy (work_id, format_order, updated_at)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT (work_id) DO UPDATE SET
                         format_order = excluded.format_order,
                         updated_at = excluded.updated_at",
                )?
                .execute(params![work, format_order.to_string(), now])?;
                tx.commit()?;
                Ok(true)
            })
            .await
    }

    /// Takes the work's own format order away, so it follows the common
    /// policy again. `false` when there is no such work; a work with no order
    /// of its own is left as it is.
    pub async fn delete_work_format_order(&self, work_id: &str) -> Result<bool, SettingsError> {
        let work = work_id.to_owned();
        self.db
            .run(move |c| {
                let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let known: i64 = tx
                    .prepare_cached("SELECT count(*) FROM works WHERE id = ?1")?
                    .query_row([&work], |r| r.get(0))?;
                if known == 0 {
                    return Ok(false);
                }
                tx.prepare_cached("DELETE FROM work_subtitle_policy WHERE work_id = ?1")?
                    .execute([&work])?;
                tx.commit()?;
                Ok(true)
            })
            .await
    }

    /// The works that order the formats their own way, most recently changed
    /// first.
    pub async fn work_format_orders(&self) -> Result<Vec<WorkFormatOrder>, SettingsError> {
        self.db
            .run(|c| {
                let mut stmt = c.prepare_cached(
                    "SELECT p.work_id, w.dir_name, p.format_order, p.updated_at
                       FROM work_subtitle_policy p
                       JOIN works w ON w.id = p.work_id
                       JOIN watch_folders f ON f.id = w.watch_folder_id
                      WHERE f.unregistered_at IS NULL
                      ORDER BY p.updated_at DESC, p.work_id",
                )?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get(3)?,
                        ))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                rows.into_iter()
                    .map(|(work_id, name, order, updated_at)| {
                        Ok(WorkFormatOrder {
                            work_id,
                            name,
                            format_order: FormatOrder::parse_stored(&order).ok_or(
                                SettingsError::Invalid("a stored format order is not one"),
                            )?,
                            updated_at,
                        })
                    })
                    .collect()
            })
            .await
    }
}

#[cfg(test)]
mod tests;
