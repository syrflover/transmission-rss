use std::{path::PathBuf, time::Duration};

use futures::{stream, StreamExt};
use rss::{Channel, Item};
use tokio::time::sleep;
use transmission_rpc::{
    types::{
        Id, SessionSetArgs, Torrent, TorrentAction, TorrentAddArgs, TorrentAddedOrDuplicate,
        TorrentGetField, TorrentStatus,
    },
    TransClient,
};
use transmission_rss::{
    config::{ChannelConfig, Config},
    rule::Rule,
};
use trname::trname;
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum ChannelParseError {
    #[error("reqwest: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("rss: {0}")]
    Rss(#[from] rss::Error),
}

async fn parse_channel(channel_config: &ChannelConfig) -> Result<Channel, ChannelParseError> {
    let buf = reqwest::get(&channel_config.url).await?.bytes().await?;
    let channel = rss::Channel::read_from(&buf[..])?;

    Ok(channel)
}

// fn parse_hash(magnet: &str) -> Option<&str> {
//     if magnet.starts_with("magnet:?xt=urn:btih:") {
//         Some(
//             magnet["magnet:?xt=urn:btih:".len()..]
//                 .splitn(2, '&')
//                 .next()
//                 .unwrap(),
//         )
//     } else {
//         None
//     }
// }

// #[test]
// fn test_parse_hash() {
//     let magnet = "magnet:?xt=urn:btih:3H7C7X5AMCRENTMG23FFM3O4EABRMRH5&dn=%5BSubsPlease%5D%20Tensei%20Shitara%20Slime%20Datta%20Ken%20-%2062%20%281080p%29%20%5B0214B01E%5D.mkv&xl=1497945704&tr=http%3A%2F%2Fnyaa.tracker.wf%3A7777%2Fannounce&tr=udp%3A%2F%2Ftracker.coppersurfer.tk%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce&tr=udp%3A%2F%2F9.rarbg.to%3A2710%2Fannounce&tr=udp%3A%2F%2F9.rarbg.me%3A2710%2Fannounce&tr=udp%3A%2F%2Ftracker.leechers-paradise.org%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.internetwarriors.net%3A1337%2Fannounce&tr=udp%3A%2F%2Ftracker.cyberia.is%3A6969%2Fannounce&tr=udp%3A%2F%2Fexodus.desync.com%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker3.itzmx.com%3A6961%2Fannounce&tr=udp%3A%2F%2Ftracker.torrent.eu.org%3A451%2Fannounce&tr=udp%3A%2F%2Ftracker.tiny-vps.com%3A6969%2Fannounce&tr=udp%3A%2F%2Fretracker.lanta-net.ru%3A2710%2Fannounce&tr=http%3A%2F%2Fopen.acgnxtracker.com%3A80%2Fannounce&tr=wss%3A%2F%2Ftracker.openwebtorrent.com";

//     assert_eq!(
//         "3H7C7X5AMCRENTMG23FFM3O4EABRMRH5",
//         parse_hash(magnet).unwrap()
//     );

//     let magnet = "magnet:?xt=urn:btih:3H7C7X5AMCRENTMG23FFM3O4EABRMRH5&";

//     assert_eq!(
//         "3H7C7X5AMCRENTMG23FFM3O4EABRMRH5",
//         parse_hash(magnet).unwrap()
//     );

//     let magnet = "magnet:?xt=urn:btih:3H7C7X5AMCRENTMG23FFM3O4EABRMRH5";

//     assert_eq!(
//         "3H7C7X5AMCRENTMG23FFM3O4EABRMRH5",
//         parse_hash(magnet).unwrap()
//     );

//     let anything = "https://google.com";

//     assert!(parse_hash(anything).is_none());
// }

async fn get_torrents(
    transmission: &mut TransClient,
) -> transmission_rpc::types::Result<Vec<Torrent>> {
    let res = transmission
        .torrent_get(
            Some(vec![
                TorrentGetField::Id,
                TorrentGetField::Name,
                TorrentGetField::HashString,
                TorrentGetField::Labels,
            ]),
            None,
        )
        .await?;

    Ok(res.arguments.torrents)
}

async fn get_torrent(
    transmission: &mut TransClient,
    hash: &str,
) -> transmission_rpc::types::Result<Option<Torrent>> {
    let res = transmission
        .torrent_get(
            Some(vec![
                TorrentGetField::Id,
                TorrentGetField::Name,
                TorrentGetField::HashString,
                TorrentGetField::Status,
                TorrentGetField::Labels,
                TorrentGetField::FileCount,
            ]),
            Some(vec![Id::Hash(hash.to_owned())]),
        )
        .await?;

    Ok(res.arguments.torrents.into_iter().next())
}

#[tokio::test]
async fn test_get_torrent() {
    let mut transmission = TransClient::new(
        "http://192.168.1.21:32091/transmission/rpc"
            .parse()
            .expect("can't parse transmission url"),
    );

    let res = transmission
        .torrent_get(
            Some(vec![
                TorrentGetField::Id,
                TorrentGetField::Name,
                TorrentGetField::HashString,
                TorrentGetField::Status,
                TorrentGetField::Labels,
                TorrentGetField::Error,
                TorrentGetField::ErrorString,
                TorrentGetField::DoneDate,
                TorrentGetField::FileStats,
                TorrentGetField::FileCount,
                TorrentGetField::Files,
            ]),
            // Some(vec![Id::Hash(
            //     "457be58a312d7a3881783b014cbf766e370c0598".to_owned(),
            // )]),
            None,
        )
        .await
        .unwrap();

    println!("{res:#?}");
}

const BOT_LABEL: &str = "managed:transmission-rss";

fn has_label(labels: Option<&[String]>, x: &str) -> bool {
    labels.is_some_and(|labels| labels.iter().any(|label| label == x))
}

async fn add_torrent(
    transmission: &mut TransClient,
    link: &str,
    download_dir: &PathBuf,
) -> transmission_rpc::types::Result<TorrentAddedOrDuplicate> {
    let mut res = transmission
        .torrent_add(TorrentAddArgs {
            filename: Some(link.to_owned()),
            labels: Some(vec![BOT_LABEL.to_owned()]),
            paused: Some(false),
            download_dir: download_dir.to_str().map(|x| x.to_owned()),
            ..Default::default()
        })
        .await?;

    match &mut res.arguments {
        TorrentAddedOrDuplicate::TorrentDuplicate(torrent) => {
            *torrent = get_torrent(transmission, torrent.hash_string.as_deref().unwrap())
                .await?
                .unwrap();
        }
        TorrentAddedOrDuplicate::TorrentAdded(torrent) => {
            *torrent = get_torrent(transmission, torrent.hash_string.as_deref().unwrap())
                .await?
                .unwrap();
        }
        TorrentAddedOrDuplicate::Error => {
            eprintln!("{}", res.result);
        }
    }

    Ok(res.arguments)
}

#[tokio::test]
#[ignore]
async fn test_add_torrent() {
    let link = "magnet:?xt=urn:btih:SEFD6A5N67C2CJ3NDJI74AP6CZXTCFSZ&dn=%5BSubsPlease%5D%20Katsute%20Mahou%20Shoujo%20to%20Aku%20wa%20Tekitai%20shiteita%20-%2002%20%281080p%29%20%5BC2A5EFC3%5D.mkv&xl=767183596&tr=http%3A%2F%2Fnyaa.tracker.wf%3A7777%2Fannounce&tr=udp%3A%2F%2Ftracker.coppersurfer.tk%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce&tr=udp%3A%2F%2F9.rarbg.to%3A2710%2Fannounce&tr=udp%3A%2F%2F9.rarbg.me%3A2710%2Fannounce&tr=udp%3A%2F%2Ftracker.leechers-paradise.org%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.internetwarriors.net%3A1337%2Fannounce&tr=udp%3A%2F%2Ftracker.cyberia.is%3A6969%2Fannounce&tr=udp%3A%2F%2Fexodus.desync.com%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker3.itzmx.com%3A6961%2Fannounce&tr=udp%3A%2F%2Ftracker.torrent.eu.org%3A451%2Fannounce&tr=udp%3A%2F%2Ftracker.tiny-vps.com%3A6969%2Fannounce&tr=udp%3A%2F%2Fretracker.lanta-net.ru%3A2710%2Fannounce&tr=http%3A%2F%2Fopen.acgnxtracker.com%3A80%2Fannounce&tr=wss%3A%2F%2Ftracker.openwebtorrent.com";

    let mut transmission = TransClient::new(
        "http://192.168.1.21:32091/transmission/rpc"
            .parse()
            .expect("can't parse transmission url"),
    );

    let res = add_torrent(
        &mut transmission,
        link,
        &PathBuf::from(
            "/downloads/Shows (current)/Katsute Mahou Shoujo to Aku wa Tekitai shiteita/Season 01",
        ),
    )
    .await
    .unwrap();

    println!("{res:#?}");
}

async fn rename_torrent(
    transmission: &mut TransClient,
    hash: &str,
    download_dir: &PathBuf,
    starts_episode_at: isize,
) -> transmission_rpc::types::Result<Option<String>> {
    let Some(torrent) = get_torrent(transmission, hash).await? else {
        return Ok(None);
    };

    if torrent.file_count.unwrap() == 1 {
        let old_file_name = torrent.name.clone().unwrap();

        if let Some(new_file_name) = trname(&download_dir, &old_file_name, starts_episode_at) {
            let res = transmission
                .torrent_rename_path(
                    vec![Id::Hash(hash.to_owned())],
                    old_file_name,
                    new_file_name.clone(),
                )
                .await?;

            if res.result == "success" {
                return Ok(Some(new_file_name));
            }
        }
    }

    Ok(None)
}

async fn run() {
    let config = Config::new();
    let channels_config: Vec<ChannelConfig> = serde_yaml::from_slice(
        &reqwest::get(&config.channels_config_url)
            .await
            .expect("can't get channels configuration")
            .bytes()
            .await
            .unwrap(),
    )
    .expect("can't deserialize channels configuration");

    let transmission_url = config
        .transmission_url
        .parse::<Url>()
        .expect("can't parse transmission url");

    let mut transmission = TransClient::new(transmission_url.clone());

    let transmission_config = SessionSetArgs {
        download_dir: config.download_dir,
        speed_limit_up_enabled: config.speed_limit_up.is_some().then_some(true),
        speed_limit_up: config.speed_limit_up,
        speed_limit_down_enabled: config.speed_limit_down.is_some().then_some(true),
        speed_limit_down: config.speed_limit_down,
        download_queue_enabled: config.download_queue_size.is_some().then_some(true),
        download_queue_size: config.download_queue_size,
        seed_queue_enabled: config.seed_queue_size.is_some().then_some(true),
        seed_queue_size: config.seed_queue_size,
        ..Default::default()
    };

    println!("{:#?}", transmission_config);

    transmission
        .session_set(transmission_config)
        .await
        .expect("can't set transmission configuration");

    pub fn collect_items<'a>(
        channels: impl Iterator<Item = &'a mut (Channel, ChannelConfig)>,
    ) -> Vec<(PathBuf, &'a Rule, &'a mut Item)> {
        let mut items = Vec::new();

        for (channel, channel_config) in channels {
            for item in channel.items_mut() {
                let matched = channel_config
                    .rules
                    .iter()
                    .find(|rule| rule.test(item.title().unwrap_or_default()));

                let Some(matched) = matched else {
                    continue;
                };

                items.push((channel_config.directory.clone(), matched, item));

                println!("Matched {}", matched.r#match);

                // return items;
            }
        }

        return items;
    }

    let mut channels = stream::iter(channels_config.into_iter())
        .map(|channel_config| async {
            (
                parse_channel(&channel_config)
                    .await
                    .inspect(|channel| println!("Parsed {}", channel.link())),
                channel_config,
            )
        })
        .buffered(5)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .filter_map(|(res, channel_config)| {
            res.inspect_err(|err| println!("{err}"))
                .ok()
                .map(|channel| (channel, channel_config))
        })
        .collect::<Vec<_>>();

    println!();

    let matched_items = collect_items(channels.iter_mut());

    println!();

    stream::iter(matched_items.into_iter())
        .for_each_concurrent(100, |(base_directory, matched, item)| {
            let transmission_url = transmission_url.clone();

            async move {
                let mut transmission = TransClient::new(transmission_url);

                let link = item.link().unwrap_or_default();

                let torrent =
                    match add_torrent(&mut transmission, link, &matched.directory(&base_directory))
                        .await
                    {
                        Ok(r) => match r {
                            TorrentAddedOrDuplicate::TorrentDuplicate(torrent) => {
                                let hash = torrent.hash_string.as_deref().unwrap();

                                match torrent.status.unwrap() {
                                    TorrentStatus::QueuedToSeed | TorrentStatus::Seeding
                                        if has_label(torrent.labels.as_deref(), BOT_LABEL) =>
                                    {
                                        // pause_torrent
                                        transmission
                                            .torrent_action(
                                                TorrentAction::Stop,
                                                vec![Id::Hash(hash.to_owned())],
                                            )
                                            .await
                                            .inspect_err(|err| eprintln!("{err}"))
                                            .ok(); // FIXME: error handle

                                        println!(
                                            "Stopped {} | {}",
                                            torrent.name.as_deref().unwrap(),
                                            torrent.hash_string.as_deref().unwrap()
                                        );
                                    }
                                    _ => {
                                        println!(
                                            "Already {} | {}",
                                            torrent.name.as_deref().unwrap(),
                                            torrent.hash_string.as_deref().unwrap()
                                        );
                                    }
                                }

                                torrent
                            }
                            TorrentAddedOrDuplicate::TorrentAdded(torrent) => {
                                let hash = torrent.hash_string.as_deref().unwrap();
                                let name = torrent.name.as_deref().unwrap();

                                println!("Added {} | {}", name, hash);

                                torrent
                            }
                            TorrentAddedOrDuplicate::Error => {
                                return;
                            }
                        },
                        Err(err) => {
                            eprintln!("{err}");
                            return;
                        }
                    };

                let hash = torrent.hash_string.unwrap();

                // rename
                {
                    let mut i = 0;

                    loop {
                        sleep(Duration::from_secs(1)).await;

                        let res = rename_torrent(
                            &mut transmission,
                            &hash,
                            &matched.directory(&base_directory),
                            matched.starts_episode_at,
                        )
                        .await
                        .inspect_err(|err| println!("{err}"));

                        match res {
                            Ok(Some(_name)) => break,
                            _ => {
                                if i > 15 {
                                    break;
                                }
                                i += 1;
                            }
                        };
                    }
                };

                // set torrent hash
                item.set_description(hash);
            }
        })
        .await;

    match get_torrents(&mut transmission).await {
        Ok(torrents) => {
            // remove oldest torrents
            let items = channels
                .into_iter()
                .flat_map(|(channel, _)| channel.items)
                .collect::<Vec<_>>();

            let oldest_torrents = torrents
                .into_iter()
                .filter(|torrent| has_label(torrent.labels.as_deref(), BOT_LABEL))
                .filter(|torrent| {
                    !items.iter().any(|item| {
                        item.description()
                            .is_some_and(|desc| desc == torrent.hash_string.as_deref().unwrap())
                    })
                })
                .collect::<Vec<_>>();

            if !oldest_torrents.is_empty() {
                transmission
                    .torrent_remove(
                        oldest_torrents
                            .clone()
                            .into_iter()
                            .map(|torrent| Id::Hash(torrent.hash_string.unwrap()))
                            .collect(),
                        false,
                    )
                    .await
                    .inspect_err(|err| eprintln!("{err}"))
                    .ok();

                println!();

                for oldest_torrent in oldest_torrents {
                    println!(
                        "Removed {} | {}",
                        oldest_torrent.name.unwrap(),
                        oldest_torrent.hash_string.unwrap()
                    );
                }
            }
        }
        Err(err) => {
            eprintln!("{err}");
        }
    }
}

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();

    run().await;

    // TODO: graceful shutdown
}
