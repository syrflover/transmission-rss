//! Compares the shared evaluation, as used by the `transmission-rss` binary, with the
//! selection logic the binary had before it moved (`reference_collect`, copied from the old
//! `collect_items` and `Rule::test`) on a sample legacy YAML and a sample RSS feed.

use std::path::PathBuf;

use rss::Channel;
use transmission_rss::{config::ChannelConfig, rss::legacy};

const CHANNELS_YAML: &str = include_str!("fixtures/legacy_channels.yml");
const FEED_XML: &str = include_str!("fixtures/sample_feed.xml");

/// What the binary needs per selected item: link, save path, rename start episode.
type Selection = (String, PathBuf, isize);

fn load() -> Vec<(Channel, ChannelConfig)> {
    let configs: Vec<ChannelConfig> =
        yaml_serde::from_str(CHANNELS_YAML).expect("fixture yaml parses");
    let feed = Channel::read_from(FEED_XML.as_bytes()).expect("fixture feed parses");

    configs
        .into_iter()
        .map(|config| (feed.clone(), config))
        .collect()
}

/// The pre-change logic: channel excludes, then the first rule whose (non-regex) test passes,
/// then `channel.directory.join(rule.directory)`.
fn reference_collect(channels: &[(Channel, ChannelConfig)]) -> Vec<Selection> {
    let mut selected = Vec::new();

    for (channel, config) in channels {
        for item in channel.items() {
            let title = item.title().unwrap_or_default();

            if config.excludes.iter().any(|ex| title.contains(ex.as_str())) {
                continue;
            }

            let matched = config.rules.iter().find(|rule| {
                assert!(!rule.regex, "the old logic panicked on regex rules");
                if rule.case_insensitive {
                    title.to_lowercase().contains(&rule.r#match.to_lowercase())
                } else {
                    title.contains(&rule.r#match)
                }
            });

            let Some(rule) = matched else {
                continue;
            };

            selected.push((
                item.link().unwrap_or_default().to_owned(),
                rule.directory(&config.directory),
                rule.starts_episode_at,
            ));
        }
    }

    selected
}

fn new_collect(channels: &mut [(Channel, ChannelConfig)]) -> Vec<Selection> {
    legacy::collect_items(channels.iter_mut())
        .into_iter()
        .map(|selected| {
            (
                selected.item.link().unwrap_or_default().to_owned(),
                selected.save_path,
                selected.episode,
            )
        })
        .collect()
}

fn link(n: u32) -> String {
    format!("magnet:?xt=urn:btih:AAAA00000000000000000000000000{n:02}")
}

#[test]
fn new_module_selects_the_same_items_rules_and_paths_as_the_old_logic() {
    let mut channels = load();

    let old = reference_collect(&channels);
    let new = new_collect(&mut channels);

    assert_eq!(new, old);
}

#[test]
fn sample_selection_is_the_expected_non_trivial_set() {
    // Guards the comparison above against passing on an empty or degenerate result.
    let mut channels = load();

    let expected: Vec<Selection> = vec![
        // channel 1: /media/anime (excludes "[Batch]" and "(720p)")
        (link(1), "/media/anime/Sayonara Lara/Season 01".into(), 1),
        // case-insensitive rule wins over the later, overlapping case-sensitive one
        (
            link(3),
            "/media/anime/Sono Bisque Doll/Season 02".into(),
            -12,
        ),
        (
            link(4),
            "/media/anime/Sono Bisque Doll/Season 02".into(),
            -12,
        ),
        (link(6), "/media/anime/Slime/Season 04".into(), -24),
        // link(8) differs only by case from a case-sensitive rule, link(7) and link(2) are
        // excluded by "(720p)", link(5) by "[Batch]", link(9)/(10) match no rule
        // channel 2: /media/other (no excludes)
        (link(1), "/media/other/Elsewhere/Sayonara Lara".into(), 1),
        (link(2), "/media/other/Elsewhere/Sayonara Lara".into(), 1),
    ];

    assert_eq!(new_collect(&mut channels), expected);
    assert_eq!(reference_collect(&channels), expected);
}
