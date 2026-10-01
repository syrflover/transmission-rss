//! Path arithmetic for folding per-channel base folders into the app-wide
//! collect folder.
//!
//! Two callers need it: the migration that removed `channels.base_dir`
//! ([`fold_bases`]) and the legacy YAML import ([`common_ancestor`],
//! [`relative_under`]). They differ in what they must keep. The migration must
//! keep every rule's save path *byte for byte* (what the cycle passed to
//! Transmission is `base.join(directory)`), so it works on the literal text and
//! never normalizes. The import builds new rules and only needs path
//! components, so `//`, `.` and trailing slashes do not matter there.
//!
//! Both compare whole path components, never string prefixes: `/downloads/Shows`
//! is not inside `/downloads/Sho`.

use std::path::{Component, Path, PathBuf};

/// The text of a path split at `/`, after its trailing slashes are dropped.
/// An absolute path starts with an empty segment; `/` alone is one empty segment.
fn segments(path: &str) -> Vec<&str> {
    path.trim_end_matches('/').split('/').collect()
}

/// Base folders folded into one collect folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folding {
    /// The collect folder.
    pub collect: String,
    /// For each input base, what has to be put in front of a rule's directory
    /// so that `collect.join(prefixed)` is the text `base.join(directory)` was.
    /// Empty when the base is the collect folder itself.
    pub prefixes: Vec<String>,
}

/// Why some bases cannot be folded into one collect folder. Indices are
/// positions in the `bases` given to [`fold_bases`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoldError {
    /// These bases are empty text; an empty folder is no collect folder.
    Empty(Vec<usize>),
    /// The bases have no leading component in common, which only relative
    /// folders can do; no folder can stand for all of them.
    NoCommonFolder,
    /// A collect folder exists, but the base at this index cannot be written as
    /// it plus a prefix without changing the text its save paths join to (for
    /// example a base ending in more than one `/`).
    NotExact(usize),
}

/// Folds channel base folders into a collect folder.
///
/// `Ok(None)` when there are no bases. When every base is the same text, that
/// text is the collect folder untouched and no rule changes. Otherwise the
/// collect folder is the longest common ancestor by literal path segments, and
/// each base contributes the segments below it as a prefix.
///
/// The result is checked against the contract before it is returned: for every
/// base, the join of the collect folder and [`prefixed`] `(prefix, directory)`
/// must equal the join of the base and `directory`, for the empty directory
/// (which ends in `/`) and a non-empty one alike.
pub fn fold_bases(bases: &[&str]) -> Result<Option<Folding>, FoldError> {
    let Some(&first) = bases.first() else {
        return Ok(None);
    };
    let empty: Vec<usize> = (0..bases.len()).filter(|&i| bases[i].is_empty()).collect();
    if !empty.is_empty() {
        return Err(FoldError::Empty(empty));
    }
    if bases.iter().all(|base| *base == first) {
        return Ok(Some(Folding {
            collect: first.to_owned(),
            prefixes: vec![String::new(); bases.len()],
        }));
    }

    let split: Vec<Vec<&str>> = bases.iter().map(|base| segments(base)).collect();
    let mut keep = (0..split[0].len())
        .take_while(|&i| split.iter().all(|s| s.get(i) == split[0].get(i)))
        .count();
    // An empty segment inside the shared part (`/a//b`) would end the collect
    // folder in a slash, and one at the start of a remainder would make the
    // prefix absolute. Stop before either.
    if let Some(at) = (1..keep).find(|&i| split[0][i].is_empty()) {
        keep = at;
    }
    while keep > 0
        && split
            .iter()
            .any(|s| s.get(keep).is_some_and(|seg| seg.is_empty()))
    {
        keep -= 1;
    }
    if keep == 0 {
        return Err(FoldError::NoCommonFolder);
    }

    let collect = if keep == 1 && split[0][0].is_empty() {
        "/".to_owned()
    } else {
        split[0][..keep].join("/")
    };
    let prefixes: Vec<String> = split
        .iter()
        .map(|s| s[keep.min(s.len())..].join("/"))
        .collect();

    for (index, (base, prefix)) in bases.iter().zip(&prefixes).enumerate() {
        for directory in ["", "Show/Season 01"] {
            let before = Path::new(base).join(directory);
            let after = Path::new(&collect).join(prefixed(prefix, directory));
            // `Path` equality ignores slashes, so compare the text.
            if before.as_os_str() != after.as_os_str() {
                return Err(FoldError::NotExact(index));
            }
        }
    }
    Ok(Some(Folding { collect, prefixes }))
}

/// A rule directory with its channel's `prefix` put in front. An absolute
/// directory is returned as it is, as it was with the old base (a join with an
/// absolute path replaces the base).
pub fn prefixed(prefix: &str, directory: &str) -> String {
    if prefix.is_empty() || Path::new(directory).is_absolute() {
        directory.to_owned()
    } else {
        format!("{prefix}/{directory}")
    }
}

/// The components of `path` with `.` and repeated or trailing slashes gone.
/// `..` is kept: it cannot be resolved without the disk.
fn components(path: &Path) -> Vec<Component<'_>> {
    path.components().collect()
}

/// Whether `path` has a `..` component. Such a path cannot be placed against
/// another by its text: `/d/Shows/../Movies` is textually inside `/d/Shows`
/// but is not.
pub fn has_parent_dir(path: &Path) -> bool {
    path.components().any(|c| c == Component::ParentDir)
}

/// Whether `path` is made of nothing but `.` components (or is empty): it
/// names no folder below the collect folder, so a rule with it saves into the
/// collect folder itself. `.` and `./` are what a person may type for it.
pub fn is_collect_folder_itself(path: &Path) -> bool {
    path.components().all(|c| c == Component::CurDir)
}

/// The folder every path is inside (or is), by whole components, or `None` when
/// they share no leading component or there are no paths. The folder never
/// contains a `..` component: the shared part stops before the first one.
pub fn common_ancestor(paths: &[&Path]) -> Option<PathBuf> {
    let first = components(paths.first()?);
    let shared = (0..first.len())
        .take_while(|&i| first[i] != Component::ParentDir)
        .take_while(|&i| paths.iter().all(|p| components(p).get(i) == first.get(i)))
        .count();
    (shared > 0).then(|| first[..shared].iter().collect())
}

/// What is left of `path` below `folder`, by whole components: empty when they
/// are the same folder, `None` when `path` is not inside `folder`. A rest that
/// has a `..` component counts as not inside: it could lead out of `folder`.
pub fn relative_under(folder: &Path, path: &Path) -> Option<PathBuf> {
    let rest = path.strip_prefix(folder).ok()?;
    (!has_parent_dir(rest)).then(|| rest.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fold(bases: &[&str]) -> Folding {
        fold_bases(bases).unwrap().unwrap()
    }

    #[test]
    fn no_bases_fold_to_nothing() {
        assert_eq!(fold_bases(&[]), Ok(None));
    }

    #[test]
    fn the_same_base_everywhere_is_the_collect_folder_untouched() {
        let folding = fold(&["/downloads/Shows (current)", "/downloads/Shows (current)"]);
        assert_eq!(folding.collect, "/downloads/Shows (current)");
        assert_eq!(folding.prefixes, ["", ""]);

        // Trailing slash and all: nothing is rewritten when nothing differs.
        assert_eq!(fold(&["/d/x/"]).collect, "/d/x/");
    }

    #[test]
    fn differing_bases_fold_to_their_common_ancestor_by_components() {
        let folding = fold(&["/downloads/Shows (current)", "/downloads/Movies"]);
        assert_eq!(folding.collect, "/downloads");
        assert_eq!(folding.prefixes, ["Shows (current)", "Movies"]);

        // `/d/Show` is not inside `/d/Sho`: a string prefix is not an ancestor.
        let folding = fold(&["/d/Sho", "/d/Show"]);
        assert_eq!(folding.collect, "/d");
        assert_eq!(folding.prefixes, ["Sho", "Show"]);

        // One base above another: it keeps an empty prefix.
        let folding = fold(&["/downloads", "/downloads/Movies/2025"]);
        assert_eq!(folding.collect, "/downloads");
        assert_eq!(folding.prefixes, ["", "Movies/2025"]);
    }

    #[test]
    fn bases_with_only_the_root_in_common_fold_to_the_root() {
        let folding = fold(&["/media", "/downloads/x"]);
        assert_eq!(folding.collect, "/");
        assert_eq!(folding.prefixes, ["media", "downloads/x"]);
        assert_eq!(
            Path::new(&folding.collect).join(prefixed("media", "Show")),
            Path::new("/media").join("Show")
        );
    }

    #[test]
    fn odd_spelling_of_a_base_keeps_the_save_path_text() {
        for bases in [
            &["/d/a/", "/d/b"][..],
            &["/d//a", "/d//b"][..],
            &["/d/a/", "/d/a", "/d/b/"][..],
            &["/a//b", "/a//c"][..],
            &["/", "/media"][..],
        ] {
            let folding = fold(bases);
            for (base, prefix) in bases.iter().zip(&folding.prefixes) {
                for directory in ["", "Show", "Show/Season 01"] {
                    assert_eq!(
                        Path::new(&folding.collect)
                            .join(prefixed(prefix, directory))
                            .as_os_str(),
                        Path::new(base).join(directory).as_os_str(),
                        "{bases:?} {base} {directory:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn an_empty_directory_keeps_its_trailing_slash() {
        let folding = fold(&["/downloads/Shows", "/downloads/Movies"]);
        assert_eq!(prefixed(&folding.prefixes[0], ""), "Shows/");
        assert_eq!(
            Path::new(&folding.collect)
                .join(prefixed(&folding.prefixes[0], ""))
                .to_str(),
            Path::new("/downloads/Shows").join("").to_str()
        );
        assert_eq!(
            Path::new(&folding.collect)
                .join(prefixed(&folding.prefixes[0], ""))
                .to_str(),
            Some("/downloads/Shows/")
        );
    }

    #[test]
    fn an_absolute_directory_is_left_alone() {
        assert_eq!(prefixed("Shows", "/abs/x"), "/abs/x");
    }

    #[test]
    fn relative_folders_with_nothing_in_common_cannot_be_folded() {
        assert_eq!(fold_bases(&["a/x", "b/y"]), Err(FoldError::NoCommonFolder));
        assert_eq!(fold_bases(&["/a", "b"]), Err(FoldError::NoCommonFolder));
        let folding = fold(&["rel/a", "rel/b"]);
        assert_eq!(folding.collect, "rel");
    }

    #[test]
    fn folding_says_which_bases_it_refused_and_why() {
        assert_eq!(
            fold_bases(&["/d/a", "", "/d/b", ""]),
            Err(FoldError::Empty(vec![1, 3]))
        );
        assert_eq!(fold_bases(&[""]), Err(FoldError::Empty(vec![0])));
        // Two trailing slashes cannot be put back byte for byte.
        assert_eq!(fold_bases(&["/d/a", "/d/b//"]), Err(FoldError::NotExact(1)));
        assert_eq!(fold_bases(&["/d/a//", "/d/b"]), Err(FoldError::NotExact(0)));
    }

    #[test]
    fn a_rest_with_parent_components_is_not_inside() {
        let folder = Path::new("/downloads/Shows");
        assert_eq!(
            relative_under(folder, Path::new("/downloads/Shows/../Movies")),
            None
        );
        assert_eq!(
            relative_under(folder, Path::new("/downloads/Shows/A/../../Movies")),
            None
        );
        assert_eq!(
            relative_under(folder, Path::new("/downloads/Shows/..")),
            None
        );
        // `.` is harmless and dropped by the component view.
        assert_eq!(
            relative_under(folder, Path::new("/downloads/Shows/./A")),
            Some(PathBuf::from("A"))
        );
        assert!(has_parent_dir(Path::new("/a/../b")));
        assert!(!has_parent_dir(Path::new("/a/b..c/..d")));
        // Only `.` components (or nothing) name the collect folder itself.
        for itself in ["", ".", "./", "././", "./."] {
            assert!(is_collect_folder_itself(Path::new(itself)), "{itself:?}");
        }
        for below in ["a", "./a", "a/.", "..", ".a", "..."] {
            assert!(!is_collect_folder_itself(Path::new(below)), "{below:?}");
        }
    }

    #[test]
    fn the_common_ancestor_stops_before_a_parent_component() {
        assert_eq!(
            common_ancestor(&[Path::new("/d/Shows/../A"), Path::new("/d/Shows/../B")]),
            Some(PathBuf::from("/d/Shows"))
        );
        assert_eq!(
            common_ancestor(&[Path::new("/d/../A"), Path::new("/d/../A")]),
            Some(PathBuf::from("/d"))
        );
        assert_eq!(
            common_ancestor(&[Path::new("../A"), Path::new("../A")]),
            None
        );
    }

    #[test]
    fn the_import_helpers_compare_whole_components() {
        let paths = [Path::new("/d/Shows"), Path::new("/d/Movies/")];
        assert_eq!(common_ancestor(&paths), Some(PathBuf::from("/d")));
        assert_eq!(
            common_ancestor(&[Path::new("/d/Shows"), Path::new("/d/Shows/")]),
            Some(PathBuf::from("/d/Shows"))
        );
        assert_eq!(common_ancestor(&[Path::new("a"), Path::new("/b")]), None);
        assert_eq!(common_ancestor(&[]), None);

        let folder = Path::new("/downloads/Shows");
        assert_eq!(
            relative_under(folder, Path::new("/downloads/Shows/")),
            Some(PathBuf::new())
        );
        assert_eq!(
            relative_under(folder, Path::new("/downloads/Shows/Old/2024")),
            Some(PathBuf::from("Old/2024"))
        );
        assert_eq!(relative_under(folder, Path::new("/downloads/Shows2")), None);
        assert_eq!(relative_under(folder, Path::new("/downloads")), None);
    }
}
