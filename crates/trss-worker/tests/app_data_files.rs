//! The start-up check of the app data folder
//! ([`trss_core::access::check_app_data`]) names the files trss writes next to
//! the database by their suffix. This keeps that list in step with the
//! functions that make the paths, which live in crates `trss-core` cannot
//! depend on.

use std::path::{Path, PathBuf};

use trss_core::access::DATABASE_FILE_SUFFIXES;

fn assert_checked(db: &Path, path: PathBuf) {
    let name = path.file_name().unwrap().to_str().unwrap().to_owned();
    let suffix = name
        .strip_prefix(db.file_name().unwrap().to_str().unwrap())
        .unwrap_or_else(|| panic!("{name} does not start with the database's name"));
    assert!(
        DATABASE_FILE_SUFFIXES.contains(&suffix),
        "{name}: add {suffix:?} to DATABASE_FILE_SUFFIXES, or the start-up check skips it"
    );
    assert_eq!(
        path.parent(),
        db.parent(),
        "{name} is not next to the database"
    );
}

#[test]
fn app_data_files_cover_every_lock_and_the_wake_socket() {
    let db = Path::new("/data/trss.db");
    assert_checked(db, trss_core::lock_path_for(db));
    assert_checked(db, trss_core::wake::wake_path_for(db));
    assert_checked(db, trss_worker::browser::lock_path_for(db));
    assert_checked(db, trss_library::artwork::queue::lock_path_for(db));
    assert_checked(db, trss_library::seasons::queue::lock_path_for(db));
    assert_checked(db, trss_collect::anissia::lock_path_for(db));
    assert_checked(db, trss_collect::anissia::captions::lock_path_for(db));
}

#[test]
fn app_data_folders_cover_the_receive_area_the_artwork_and_the_subtitle_files() {
    use trss_core::access::APP_DATA_FOLDERS;
    let root = Path::new("/data");
    let receive = trss_jobs::ReceiveArea::in_app_data(root);
    assert!(APP_DATA_FOLDERS
        .iter()
        .any(|f| root.join(f) == receive.root()));
    assert!(APP_DATA_FOLDERS.contains(&trss_library::artwork::files::ARTWORK_DIR));
    assert!(APP_DATA_FOLDERS.contains(&trss_jobs::place::files::APP_FILES_DIR));
}
