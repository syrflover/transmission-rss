//! The releases, rules and torrents the tests of `다시 받기` share. The
//! items retried here were picked by a rule whose add Transmission refused
//! ([`World::fail_adds`]), so they are `add_failed` with the rule recorded, as
//! the rule's own cycle leaves them. Names and hashes are made up.

use trss_transmission::{fake::FakeTorrent, BOT_LABEL};

use crate::{
    store::{channels::RuleInput, history::HistoryItem},
    test_world::World,
};

/// The command the tests accept.
pub(super) const CMD: &str = "0b7d5a44-6c1e-4c62-9a6a-3f0c1d2e4b55";
/// Another command, for a second request.
pub(super) const SECOND_CMD: &str = "1e2d3c4b-0000-4000-8000-000000000002";

pub(super) const LIAR: &str = "[SubsPlease] LIAR GAME - 26 (1080p) [ABCD1234].mkv";
pub(super) const OTHER: &str = "[SubsPlease] Another Show - 03 (1080p) [ABCD1235].mkv";
/// The folder the rule of `LIAR` saves into, under the collect folder.
pub(super) const LIAR_DIR: &str = "LIAR GAME/Season 01";

/// A hash for the n-th made-up release.
pub(super) fn hash(n: u32) -> String {
    format!("cccc{n:036}")
}

pub(super) fn rule(match_text: &str, directory: &str) -> RuleInput {
    RuleInput {
        r#match: Some(match_text.to_owned()),
        directory: directory.to_owned(),
        ..Default::default()
    }
}

/// A rule for each release above, with the folders a person would give them.
pub(super) fn picked_rules() -> Vec<RuleInput> {
    vec![
        rule("LIAR GAME", LIAR_DIR),
        rule("Another Show", "Another Show/Season 01"),
    ]
}

/// A rule that matches neither release above, so both are `no_match`.
pub(super) fn unrelated_rule() -> Vec<RuleInput> {
    vec![rule("Some Other Show", "Some Other Show/Season 01")]
}

impl World {
    /// The world of [`picked_rules`], with `LIAR` refused by Transmission.
    pub async fn liar_failed() -> (World, HistoryItem) {
        let s = World::with_rules(picked_rules()).await;
        s.fail_adds(&[(&hash(26), LIAR)]).await;
        let item = s.item_containing("LIAR GAME - 26").await;
        (s, item)
    }

    /// The torrent Transmission holds after taking this command's add for
    /// `item` into the rule's folder: the labels of the add, as an earlier
    /// start of the command left them.
    pub fn taken_by_the_commands_add(&self, item: &HistoryItem) -> FakeTorrent {
        FakeTorrent {
            labels: vec![
                BOT_LABEL.to_owned(),
                self.item_label(item),
                trss_transmission::command_label(CMD),
            ],
            ..FakeTorrent::new(&hash(26), LIAR).in_dir(self.media.join(LIAR_DIR))
        }
    }

    /// Whether any torrent still carries a label of a command.
    pub fn carries_a_command_label(&self) -> bool {
        self.tr
            .torrents()
            .iter()
            .any(|t| t.labels.iter().any(|l| l.starts_with("trss-cmd:")))
    }
}
