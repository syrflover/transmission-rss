//! The first run's checklist state (`docs/specs/settings.md`, 처음 실행).
//!
//! Only what the data cannot say is stored: whether the install began empty (a
//! `first_run` row exists, see `schema.sql`) and which steps the user skipped.
//! Whether a step is done is read from the data by the caller.

#[cfg(test)]
mod tests;

use rusqlite::{params, OptionalExtension};

use super::{
    db::{Db, DbError},
    history::Millis,
};

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error(transparent)]
    Db(#[from] DbError),
}

impl From<rusqlite::Error> for SetupError {
    fn from(e: rusqlite::Error) -> Self {
        SetupError::Db(DbError::Sqlite(e))
    }
}

/// A step of the `처음 설정` checklist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// `감시 폴더 등록`.
    Folder,
    /// `기존 설정 가져오기`.
    Import,
}

impl Step {
    /// In checklist order.
    pub const ALL: [Step; 2] = [Step::Folder, Step::Import];

    pub fn code(self) -> &'static str {
        match self {
            Step::Folder => "folder",
            Step::Import => "import",
        }
    }

    pub fn parse(code: &str) -> Option<Step> {
        Step::ALL.into_iter().find(|s| s.code() == code)
    }

    fn column(self) -> &'static str {
        match self {
            Step::Folder => "folder_skipped_at",
            Step::Import => "import_skipped_at",
        }
    }
}

/// The skipped steps of an install that began empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FirstRun {
    pub folder_skipped: bool,
    pub import_skipped: bool,
}

impl FirstRun {
    pub fn skipped(&self, step: Step) -> bool {
        match step {
            Step::Folder => self.folder_skipped,
            Step::Import => self.import_skipped,
        }
    }
}

/// Async access to the first run's state. Cheap to clone.
#[derive(Clone)]
pub struct SetupStore {
    db: Db,
}

impl SetupStore {
    pub fn new(db: Db) -> Self {
        SetupStore { db }
    }

    /// The skipped steps, or `None` for an install that did not begin empty.
    pub async fn first_run(&self) -> Result<Option<FirstRun>, SetupError> {
        self.db
            .run(|c| {
                Ok::<_, SetupError>(
                    c.query_row(
                        "SELECT folder_skipped_at IS NOT NULL, import_skipped_at IS NOT NULL
                           FROM first_run WHERE id = 1",
                        [],
                        |r| {
                            Ok(FirstRun {
                                folder_skipped: r.get(0)?,
                                import_skipped: r.get(1)?,
                            })
                        },
                    )
                    .optional()?,
                )
            })
            .await
    }

    /// Skips `step` at `now`, or takes the skip back. Skipping a step that is
    /// skipped already keeps the first time. `false` when the install did not
    /// begin empty (there is nothing to skip).
    pub async fn set_skipped(
        &self,
        step: Step,
        skipped: bool,
        now: Millis,
    ) -> Result<bool, SetupError> {
        self.db
            .run(move |c| {
                let column = step.column();
                let changed = c.execute(
                    &format!(
                        "UPDATE first_run
                            SET {column} = CASE WHEN ?1 THEN coalesce({column}, ?2) END
                          WHERE id = 1"
                    ),
                    params![skipped, now],
                )?;
                Ok::<_, SetupError>(changed > 0)
            })
            .await
    }
}
