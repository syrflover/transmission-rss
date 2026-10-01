//! The work a folder of a watch folder is (ticket 0024: where a rule's videos
//! will land).

use rusqlite::{params, Connection, OptionalExtension};

use super::{LibraryError, LibraryStore};

/// The ID of the work whose folder is `dir_name` directly under the registered
/// watch folder at `folder_path`, if the library has it and its folder is
/// there.
fn work_in_folder(
    conn: &Connection,
    folder_path: &str,
    dir_name: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT w.id FROM works w
           JOIN watch_folders f ON f.id = w.watch_folder_id
          WHERE f.path = ?1 AND f.unregistered_at IS NULL
            AND w.dir_name = ?2 AND w.missing = 0",
        params![folder_path, dir_name],
        |row| row.get(0),
    )
    .optional()
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
}
