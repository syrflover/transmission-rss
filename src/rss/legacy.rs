//! Adapter from the legacy YAML channel configuration to the shared evaluation,
//! used by the `transmission-rss` binary.

use std::path::PathBuf;

use rss::{Channel, Item};

use super::{ChannelEvaluator, Outcome};
use crate::config::ChannelConfig;

/// An item selected for download, with the inputs the binary needs to add and rename it.
pub struct SelectedItem<'a> {
    /// Where Transmission should download the item.
    pub save_path: PathBuf,
    /// The applied rule's `episode`, the start episode passed to the renamer.
    pub episode: isize,
    pub item: &'a mut Item,
}

/// Evaluate every item of every channel and return the selected ones in feed order.
///
/// A rule whose regular expression does not compile is reported on stderr and matches
/// nothing; the channel's other rules are still evaluated.
pub fn collect_items<'a>(
    channels: impl Iterator<Item = &'a mut (Channel, ChannelConfig)>,
) -> Vec<SelectedItem<'a>> {
    let mut items = Vec::new();

    for (channel, channel_config) in channels {
        let evaluator = ChannelEvaluator::new((&*channel_config).into());

        for err in evaluator.rule_errors() {
            eprintln!(
                "Invalid regex in rule #{} ({}) of {}: {}",
                err.rule + 1,
                channel_config.rules[err.rule].r#match,
                channel.link(),
                err.source
            );
        }

        for item in channel.items_mut() {
            let title = item.title().unwrap_or_default();

            let Outcome::Selected {
                rule,
                save_path,
                episode,
            } = evaluator.evaluate(title).outcome
            else {
                continue;
            };

            println!("Matched {}", channel_config.rules[rule].r#match);

            items.push(SelectedItem {
                save_path,
                episode,
                item,
            });
        }
    }

    items
}
