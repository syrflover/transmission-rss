//! The watch folders that follow the collection settings
//! (`docs/specs/library.md`, 작품 발견과 감시 폴더).
//!
//! The collect folder and the archive folder are always watch folders. [`plan`]
//! compares the folders the settings call for with the registered watch folders
//! and says what to change; [`crate::store::library::AutomaticPlan`] is applied
//! by the library store, in the same transaction as a settings save, or by the
//! worker for a database that was saved before the folders were watched.
//!
//! Folders are compared after resolving links (`canonicalize`), so a link cannot
//! hide that two folders are one place or overlap. A registered folder that
//! cannot be resolved (a mount that is not there right now) is compared as
//! written.
//!
//! What a plan does, for each folder the settings call for:
//!
//! - one registered at the same place (automatic or by hand) is kept; one by
//!   hand is made automatic and given the path as the settings write it (one
//!   that is automatic already keeps the spelling it has); its records stay;
//! - otherwise a new automatic one is registered, unless it would be inside a
//!   folder the user registered by hand or contain one: that is refused, since
//!   the same work would be found twice;
//!
//! and an automatic folder no setting calls for is removed.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use crate::{
    discovery,
    store::library::{AutomaticPlan, NewAutomatic, WatchFolder},
};

/// A folder the settings call for, and what the settings call it in a message.
#[derive(Debug, Clone)]
pub struct Wanted {
    /// `수집 폴더` or `보관 폴더`.
    pub what: &'static str,
    pub path: String,
}

/// `path` with links resolved, or as written when it cannot be resolved.
pub fn resolved(path: &str) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

/// The plan that makes the automatic watch folders `wanted`, over `registered`.
/// The error is a sentence for the user: a wanted folder overlaps a folder the
/// user registered by hand.
pub fn plan(wanted: &[Wanted], registered: &[WatchFolder]) -> Result<AutomaticPlan, String> {
    let real: Vec<PathBuf> = registered.iter().map(|f| resolved(&f.path)).collect();
    let mut plan = AutomaticPlan::over(registered);
    let mut kept: HashSet<usize> = HashSet::new();

    for want in wanted {
        let real_want = resolved(&want.path);
        let same = registered
            .iter()
            .enumerate()
            .find(|(i, f)| f.path == want.path || real[*i] == real_want);
        if let Some((i, folder)) = same {
            kept.insert(i);
            // One that is automatic already is settled, however it is spelled:
            // the store cannot always take the settings' spelling (an
            // unregistered folder may hold it), and a keep that changes
            // nothing would come back every cycle.
            if !folder.automatic {
                plan.keep.push((folder.id.clone(), want.path.clone()));
            }
            continue;
        }

        for (i, other) in registered.iter().enumerate() {
            if other.automatic {
                continue;
            }
            if real_want.starts_with(&real[i]) {
                return Err(format!(
                    "{} `{}`가 이미 등록한 감시 폴더 `{}` 안에 있어요. 같은 작품을 두 번 찾게 되므로, 그 감시 폴더의 등록을 해제하거나 다른 폴더를 정해 주세요.",
                    want.what, want.path, other.path
                ));
            }
            if real[i].starts_with(&real_want) {
                return Err(format!(
                    "{} `{}` 안에 이미 등록한 감시 폴더 `{}`가 있어요. 같은 작품을 두 번 찾게 되므로, 그 감시 폴더의 등록을 해제하거나 다른 폴더를 정해 주세요.",
                    want.what, want.path, other.path
                ));
            }
        }
        plan.add.push(NewAutomatic {
            path: want.path.clone(),
            scan: None,
        });
    }

    for (i, folder) in registered.iter().enumerate() {
        if folder.automatic && !kept.contains(&i) {
            plan.remove.push(folder.id.clone());
        }
    }
    Ok(plan)
}

/// Gives each folder `plan` registers its first reading. A folder that cannot
/// be read is left without one: the settings are saved all the same, and the
/// worker reads it and shows why on its row.
pub fn read_new_folders(plan: &mut AutomaticPlan) {
    for new in &mut plan.add {
        match discovery::scan(Path::new(&new.path)) {
            Ok(scan) => new.scan = Some(scan),
            Err(e) => eprintln!("trss: cannot read the new watch folder {}: {e}", new.path),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn folder(id: &str, path: &Path, automatic: bool) -> WatchFolder {
        WatchFolder {
            id: id.to_owned(),
            path: path.to_str().unwrap().to_owned(),
            created_at: 0,
            automatic,
            baselined: true,
            checked_at: None,
            error: None,
            watch_note: None,
        }
    }

    fn want(what: &'static str, path: &Path) -> Wanted {
        Wanted {
            what,
            path: path.to_str().unwrap().to_owned(),
        }
    }

    struct Tree {
        dir: tempfile::TempDir,
    }

    impl Tree {
        fn new(names: &[&str]) -> Tree {
            let dir = tempfile::tempdir().unwrap();
            for name in names {
                fs::create_dir_all(dir.path().join(name)).unwrap();
            }
            Tree { dir }
        }

        fn at(&self, name: &str) -> PathBuf {
            self.dir.path().join(name)
        }
    }

    #[test]
    fn new_folders_are_added_and_nothing_else_changes() {
        let t = Tree::new(&["current", "archive"]);
        let made = plan(
            &[
                want("수집 폴더", &t.at("current")),
                want("보관 폴더", &t.at("archive")),
            ],
            &[],
        )
        .unwrap();
        assert_eq!(made.add.len(), 2);
        assert!(made.keep.is_empty() && made.remove.is_empty());
    }

    #[test]
    fn folders_that_are_automatic_already_make_an_empty_plan() {
        let t = Tree::new(&["current"]);
        let registered = [folder("a", &t.at("current"), true)];
        let made = plan(&[want("수집 폴더", &t.at("current"))], &registered).unwrap();
        assert!(made.is_empty());
    }

    #[test]
    fn a_folder_registered_by_hand_at_the_same_place_is_kept_and_made_automatic() {
        let t = Tree::new(&["shows"]);
        std::os::unix::fs::symlink(t.at("shows"), t.at("alias")).unwrap();
        // By the same text, and by a link to the same folder (then the path
        // becomes the one the settings write).
        for typed in ["shows", "alias"] {
            let registered = [folder("m", &t.at("shows"), false)];
            let made = plan(&[want("보관 폴더", &t.at(typed))], &registered).unwrap();
            assert!(made.add.is_empty() && made.remove.is_empty());
            assert_eq!(
                made.keep,
                [("m".to_owned(), t.at(typed).to_str().unwrap().to_owned())]
            );
        }
    }

    #[test]
    fn a_folder_inside_or_around_one_registered_by_hand_is_refused_with_a_sentence() {
        let t = Tree::new(&["downloads/Shows (current)", "other"]);
        let registered = [folder("m", &t.at("downloads"), false)];
        let inside = plan(
            &[want("수집 폴더", &t.at("downloads/Shows (current)"))],
            &registered,
        )
        .unwrap_err();
        assert!(
            inside.contains("수집 폴더") && inside.contains("안에 있어요"),
            "{inside}"
        );

        let registered = [folder("m", &t.at("downloads/Shows (current)"), false)];
        let around = plan(&[want("보관 폴더", &t.at("downloads"))], &registered).unwrap_err();
        assert!(
            around.contains("보관 폴더") && around.contains("등록한 감시 폴더"),
            "{around}"
        );

        // A folder elsewhere is fine.
        assert!(plan(&[want("수집 폴더", &t.at("other"))], &registered).is_ok());
    }

    #[test]
    fn an_automatic_folder_the_settings_stopped_using_is_removed_and_a_changed_one_is_swapped() {
        let t = Tree::new(&["a", "c"]);
        let registered = [folder("a", &t.at("a"), true)];
        let made = plan(&[want("수집 폴더", &t.at("c"))], &registered).unwrap();
        assert_eq!(made.remove, ["a"]);
        assert_eq!(made.add.len(), 1);
        // Nothing wanted: every automatic folder goes, a folder by hand stays.
        let registered = [
            folder("a", &t.at("a"), true),
            folder("m", &t.at("c"), false),
        ];
        let made = plan(&[], &registered).unwrap();
        assert_eq!(made.remove, ["a"]);
    }

    #[test]
    fn the_new_collect_folder_may_sit_where_the_old_automatic_one_was_around() {
        // The old collect folder is going away, so it is no reason to refuse.
        let t = Tree::new(&["shows/current"]);
        let registered = [folder("a", &t.at("shows"), true)];
        let made = plan(&[want("수집 폴더", &t.at("shows/current"))], &registered).unwrap();
        assert_eq!(made.remove, ["a"]);
        assert_eq!(made.add.len(), 1);
    }

    #[test]
    fn a_new_folder_is_read_once_and_one_that_cannot_be_read_is_left_unread() {
        let t = Tree::new(&["current"]);
        fs::create_dir_all(t.at("current/W/Season 01")).unwrap();
        fs::write(t.at("current/W/Season 01/W S01E01.mkv"), "x").unwrap();
        let mut made = plan(
            &[
                want("수집 폴더", &t.at("current")),
                want("보관 폴더", &t.at("missing")),
            ],
            &[],
        )
        .unwrap();
        read_new_folders(&mut made);
        assert_eq!(made.add[0].scan.as_ref().unwrap().works.len(), 1);
        assert!(made.add[1].scan.is_none());
    }

    #[tokio::test]
    async fn a_folder_kept_under_another_spelling_is_settled_and_keeps_its_note() {
        use crate::{
            discovery::Scan,
            store::{
                library::{LibraryError, LibraryStore},
                Db,
            },
        };

        // The settings write the folder as `real`; a folder is registered at
        // the same place as `alias`, and an unregistered one still holds the
        // text `real` (it is that path's to come back to), so the registered
        // one cannot take the settings' spelling.
        let t = Tree::new(&["real"]);
        std::os::unix::fs::symlink(t.at("real"), t.at("alias")).unwrap();
        let store = LibraryStore::new(Db::open_blocking(":memory:").unwrap());
        let scan = || Scan { works: Vec::new() };
        let real = t.at("real").to_str().unwrap().to_owned();
        let (old, _) = store
            .add_folder(real.clone(), scan(), 1, &[])
            .await
            .unwrap();
        assert!(store.remove_folder(&old.id, 2).await.unwrap().is_some());
        let (folder, _) = store
            .add_folder(t.at("alias").to_str().unwrap().to_owned(), scan(), 3, &[])
            .await
            .unwrap();
        store
            .set_watch_note(&folder.id, Some("폴더 3개를 지켜보지 못해요.".into()))
            .await
            .unwrap();

        let wanted = [want("수집 폴더", &t.at("real"))];
        let mut cycles_with_a_change = 0;
        for cycle in 0..3 {
            let registered = store.folders().await.unwrap();
            let made = plan(&wanted, &registered).unwrap();
            if cycle == 0 {
                // The first cycle makes it automatic.
                assert_eq!(made.keep.len(), 1);
            }
            if made.is_empty() {
                continue;
            }
            cycles_with_a_change += 1;
            match store.sync_automatic(made, 0, 10 + cycle).await {
                Ok(_) | Err(LibraryError::Changed) => {}
                Err(e) => panic!("{e}"),
            }
        }
        let after = store.folder(&folder.id).await.unwrap().unwrap();
        assert!(after.automatic);
        // Every cycle after the first finds nothing to change, and the note of
        // the folder is still there.
        assert_eq!(cycles_with_a_change, 1);
        assert_eq!(
            after.watch_note.as_deref(),
            Some("폴더 3개를 지켜보지 못해요.")
        );
    }
}
