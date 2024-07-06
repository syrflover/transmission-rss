use rss::Channel;
use transmission_rpc::{
    types::{
        Id, Torrent, TorrentAction, TorrentAddArgs, TorrentAddedOrDuplicate, TorrentGetField,
        TorrentStatus,
    },
    TransClient,
};
use transmission_rss::config::{ChannelConfig, Config};

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
            ]),
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
) -> transmission_rpc::types::Result<TorrentAddedOrDuplicate> {
    let mut res = transmission
        .torrent_add(TorrentAddArgs {
            filename: Some(link.to_owned()),
            labels: Some(vec![BOT_LABEL.to_owned()]),
            ..Default::default()
        })
        .await?;

    match &mut res.arguments {
        TorrentAddedOrDuplicate::TorrentDuplicate(torrent) => {
            *torrent = get_torrent(transmission, torrent.hash_string.as_deref().unwrap())
                .await?
                .unwrap();
        }
        TorrentAddedOrDuplicate::TorrentAdded(_torrent) => {}
    }

    Ok(res.arguments)
}

#[tokio::test]
#[ignore]
async fn test_add_torrent() {
    let link = "magnet:?xt=urn:btih:HTMN5OOCMXUKUQEG3XIXAQYYXGMRMPGG&dn=%5BSubsPlease%5D%20Oshi%20no%20Ko%20-%2012%20%281080p%29%20%5BEC3811BA%5D.mkv&xl=1033669775&tr=http%3A%2F%2Fnyaa.tracker.wf%3A7777%2Fannounce&tr=udp%3A%2F%2Ftracker.coppersurfer.tk%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce&tr=udp%3A%2F%2F9.rarbg.to%3A2710%2Fannounce&tr=udp%3A%2F%2F9.rarbg.me%3A2710%2Fannounce&tr=udp%3A%2F%2Ftracker.leechers-paradise.org%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.internetwarriors.net%3A1337%2Fannounce&tr=udp%3A%2F%2Ftracker.cyberia.is%3A6969%2Fannounce&tr=udp%3A%2F%2Fexodus.desync.com%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker3.itzmx.com%3A6961%2Fannounce&tr=udp%3A%2F%2Ftracker.torrent.eu.org%3A451%2Fannounce&tr=udp%3A%2F%2Ftracker.tiny-vps.com%3A6969%2Fannounce&tr=udp%3A%2F%2Fretracker.lanta-net.ru%3A2710%2Fannounce&tr=http%3A%2F%2Fopen.acgnxtracker.com%3A80%2Fannounce&tr=wss%3A%2F%2Ftracker.openwebtorrent.com";

    let mut transmission = TransClient::new(
        "http://192.168.1.21:32091/transmission/rpc"
            .parse()
            .expect("can't parse transmission url"),
    );

    let res = add_torrent(&mut transmission, link).await.unwrap();

    println!("{res:#?}");
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

    let mut transmission = TransClient::new(
        config
            .transmission_url
            .parse()
            .expect("can't parse transmission url"),
    );

    let mut channels = Vec::new();

    for channel_config in &channels_config {
        let mut channel = match parse_channel(channel_config).await {
            Ok(r) => r,
            Err(err) => {
                eprintln!("{err}");
                continue;
            }
        };

        println!("Parsed {}", channel.link());

        for item in channel.items_mut() {
            let matched = channel_config
                .rules
                .iter()
                .any(|rule| rule.test(item.title().unwrap_or_default()));

            if !matched {
                continue;
            }

            println!();
            println!("Matched {}", item.title().unwrap_or_default(),);

            let link = item.link().unwrap_or_default();

            let torrent = match add_torrent(&mut transmission, link).await {
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
                        println!(
                            "Added {} | {}",
                            torrent.name.as_deref().unwrap(),
                            torrent.hash_string.as_deref().unwrap()
                        );

                        torrent
                    }
                },
                Err(err) => {
                    eprintln!("{err}");
                    continue;
                }
            };

            // set torrent hash
            item.set_description(torrent.hash_string.unwrap());
        }

        channels.push(channel);
    }

    // remove oldest torrents
    match get_torrents(&mut transmission).await {
        Ok(torrents) => {
            let items = channels
                .into_iter()
                .flat_map(|channel| channel.items)
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
    println!("Hello, world!");

    dotenv::dotenv().ok();

    run().await;

    // TODO: graceful shutdown
}
