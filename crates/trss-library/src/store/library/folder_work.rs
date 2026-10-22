//! The work a folder of a watch folder is (ticket 2-work-discovery-and-subscriptions/0024: where a rule's videos
//! will land), and the works a save folder is chosen from (`docs/specs/
//! collection.md`, 방영작 구독).

use rusqlite::{params, Connection, OptionalExtension};

use crate::store::{
    library::{LibraryError, LibraryStore},
    seasons::anissia::WORK_TITLE,
};

/// A work directly under a watch folder, as a save folder is chosen from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderWork {
    pub id: String,
    /// The watch folder's path, as registered.
    pub folder: String,
    pub dir_name: String,
    /// The work title (작품명).
    pub title: String,
    /// The highest-numbered recorded season from 1 on; `None` when the work
    /// records none.
    pub last_season: Option<u32>,
}

/// The ID of the work whose folder is `dir_name` directly under the registered
/// watch folder at `folder_path`, if the library has it and its folder is
/// there.
fn work_in_folder(
    conn: &Connection,
    folder_path: &str,
    dir_name: &str,
) -> rusqlite::Result<Option<String>> {
    conn.prepare_cached(
        "SELECT w.id FROM works w
           JOIN watch_folders f ON f.id = w.watch_folder_id
          WHERE f.path = ?1 AND f.unregistered_at IS NULL
            AND w.dir_name = ?2 AND w.missing = 0",
    )?
    .query_row(params![folder_path, dir_name], |row| row.get(0))
    .optional()
}

fn works_in_folder(conn: &Connection, folder_path: &str) -> rusqlite::Result<Vec<FolderWork>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT w.id, f.path, w.dir_name, {WORK_TITLE},
                (SELECT max(s.number) FROM seasons s WHERE s.work_id = w.id AND s.number >= 1)
           FROM works w
           JOIN watch_folders f ON f.id = w.watch_folder_id
          WHERE f.path = ?1 AND f.unregistered_at IS NULL AND w.missing = 0
          ORDER BY w.dir_name"
    ))?;
    let rows = stmt.query_map([folder_path], |r| {
        Ok(FolderWork {
            id: r.get(0)?,
            folder: r.get(1)?,
            dir_name: r.get(2)?,
            title: r.get(3)?,
            last_season: r.get(4)?,
        })
    })?;
    rows.collect()
}

impl LibraryStore {
    /// The work whose folder is `dir_name` under the watch folder `folder_path`
    /// (as registered, without a trailing `/`), or `None` when the library has
    /// no such work.
    pub async fn work_in_folder(
        &self,
        folder_path: &str,
        dir_name: &str,
    ) -> Result<Option<String>, LibraryError> {
        let (folder_path, dir_name) = (folder_path.to_owned(), dir_name.to_owned());
        self.db
            .run(move |c| Ok(work_in_folder(c, &folder_path, &dir_name)?))
            .await
    }

    /// The works whose folders are directly under the registered watch folders
    /// at `folder_paths` (as registered, without a trailing `/`), folder by
    /// folder in the order given, each folder's by folder name. A path that is
    /// no registered watch folder has none.
    pub async fn works_in_folders(
        &self,
        folder_paths: Vec<String>,
    ) -> Result<Vec<FolderWork>, LibraryError> {
        self.db
            .run(move |c| {
                let mut out = Vec::new();
                for path in &folder_paths {
                    out.extend(works_in_folder(c, path)?);
                }
                Ok(out)
            })
            .await
    }
}
