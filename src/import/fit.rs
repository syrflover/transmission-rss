//! Placing the file's channels under the app's collect folder.
//!
//! The legacy file gives every channel its own `directory`; the app has one
//! collect folder and each rule's directory is relative to it
//! ([`crate::rss::save_path`]). So an import decides, for each channel, where
//! its folder stands against the collect folder:
//!
//! - **No collect folder set yet.** The import sets it from the file: the
//!   channels' shared folder when they all use the same one, otherwise their
//!   longest common ancestor (by whole path components). Every channel with an
//!   absolute folder is then inside it; a relative one falls outside. The
//!   folder the import would set must be one the settings screen would accept
//!   in kind: absolute, and not the filesystem root (nothing could be archived
//!   beside it). When it would not be, the whole import is refused with the
//!   reason ([`Refused`]) and the user sets the collect folder first.
//! - **A collect folder set.** A channel folder equal to it or inside it is
//!   imported, with what lies between the two put in front of each rule's
//!   directory, so the rule saves where the old file put it. A channel folder
//!   outside it is not imported and is reported with the reason; moving it is
//!   not this import's business.
//!
//! A channel folder with a `..` component is never placed: its text says
//! nothing reliable about where it is, and a rule directory built from it could
//! leave the collect folder. It is reported like one outside the folder and
//! takes no part in choosing the collect folder.

use std::path::{Component, Path, PathBuf};

use super::legacy::LegacyChannel;
use crate::folders::{common_ancestor, has_parent_dir, prefixed, relative_under};
use crate::store::channels::import::ImportChannel;

/// What an import does with one channel of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fit {
    /// Imported, its rules' directories relative to the collect folder.
    Import(ImportChannel),
    /// Not imported; the reason is a sentence for the user.
    Outside(String),
}

/// The channels placed under the collect folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fitted {
    /// The collect folder this import would set, when none is set yet.
    pub collect_folder: Option<String>,
    /// One per file channel, in file order.
    pub channels: Vec<Fit>,
}

/// The import cannot set a usable collect folder from the file, so none of it
/// is imported. The reason is a sentence for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

const SET_FIRST: &str = "설정에서 수집 폴더를 먼저 정한 뒤 다시 가져와 주세요.";

fn is_root(folder: &Path) -> bool {
    folder.components().all(|c| c == Component::RootDir)
}

fn parent_reason(folder: &str) -> Fit {
    Fit::Outside(format!(
        "채널 폴더 `{folder}`에 `..`가 들어 있어서 가져오지 않아요. `..` 없는 전체 경로로 고친 뒤 다시 가져와 주세요."
    ))
}

/// Places `file` under `current`, the collect folder as it is now.
pub fn fit(file: Vec<LegacyChannel>, current: Option<&str>) -> Result<Fitted, Refused> {
    let (collect, to_set) = match current {
        Some(folder) => (PathBuf::from(folder), None),
        None => {
            if file.is_empty() {
                // An empty file has nothing to import; `parse` refuses it first.
                return Ok(Fitted {
                    collect_folder: None,
                    channels: Vec::new(),
                });
            }
            let placeable: Vec<&Path> = file
                .iter()
                .map(|c| Path::new(&c.folder))
                .filter(|folder| !has_parent_dir(folder))
                .collect();
            if placeable.is_empty() {
                // Every folder has a `..`: nothing can be placed, nothing is set.
                return Ok(Fitted {
                    collect_folder: None,
                    channels: file.iter().map(|c| parent_reason(&c.folder)).collect(),
                });
            }
            let absolute: Vec<&Path> = placeable
                .into_iter()
                .filter(|folder| folder.is_absolute())
                .collect();
            // All absolute folders start at the root, so they have an ancestor.
            let Some(folder) = common_ancestor(&absolute) else {
                return Err(Refused(format!(
                    "파일의 채널 폴더가 `/`로 시작하는 전체 경로가 아니어서, 가져오면서 수집 폴더를 정할 수 없어요. {SET_FIRST}"
                )));
            };
            if is_root(&folder) {
                return Err(Refused(format!(
                    "파일의 채널 폴더들이 `/` 말고는 겹치는 곳이 없어서, 수집 폴더가 `/`가 되어 버려요. `/`는 수집 폴더로 쓸 수 없으니(옆에 보관 폴더를 둘 수 없어요) 가져오지 않았어요. {SET_FIRST}"
                )));
            }
            let text = folder.to_string_lossy().into_owned();
            (folder, Some(text))
        }
    };

    let channels = file
        .into_iter()
        .map(|legacy| {
            if has_parent_dir(Path::new(&legacy.folder)) {
                return parent_reason(&legacy.folder);
            }
            match relative_under(&collect, Path::new(&legacy.folder)) {
                Some(rest) => {
                    let prefix = rest.to_string_lossy().into_owned();
                    let mut channel = legacy.channel;
                    for rule in &mut channel.rules {
                        rule.directory = prefixed(&prefix, &rule.directory);
                    }
                    Fit::Import(channel)
                }
                None => Fit::Outside(format!(
                    "채널 폴더 `{}`가 수집 폴더 `{}` 밖에 있어서 가져오지 않아요. 채널 폴더를 수집 폴더 안으로 옮기거나, 설정에서 수집 폴더를 이 폴더들을 모두 품는 폴더로 바꾼 뒤 다시 가져와 주세요.",
                    legacy.folder,
                    collect.display()
                )),
            }
        })
        .collect();

    Ok(Fitted {
        collect_folder: to_set,
        channels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::channels::{ChannelInput, RuleInput};

    fn legacy(folder: &str, directories: &[&str]) -> LegacyChannel {
        LegacyChannel {
            folder: folder.to_owned(),
            channel: ImportChannel {
                input: ChannelInput::new("https://x.test/rss"),
                rules: directories
                    .iter()
                    .map(|d| RuleInput {
                        directory: (*d).to_owned(),
                        ..RuleInput::default()
                    })
                    .collect(),
            },
        }
    }

    fn place(file: Vec<LegacyChannel>, current: Option<&str>) -> Fitted {
        fit(file, current).unwrap()
    }

    fn directories(fit: &Fit) -> Vec<&str> {
        match fit {
            Fit::Import(c) => c.rules.iter().map(|r| r.directory.as_str()).collect(),
            Fit::Outside(_) => panic!("expected an imported channel"),
        }
    }

    #[test]
    fn with_no_collect_folder_one_shared_channel_folder_becomes_it() {
        let fitted = place(
            vec![
                legacy("/downloads/Shows (current)", &["A/Season 01", ""]),
                legacy("/downloads/Shows (current)/", &["B"]),
            ],
            None,
        );
        assert_eq!(
            fitted.collect_folder.as_deref(),
            Some("/downloads/Shows (current)")
        );
        assert_eq!(directories(&fitted.channels[0]), ["A/Season 01", ""]);
        assert_eq!(directories(&fitted.channels[1]), ["B"]);
    }

    #[test]
    fn with_no_collect_folder_differing_folders_share_their_common_ancestor() {
        let fitted = place(
            vec![
                legacy("/downloads/Shows (current)", &["A/Season 01", ""]),
                legacy("/downloads/Movies", &["Dune"]),
            ],
            None,
        );
        assert_eq!(fitted.collect_folder.as_deref(), Some("/downloads"));
        assert_eq!(
            directories(&fitted.channels[0]),
            ["Shows (current)/A/Season 01", "Shows (current)/"]
        );
        assert_eq!(directories(&fitted.channels[1]), ["Movies/Dune"]);
    }

    #[test]
    fn a_channel_folder_inside_the_collect_folder_gets_the_rest_in_front_of_its_rules() {
        let fitted = place(
            vec![
                legacy("/downloads/Shows (current)", &["A"]),
                legacy("/downloads/Shows (current)/Old", &["B", ""]),
                legacy("/downloads/Shows (current)/", &["C"]),
            ],
            Some("/downloads/Shows (current)"),
        );
        assert_eq!(fitted.collect_folder, None);
        assert_eq!(directories(&fitted.channels[0]), ["A"]);
        assert_eq!(directories(&fitted.channels[1]), ["Old/B", "Old/"]);
        assert_eq!(directories(&fitted.channels[2]), ["C"]);
    }

    #[test]
    fn a_channel_folder_outside_the_collect_folder_is_reported_not_imported() {
        let fitted = place(
            vec![
                legacy("/downloads/Shows (current)", &["A"]),
                legacy("/downloads/Movies", &["Dune"]),
                // A string prefix is not inside.
                legacy("/downloads/Shows (current)2", &["X"]),
                legacy("relative/Shows", &["Y"]),
            ],
            Some("/downloads/Shows (current)"),
        );
        assert!(matches!(fitted.channels[0], Fit::Import(_)));
        for outside in &fitted.channels[1..] {
            let Fit::Outside(reason) = outside else {
                panic!("expected the channel to fall outside");
            };
            assert!(
                reason.contains("수집 폴더 `/downloads/Shows (current)` 밖"),
                "{reason}"
            );
        }
        let Fit::Outside(reason) = &fitted.channels[1] else {
            unreachable!()
        };
        assert!(reason.contains("`/downloads/Movies`"), "{reason}");
    }

    fn refusal(file: Vec<LegacyChannel>, current: Option<&str>) -> String {
        match fit(file, current) {
            Err(Refused(reason)) => reason,
            Ok(fitted) => panic!("expected a refusal, got {fitted:?}"),
        }
    }

    fn outside_reason(fit: &Fit) -> &str {
        match fit {
            Fit::Outside(reason) => reason,
            Fit::Import(_) => panic!("expected the channel to fall outside"),
        }
    }

    #[test]
    fn with_no_collect_folder_absolute_folders_win_over_relative_ones() {
        let fitted = place(
            vec![
                legacy("relative/x", &["A"]),
                legacy("/downloads/a", &["B"]),
                legacy("/downloads/b", &["C"]),
            ],
            None,
        );
        assert_eq!(fitted.collect_folder.as_deref(), Some("/downloads"));
        assert!(outside_reason(&fitted.channels[0]).contains("`relative/x`"));
        assert_eq!(directories(&fitted.channels[1]), ["a/B"]);
        assert_eq!(directories(&fitted.channels[2]), ["b/C"]);
    }

    #[test]
    fn relative_channel_folders_alone_cannot_set_the_collect_folder() {
        let reason = refusal(vec![legacy("a/x", &["A"]), legacy("b/y", &["B"])], None);
        assert!(reason.contains("전체 경로가 아니어서"), "{reason}");
        assert!(reason.contains("설정에서 수집 폴더를 먼저"), "{reason}");
        // The same folder twice folds to a relative path too.
        let reason = refusal(
            vec![legacy("rel/a", &["A"]), legacy("rel/a/", &["B"])],
            None,
        );
        assert!(reason.contains("전체 경로가 아니어서"), "{reason}");
    }

    #[test]
    fn a_common_ancestor_that_is_the_filesystem_root_is_refused() {
        let reason = refusal(
            vec![legacy("/media/a", &["A"]), legacy("/downloads/b", &["B"])],
            None,
        );
        assert!(reason.contains("`/`가 되어"), "{reason}");
        assert!(reason.contains("설정에서 수집 폴더를 먼저"), "{reason}");
        // One channel whose folder is `/` itself, too.
        let reason = refusal(vec![legacy("/", &["A"])], None);
        assert!(reason.contains("`/`가 되어"), "{reason}");
        // With a collect folder set, no such refusal applies.
        let fitted = place(
            vec![legacy("/media/a", &["A"]), legacy("/downloads/b", &["B"])],
            Some("/media"),
        );
        assert_eq!(directories(&fitted.channels[0]), ["a/A"]);
        assert!(matches!(fitted.channels[1], Fit::Outside(_)));
    }

    #[test]
    fn a_channel_folder_with_parent_components_is_reported_not_placed() {
        // Textually inside `/downloads/Shows`, but it leads out of it.
        let fitted = place(
            vec![
                legacy("/downloads/Shows", &["A"]),
                legacy("/downloads/Shows/../Movies", &["Dune"]),
                legacy("/downloads/Shows/Old/../../Other", &["X"]),
            ],
            Some("/downloads/Shows"),
        );
        assert_eq!(directories(&fitted.channels[0]), ["A"]);
        for outside in &fitted.channels[1..] {
            assert!(outside_reason(outside).contains("`..`"), "{outside:?}");
        }
        assert!(outside_reason(&fitted.channels[1]).contains("`/downloads/Shows/../Movies`"));
    }

    #[test]
    fn a_channel_folder_with_parent_components_takes_no_part_in_choosing_the_collect_folder() {
        let fitted = place(
            vec![
                legacy("/downloads/Shows/A", &["A"]),
                legacy("/downloads/Shows/../Movies", &["Dune"]),
                legacy("/downloads/Shows/B", &["B"]),
            ],
            None,
        );
        // Not `/downloads`, which the channel with `..` would seem to need.
        assert_eq!(fitted.collect_folder.as_deref(), Some("/downloads/Shows"));
        assert_eq!(directories(&fitted.channels[0]), ["A/A"]);
        assert!(outside_reason(&fitted.channels[1]).contains("`..`"));
        assert_eq!(directories(&fitted.channels[2]), ["B/B"]);

        // Only folders with `..`: nothing is placed and no collect folder is chosen.
        let fitted = place(
            vec![legacy("/d/../x", &["A"]), legacy("../y", &["B"])],
            None,
        );
        assert_eq!(fitted.collect_folder, None);
        assert!(fitted
            .channels
            .iter()
            .all(|c| outside_reason(c).contains("`..`")));
    }

    #[test]
    fn a_rule_saves_where_the_old_file_put_it() {
        let file = || {
            vec![
                legacy("/downloads/Shows (current)", &["A/Season 01", ""]),
                legacy("/downloads/Movies/", &["Dune", ""]),
            ]
        };
        let fitted = place(file(), None);
        let collect = fitted.collect_folder.clone().unwrap();
        for (legacy, fit) in file().iter().zip(&fitted.channels) {
            let Fit::Import(channel) = fit else {
                panic!("expected an import")
            };
            for (old, new) in legacy.channel.rules.iter().zip(&channel.rules) {
                assert_eq!(
                    Path::new(&collect).join(&new.directory),
                    Path::new(&legacy.folder).join(&old.directory)
                );
            }
        }
    }
}
