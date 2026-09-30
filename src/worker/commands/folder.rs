//! The save folder of a one-off receive: a path below the channel's base folder.
//!
//! The screen sends the part after the base folder (`LIAR GAME/Season 01`; empty
//! means the base folder itself). [`resolve`] turns it into the folder handed to
//! Transmission, refusing everything that could end up outside the base folder:
//!
//! - **Lexically**: an absolute path, `..`, a backslash or a control character.
//!   `.` and empty parts (`a//b`) are dropped.
//! - **By links**: a folder that already exists below the base folder and is (or
//!   passes through) a symbolic link leading outside it. This needs to see the
//!   folder, and the web or worker may not have the media volume mounted; when
//!   the base folder cannot be seen here, only the lexical rules apply. The
//!   worker checks again when it runs the command, so a link made after the
//!   request was accepted is caught too.

use std::{
    fs,
    path::{Path, PathBuf},
};

/// Why a save folder was refused. [`FolderError::message`] is the sentence for
/// the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderError {
    Absolute,
    ParentDir,
    BadCharacter,
    /// A link inside the base folder leads outside it.
    LeavesBase,
}

impl FolderError {
    pub fn message(self) -> &'static str {
        match self {
            FolderError::Absolute => {
                "저장 폴더는 채널의 기본 폴더 아래에 있는 폴더 이름으로 적어 주세요. 절대 경로는 쓸 수 없어요."
            }
            FolderError::ParentDir => {
                "저장 폴더에 `..`은 쓸 수 없어요. 채널의 기본 폴더 밖에는 받을 수 없어요."
            }
            FolderError::BadCharacter => {
                "저장 폴더에 쓸 수 없는 문자가 있어요. 폴더 이름을 `/`로 구분해 적어 주세요."
            }
            FolderError::LeavesBase => {
                "저장 폴더가 링크를 따라 채널의 기본 폴더 밖으로 나가요. 기본 폴더 안에 있는 폴더를 적어 주세요."
            }
        }
    }
}

impl std::fmt::Display for FolderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for FolderError {}

/// The folder parts of `folder`, checked lexically.
pub fn parts(folder: &str) -> Result<Vec<&str>, FolderError> {
    let folder = folder.trim();
    if folder.chars().any(|c| c.is_control() || c == '\\') {
        return Err(FolderError::BadCharacter);
    }
    if folder.starts_with('/')
        || folder == "~"
        || folder.starts_with("~/")
        || has_drive_letter(folder)
    {
        return Err(FolderError::Absolute);
    }
    let mut out = Vec::new();
    for part in folder.split('/') {
        match part.trim() {
            "" | "." => {}
            ".." => return Err(FolderError::ParentDir),
            part => out.push(part),
        }
    }
    Ok(out)
}

/// `C:` or `c:\` at the start: absolute on Windows-style paths.
fn has_drive_letter(folder: &str) -> bool {
    let mut chars = folder.chars();
    matches!((chars.next(), chars.next()), (Some(c), Some(':')) if c.is_ascii_alphabetic())
}

/// The save folder for `folder` below `base_dir`, or why it is refused. See
/// the module docs for what is checked.
pub fn resolve(base_dir: &str, folder: &str) -> Result<PathBuf, FolderError> {
    let parts = parts(folder)?;
    let base = PathBuf::from(base_dir);
    let mut target = base.clone();
    target.extend(&parts);
    if leaves_base_by_link(&base, &parts) {
        return Err(FolderError::LeavesBase);
    }
    Ok(target)
}

/// Walks the folder parts down from `base`. A part that exists as a link must
/// resolve to somewhere inside the base folder, and a link that leads nowhere
/// counts as leaving it (Transmission would create the missing target).
fn leaves_base_by_link(base: &Path, parts: &[&str]) -> bool {
    let Ok(real_base) = base.canonicalize() else {
        // Not visible from here: only the lexical rules can be applied.
        return false;
    };
    let mut current = base.to_path_buf();
    for part in parts {
        current.push(part);
        let Ok(metadata) = fs::symlink_metadata(&current) else {
            // It does not exist (or cannot be read), and neither does anything below.
            return false;
        };
        if metadata.file_type().is_symlink() {
            match current.canonicalize() {
                Ok(real) if real.starts_with(&real_base) => {}
                _ => return true,
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_folder_is_the_base_folder() {
        assert_eq!(
            resolve("/media/anime", ""),
            Ok(PathBuf::from("/media/anime"))
        );
        assert_eq!(
            resolve("/media/anime", "  "),
            Ok(PathBuf::from("/media/anime"))
        );
        assert_eq!(
            resolve("/media/anime/", "./"),
            Ok(PathBuf::from("/media/anime/"))
        );
    }

    #[test]
    fn a_relative_folder_goes_below_the_base_folder() {
        assert_eq!(
            resolve("/media/anime", "LIAR GAME/Season 01"),
            Ok(PathBuf::from("/media/anime/LIAR GAME/Season 01"))
        );
        assert_eq!(
            resolve("/media/anime", " LIAR GAME //./ Season 01/ "),
            Ok(PathBuf::from("/media/anime/LIAR GAME/Season 01"))
        );
    }

    #[test]
    fn parent_folders_are_refused_wherever_they_appear() {
        for folder in [
            "..",
            "../..",
            "../../etc",
            "a/../b",
            "a/..",
            "a/b/../../../c",
        ] {
            assert_eq!(
                resolve("/media/anime", folder),
                Err(FolderError::ParentDir),
                "{folder}"
            );
        }
    }

    #[test]
    fn absolute_paths_are_refused() {
        for folder in [
            "/etc",
            "/media/anime/LIAR GAME",
            "//host/share",
            "~/x",
            "C:/x",
            "c:x",
        ] {
            assert_eq!(
                resolve("/media/anime", folder),
                Err(FolderError::Absolute),
                "{folder}"
            );
        }
    }

    #[test]
    fn backslashes_and_control_characters_are_refused() {
        for folder in ["a\\b", "..\\..", "a\u{0}b", "a\nb", "a\tb"] {
            assert_eq!(
                resolve("/media/anime", folder),
                Err(FolderError::BadCharacter),
                "{folder:?}"
            );
        }
    }

    #[test]
    fn a_name_that_merely_contains_dots_is_fine() {
        assert_eq!(
            resolve("/media/anime", "Mr. Bean.../..hidden/Season 01"),
            Ok(PathBuf::from("/media/anime/Mr. Bean.../..hidden/Season 01"))
        );
    }

    #[test]
    fn every_refusal_has_a_sentence_for_the_user() {
        for error in [
            FolderError::Absolute,
            FolderError::ParentDir,
            FolderError::BadCharacter,
            FolderError::LeavesBase,
        ] {
            assert!(error.message().ends_with("요."), "{error:?}");
        }
    }

    #[cfg(unix)]
    mod links {
        use std::os::unix::fs::symlink;

        use super::*;

        struct Media {
            _dir: tempfile::TempDir,
            base: PathBuf,
            outside: PathBuf,
        }

        fn media() -> Media {
            let dir = tempfile::tempdir().unwrap();
            let base = dir.path().join("anime");
            let outside = dir.path().join("elsewhere");
            fs::create_dir_all(base.join("LIAR GAME/Season 01")).unwrap();
            fs::create_dir_all(&outside).unwrap();
            Media {
                _dir: dir,
                base,
                outside,
            }
        }

        fn resolve_in(media: &Media, folder: &str) -> Result<PathBuf, FolderError> {
            resolve(media.base.to_str().unwrap(), folder)
        }

        #[test]
        fn an_existing_folder_below_the_base_is_accepted() {
            let media = media();
            assert_eq!(
                resolve_in(&media, "LIAR GAME/Season 01"),
                Ok(media.base.join("LIAR GAME/Season 01"))
            );
            // A folder that does not exist yet is fine too.
            assert_eq!(
                resolve_in(&media, "New Show/Season 01"),
                Ok(media.base.join("New Show/Season 01"))
            );
        }

        #[test]
        fn a_link_to_a_folder_outside_the_base_is_refused() {
            let media = media();
            symlink(&media.outside, media.base.join("escape")).unwrap();

            assert_eq!(resolve_in(&media, "escape"), Err(FolderError::LeavesBase));
            assert_eq!(
                resolve_in(&media, "escape/Season 01"),
                Err(FolderError::LeavesBase)
            );
            // Also from the middle of a longer path.
            symlink(&media.outside, media.base.join("LIAR GAME/out")).unwrap();
            assert_eq!(
                resolve_in(&media, "LIAR GAME/out/Season 01"),
                Err(FolderError::LeavesBase)
            );
        }

        #[test]
        fn a_link_that_leads_nowhere_counts_as_leaving_the_base() {
            let media = media();
            symlink(
                media.outside.join("not made yet"),
                media.base.join("dangling"),
            )
            .unwrap();

            assert_eq!(resolve_in(&media, "dangling"), Err(FolderError::LeavesBase));
        }

        #[test]
        fn a_link_that_stays_inside_the_base_is_accepted() {
            let media = media();
            symlink(media.base.join("LIAR GAME"), media.base.join("alias")).unwrap();

            assert_eq!(
                resolve_in(&media, "alias/Season 01"),
                Ok(media.base.join("alias/Season 01"))
            );
        }

        #[test]
        fn a_base_folder_that_is_itself_a_link_is_fine() {
            let media = media();
            let alias = media.base.parent().unwrap().join("anime-alias");
            symlink(&media.base, &alias).unwrap();

            assert_eq!(
                resolve(alias.to_str().unwrap(), "LIAR GAME"),
                Ok(alias.join("LIAR GAME"))
            );
        }
    }
}
