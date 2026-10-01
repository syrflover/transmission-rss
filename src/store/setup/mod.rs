//! The first run's checklist state (`docs/specs/settings.md`, 처음 실행).
//!
//! The `first_run` row (see `schema.sql` and `ended.sql`) exists only for an
//! install that began empty. It keeps everything the checklist is made of: the
//! steps that happened (a watch folder registered, an import applied), the
//! steps the user skipped, and whether the checklist has ended. An ended
//! checklist stays ended whatever is removed later.

#[cfg(test)]
mod tests;

use rusqlite::{params, Connection, OptionalExtension};

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

    fn skipped_column(self) -> &'static str {
        match self {
            Step::Folder => "folder_skipped_at",
            Step::Import => "import_skipped_at",
        }
    }

    fn done_column(self) -> &'static str {
        match self {
            Step::Folder => "folder_added_at",
            Step::Import => "import_applied_at",
        }
    }
}

/// The state of an install that began empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FirstRun {
    /// A watch folder was registered (the folder step is done).
    pub folder_added: bool,
    /// An import was applied (the import step is done).
    pub import_applied: bool,
    pub folder_skipped: bool,
    pub import_skipped: bool,
    /// Both steps were done or skipped; the checklist does not come back.
    pub ended: bool,
}

impl FirstRun {
    pub fn done(&self, step: Step) -> bool {
        match step {
            Step::Folder => self.folder_added,
            Step::Import => self.import_applied,
        }
    }

    pub fn skipped(&self, step: Step) -> bool {
        match step {
            Step::Folder => self.folder_skipped,
            Step::Import => self.import_skipped,
        }
    }

    /// The step is done or skipped.
    pub fn settled(&self, step: Step) -> bool {
        self.done(step) || self.skipped(step)
    }

    /// The checklist is up: it has not ended and a step is still open.
    pub fn active(&self) -> bool {
        !self.ended && Step::ALL.into_iter().any(|step| !self.settled(step))
    }
}

fn read(conn: &Connection) -> rusqlite::Result<Option<FirstRun>> {
    conn.query_row(
        "SELECT folder_added_at IS NOT NULL, import_applied_at IS NOT NULL,
                folder_skipped_at IS NOT NULL, import_skipped_at IS NOT NULL,
                ended_at IS NOT NULL
           FROM first_run WHERE id = 1",
        [],
        |r| {
            Ok(FirstRun {
                folder_added: r.get(0)?,
                import_applied: r.get(1)?,
                folder_skipped: r.get(2)?,
                import_skipped: r.get(3)?,
                ended: r.get(4)?,
            })
        },
    )
    .optional()
}

/// Ends the checklist if both steps are settled and it has not ended yet.
/// Returns the state after it.
fn settle_in(conn: &Connection, now: Millis) -> rusqlite::Result<Option<FirstRun>> {
    let Some(run) = read(conn)? else {
        return Ok(None);
    };
    if run.ended || Step::ALL.into_iter().any(|step| !run.settled(step)) {
        return Ok(Some(run));
    }
    conn.execute(
        "UPDATE first_run SET ended_at = coalesce(ended_at, ?1) WHERE id = 1",
        [now],
    )?;
    read(conn)
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

    /// The state, or `None` for an install that did not begin empty.
    pub async fn first_run(&self) -> Result<Option<FirstRun>, SetupError> {
        self.db.run(|c| Ok::<_, SetupError>(read(c)?)).await
    }

    /// [`SetupStore::first_run`] that also ends the checklist when both steps
    /// are done or skipped, at `now`. This is what keeps it ended when the
    /// folders or channels that finished it are removed afterwards.
    pub async fn settle(&self, now: Millis) -> Result<Option<FirstRun>, SetupError> {
        self.db
            .run(move |c| Ok::<_, SetupError>(settle_in(c, now)?))
            .await
    }

    /// Skips `step` at `now`, or takes the skip back, and ends the checklist if
    /// that settles both steps. Skipping a step that is skipped already keeps
    /// the first time. Taking back the skip of a step that is not done also
    /// takes the end back, so the checklist is up again. `false` when the
    /// install did not begin empty (there is nothing to skip).
    pub async fn set_skipped(
        &self,
        step: Step,
        skipped: bool,
        now: Millis,
    ) -> Result<bool, SetupError> {
        self.db
            .run(move |c| {
                let tx = c.transaction()?;
                let (skip, done) = (step.skipped_column(), step.done_column());
                let changed = tx.execute(
                    &format!(
                        "UPDATE first_run
                            SET {skip} = CASE WHEN ?1 THEN coalesce({skip}, ?2) END,
                                ended_at = CASE WHEN ?1 OR {done} IS NOT NULL
                                                THEN ended_at END
                          WHERE id = 1"
                    ),
                    params![skipped, now],
                )?;
                settle_in(&tx, now)?;
                tx.commit()?;
                Ok::<_, SetupError>(changed > 0)
            })
            .await
    }

    /// Records that an import was applied at `now` (the import step is done).
    /// Keeps the first time. Nothing happens for an install that did not begin
    /// empty.
    pub async fn mark_import_applied(&self, now: Millis) -> Result<(), SetupError> {
        self.db
            .run(move |c| {
                c.execute(
                    "UPDATE first_run SET import_applied_at = coalesce(import_applied_at, ?1)
                      WHERE id = 1",
                    [now],
                )?;
                Ok::<_, SetupError>(())
            })
            .await
    }
}
