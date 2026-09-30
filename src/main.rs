use futures::{stream, StreamExt};
use reqwest::header;
use rss::Channel;
use tokio_util::sync::CancellationToken;
use transmission_rpc::TransClient;
use transmission_rss::{
    config::{ChannelConfig, Config},
    rss::legacy::{collect_items, SelectedItem},
    transmission::{
        add_item, remove_stale, rename_with_retries, AddError, AddLabels, Redactor, RenameMode,
        RenamePolicy, SessionConfig,
    },
};
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum ChannelParseError {
    #[error("reqwest: {0}")]
    Reqwest(#[from] reqwest::Error),
    #[error("rss: {0}")]
    Rss(#[from] rss::Error),
}

async fn parse_channel(channel_config: &ChannelConfig) -> Result<Channel, ChannelParseError> {
    let buf = reqwest::Client::new()
        .get(&channel_config.url)
        .header(header::USER_AGENT, transmission_rss::USER_AGENT)
        .send()
        .await?
        .bytes()
        .await?;
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

#[tokio::test]
#[ignore = "needs a Transmission instance at a hardcoded LAN address"]
async fn test_get_torrent() {
    use transmission_rpc::types::TorrentGetField;

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

#[tokio::test]
#[ignore = "needs a Transmission instance at a hardcoded LAN address and adds a torrent"]
async fn test_add_torrent() {
    let link = "magnet:?xt=urn:btih:SEFD6A5N67C2CJ3NDJI74AP6CZXTCFSZ&dn=%5BSubsPlease%5D%20Katsute%20Mahou%20Shoujo%20to%20Aku%20wa%20Tekitai%20shiteita%20-%2002%20%281080p%29%20%5BC2A5EFC3%5D.mkv&xl=767183596&tr=http%3A%2F%2Fnyaa.tracker.wf%3A7777%2Fannounce&tr=udp%3A%2F%2Ftracker.coppersurfer.tk%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.opentrackr.org%3A1337%2Fannounce&tr=udp%3A%2F%2F9.rarbg.to%3A2710%2Fannounce&tr=udp%3A%2F%2F9.rarbg.me%3A2710%2Fannounce&tr=udp%3A%2F%2Ftracker.leechers-paradise.org%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker.internetwarriors.net%3A1337%2Fannounce&tr=udp%3A%2F%2Ftracker.cyberia.is%3A6969%2Fannounce&tr=udp%3A%2F%2Fexodus.desync.com%3A6969%2Fannounce&tr=udp%3A%2F%2Ftracker3.itzmx.com%3A6961%2Fannounce&tr=udp%3A%2F%2Ftracker.torrent.eu.org%3A451%2Fannounce&tr=udp%3A%2F%2Ftracker.tiny-vps.com%3A6969%2Fannounce&tr=udp%3A%2F%2Fretracker.lanta-net.ru%3A2710%2Fannounce&tr=http%3A%2F%2Fopen.acgnxtracker.com%3A80%2Fannounce&tr=wss%3A%2F%2Ftracker.openwebtorrent.com";

    let mut transmission = TransClient::new(
        "http://192.168.1.21:32091/transmission/rpc"
            .parse()
            .expect("can't parse transmission url"),
    );

    let res = add_item(
        &mut transmission,
        link,
        std::path::Path::new(
            "/downloads/Shows (current)/Katsute Mahou Shoujo to Aku wa Tekitai shiteita/Season 01",
        ),
        AddLabels::default(),
        &Redactor::none(),
    )
    .await
    .unwrap();

    println!("{res:#?}");
}

async fn run() {
    let config = Config::new();
    let channels_config: Vec<ChannelConfig> = yaml_serde::from_slice(
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

    let transmission_config = SessionConfig {
        download_dir: config.download_dir,
        speed_limit_up: config.speed_limit_up,
        speed_limit_down: config.speed_limit_down,
        download_queue_size: config.download_queue_size,
        seed_queue_size: config.seed_queue_size,
    }
    .to_args();

    println!("{:#?}", transmission_config);

    transmission
        .session_set(transmission_config)
        .await
        .expect("can't set transmission configuration");

    let mut channels = stream::iter(channels_config)
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

    // The binary logs errors as they are; nothing here needs redacting.
    let redactor = Redactor::none();
    // Nothing cancels the binary's run.
    let cancel = CancellationToken::new();

    stream::iter(matched_items)
        .for_each_concurrent(100, |selected| {
            let transmission_url = transmission_url.clone();
            let redactor = &redactor;
            let cancel = &cancel;

            async move {
                let SelectedItem {
                    save_path,
                    episode,
                    item,
                } = selected;

                let mut transmission = TransClient::new(transmission_url);

                let link = item.link().unwrap_or_default();

                let torrent = match add_item(
                    &mut transmission,
                    link,
                    &save_path,
                    AddLabels::default(),
                    redactor,
                )
                .await
                {
                    Ok(torrent) => torrent,
                    Err(AddError::Rejected(result)) => {
                        eprintln!("{result}");
                        return;
                    }
                    Err(AddError::Unreachable(err) | AddError::Rpc(err)) => {
                        eprintln!("{err}");
                        return;
                    }
                };

                let hash = torrent.hash;

                // rename
                rename_with_retries(
                    &mut transmission,
                    &hash,
                    &save_path,
                    episode,
                    RenameMode::Added,
                    RenamePolicy::default(),
                    redactor,
                    cancel,
                )
                .await;

                // set torrent hash
                item.set_description(hash);
            }
        })
        .await;

    let items = channels
        .into_iter()
        .flat_map(|(channel, _)| channel.items)
        .collect::<Vec<_>>();

    remove_stale(
        &mut transmission,
        |hash, _| {
            items
                .iter()
                .any(|item| item.description().is_some_and(|desc| desc == hash))
        },
        &redactor,
    )
    .await;
}

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();

    run().await;

    // TODO: graceful shutdown
}
