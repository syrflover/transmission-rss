//! The names of the folders trss makes in the app data folder (the
//! database's folder).
//!
//! Each folder belongs to the crate that fills it, but the start-up check of
//! the app data folder ([`crate::access::check_app_data`]) must name them
//! all, and the web names the receive area it shows. Their names are written
//! here once, so the check cannot miss a folder an owner renames.

// A folder's name is spelled once, for the folder and for the temporary
// folder inside it.
macro_rules! artwork {
    () => {
        "artwork"
    };
}
macro_rules! subtitle_files {
    () => {
        "subtitle-files"
    };
}

/// Where the subtitle jobs put what they receive (trss-jobs' receive area).
pub const RECEIVE_DIR: &str = "receive";
/// Where the work covers and their staging are (trss-library's artwork).
pub const ARTWORK_DIR: &str = artwork!();
/// Where new covers are written before they are published, relative to the
/// app data folder.
pub const ARTWORK_STAGING_DIR: &str = concat!(artwork!(), "/.staging");
/// Where a subtitle package's attachments and companion files go
/// (trss-jobs), as `<work id>/<creator>/<name>`.
pub const SUBTITLE_FILES_DIR: &str = subtitle_files!();
/// Where the temporary files of effects in [`SUBTITLE_FILES_DIR`] go,
/// relative to the app data folder.
pub const SUBTITLE_FILES_TEMP_DIR: &str = concat!(subtitle_files!(), "/.tmp");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_names_are_the_folders_of_the_deployed_app_data() {
        // These are the names of folders that exist on a running server, so a
        // change of one is a migration, not a rename.
        assert_eq!(RECEIVE_DIR, "receive");
        assert_eq!(ARTWORK_DIR, "artwork");
        assert_eq!(ARTWORK_STAGING_DIR, "artwork/.staging");
        assert_eq!(SUBTITLE_FILES_DIR, "subtitle-files");
        assert_eq!(SUBTITLE_FILES_TEMP_DIR, "subtitle-files/.tmp");
    }

    #[test]
    fn the_write_check_names_every_folder() {
        for folder in [RECEIVE_DIR, ARTWORK_DIR, SUBTITLE_FILES_DIR] {
            assert!(
                crate::access::APP_DATA_FOLDERS.contains(&folder),
                "{folder} is not checked at start-up"
            );
        }
    }
}
