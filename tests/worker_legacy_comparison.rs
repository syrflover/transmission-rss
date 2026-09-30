//! The worker must do what the legacy `transmission-rss` binary does for the
//! same channels, rules and RSS sample. The real legacy binary is run against
//! one fake Transmission; the worker, with the same channels and rules stored
//! in its database, against another. Their Transmission requests are compared.

mod common;

use std::process::Command;

use common::*;
use tokio_util::sync::CancellationToken;
use transmission_rss::{
    config::ChannelConfig,
    store::channels::{ChannelInput, RuleInput},
    worker::TickOutcome,
};

const SESSION_ENV: [(&str, &str); 5] = [
    ("DOWNLOAD_DIR", "/downloads"),
    ("SPEED_LIMIT_UP", "100"),
    ("SPEED_LIMIT_DOWN", "2000"),
    ("DOWNLOAD_QUEUE_SIZE", "3"),
    ("SEED_QUEUE_SIZE", "4"),
];

const FINISHED_HASH: &str = "aaaa000000000000000000000000000000000001";
const GONE_HASH: &str = "gone0000000000000000000000000000000000aa";
const MANUAL_HASH: &str = "mine0000000000000000000000000000000000bb";

/// Torrents already in Transmission: a finished bot torrent that is also in
/// the feed (to be stopped), a bot torrent no longer in any feed (to be
/// removed), and one the bot never added (to be left alone).
fn preload(tr: &FakeTransmission) {
    tr.preload(
        FakeTorrent::new(
            FINISHED_HASH,
            "[SubsPlease] Sayonara Lara - 03 (1080p) [AAAA0001].mkv",
        )
        .bot()
        .status(6),
    );
    tr.preload(FakeTorrent::new(GONE_HASH, "Old Show - 12.mkv").bot());
    tr.preload(FakeTorrent::new(MANUAL_HASH, "Manual Download.mkv"));
}

fn state(tr: &FakeTransmission) -> Vec<(String, String, Vec<String>, u8)> {
    let mut torrents: Vec<_> = tr
        .torrents()
        .into_iter()
        .map(|t| (t.hash, t.name, t.labels, t.status))
        .collect();
    torrents.sort();
    torrents
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_worker_talks_to_transmission_like_the_legacy_binary() {
    let h = Harness::new().await;
    let yaml = CHANNELS_YAML
        .replace("{FEED_A}", &h.feeds.url("feed-a"))
        .replace("{FEED_B}", &h.feeds.url("feed-b"));
    h.feeds.set_xml("channels.yml", &yaml);

    // --- the legacy binary, configured by the YAML and the environment ---
    let legacy_tr = FakeTransmission::start().await;
    preload(&legacy_tr);
    let mut legacy = Command::new(env!("CARGO_BIN_EXE_transmission-rss"));
    legacy
        .current_dir(h.dir.path())
        .env("CHANNELS_CONFIG_URL", h.feeds.url("channels.yml"))
        .env("TRANSMISSION_URL", legacy_tr.url());
    for (key, value) in SESSION_ENV {
        legacy.env(key, value);
    }
    let output = tokio::task::spawn_blocking(move || legacy.output())
        .await
        .unwrap()
        .expect("run transmission-rss");
    assert!(
        output.status.success(),
        "legacy binary failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // --- the worker, configured by the same channels and rules in its database ---
    let configs: Vec<ChannelConfig> = yaml_serde::from_str(&yaml).expect("fixture yaml");
    for config in &configs {
        let mut input = ChannelInput::new(&config.url, config.directory.to_str().unwrap());
        input.excludes = config.excludes.clone();
        let rules = config
            .rules
            .iter()
            .map(|rule| RuleInput {
                r#match: Some(rule.r#match.clone()),
                regex: rule.regex,
                case_insensitive: rule.case_insensitive,
                directory: rule.directory("").to_str().unwrap().to_owned(),
                episode: rule.starts_episode_at as i64,
                ..Default::default()
            })
            .collect();
        h.channels
            .create_channel_with_rules(input, rules)
            .await
            .unwrap();
    }
    preload(&h.tr);
    let env = h.worker_env_with(&SESSION_ENV);
    let worker = h.worker_with(h.db.clone(), &env);
    let TickOutcome::Ran(report) = worker.tick(&CancellationToken::new()).await.unwrap() else {
        panic!("the worker did not run a cycle");
    };

    // --- the same requests, and the same Transmission afterwards ---
    let legacy_calls = legacy_tr.mutations();
    let worker_calls = h.tr.mutations();
    assert_eq!(
        worker_calls,
        legacy_calls,
        "Transmission requests differ.\nlegacy:\n{}\nworker:\n{}",
        legacy_calls.join("\n"),
        worker_calls.join("\n")
    );
    assert_eq!(state(&h.tr), state(&legacy_tr));

    // The comparison is not vacuous.
    let count = |prefix: &str| {
        worker_calls
            .iter()
            .filter(|c| c.starts_with(prefix))
            .count()
    };
    assert_eq!(count("session-set"), 1);
    assert_eq!(count("torrent-add"), 4, "{worker_calls:#?}");
    assert_eq!(count("torrent-rename-path"), 4);
    assert_eq!(count("torrent-stop"), 1);
    assert_eq!(count("torrent-remove"), 1);
    assert!(worker_calls
        .iter()
        .any(|c| c.contains("/media/anime/Sono Bisque Doll/Season 02")));
    assert!(worker_calls
        .iter()
        .any(|c| c.contains("/media/other/Elsewhere/Sayonara Lara/Season 01")));
    assert!(worker_calls
        .iter()
        .any(|c| c.contains(GONE_HASH) && c.contains("delete-local-data=false")));
    assert!(worker_calls
        .iter()
        .any(|c| c.starts_with("session-set") && c.contains("\"speed-limit-up\":100")));
    assert_eq!((report.added, report.duplicates), (3, 1));
}
