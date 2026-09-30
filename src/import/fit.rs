//! Placing the file's channels under the app's collect folder.
//!
//! The legacy file gives every channel its own `directory`; the app has one
//! collect folder and each rule's directory is relative to it
//! ([`crate::rss::save_path`]). So an import decides, for each channel, where
//! its folder stands against the collect folder:
//!
//! - **No collect folder set yet.** The import sets it from the file: the
//!   channels' shared folder when they all use the same one, otherwise their
//!   longest common ancestor (by whole path components). Every channel is then
//!   inside it. Channels with nothing in common (only possible for relative
//!   folders) take the first channel's folder as the collect folder, and the
//!   others fall outside.
//! - **A collect folder set.** A channel folder equal to it or inside it is
//!   imported, with what lies between the two put in front of each rule's
//!   directory, so the rule saves where the old file put it. A channel folder
//!   outside it is not imported and is reported with the reason; moving it is
//!   not this import's business.

use std::path::Path;

use super::legacy::LegacyChannel;
use crate::folders::{common_ancestor, prefixed, relative_under};
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

/// Places `file` under `current`, the collect folder as it is now.
pub fn fit(file: Vec<LegacyChannel>, current: Option<&str>) -> Fitted {
    let (collect, to_set) = match current {
        Some(folder) => (Path::new(folder).to_path_buf(), None),
        None => {
            let folders: Vec<&Path> = file.iter().map(|c| Path::new(&c.folder)).collect();
            let shared = common_ancestor(&folders)
                .or_else(|| folders.first().map(|first| first.to_path_buf()));
            match shared {
                Some(folder) => {
                    let text = folder.to_string_lossy().into_owned();
                    (folder, Some(text))
                }
                // An empty file has nothing to import; `parse` refuses it first.
                None => {
                    return Fitted {
                        collect_folder: None,
                        channels: Vec::new(),
                    }
                }
            }
        }
    };

    let channels = file
        .into_iter()
        .map(|legacy| match relative_under(&collect, Path::new(&legacy.folder)) {
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
        })
        .collect();

    Fitted {
        collect_folder: to_set,
        channels,
    }
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

    fn directories(fit: &Fit) -> Vec<&str> {
        match fit {
            Fit::Import(c) => c.rules.iter().map(|r| r.directory.as_str()).collect(),
            Fit::Outside(_) => panic!("expected an imported channel"),
        }
    }

    #[test]
    fn with_no_collect_folder_one_shared_channel_folder_becomes_it() {
        let fitted = fit(
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
        let fitted = fit(
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
        let fitted = fit(
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
        let fitted = fit(
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

    #[test]
    fn folders_with_nothing_in_common_take_the_first_as_the_collect_folder() {
        let fitted = fit(vec![legacy("a/x", &["A"]), legacy("b/y", &["B"])], None);
        assert_eq!(fitted.collect_folder.as_deref(), Some("a/x"));
        assert!(matches!(fitted.channels[0], Fit::Import(_)));
        assert!(matches!(fitted.channels[1], Fit::Outside(_)));
    }

    #[test]
    fn a_rule_saves_where_the_old_file_put_it() {
        let file = || {
            vec![
                legacy("/downloads/Shows (current)", &["A/Season 01", ""]),
                legacy("/downloads/Movies/", &["Dune", ""]),
            ]
        };
        let fitted = fit(file(), None);
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
