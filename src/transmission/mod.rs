//! The Transmission side of collection: adding an item's torrent, handling one
//! that is already present, renaming the single file with `trname`, and
//! removing bot-labelled torrents that have left the feed.
//!
//! The logic here moved out of `src/main.rs` unchanged so that the legacy
//! `transmission-rss` binary and `trss-worker` behave the same way. Where the
//! binary printed an error, these functions print it through a [`Redactor`]
//! (the binary passes [`Redactor::none`]), so the worker never logs a
//! secret that an HTTP error quotes.

mod redact;

use std::{fmt, path::Path, time::Duration};

pub use redact::{Redactor, MIN_QUERY_SECRET_LEN, REDACTED};
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use transmission_rpc::{
    types::{
        Id, SessionSetArgs, Torrent, TorrentAction, TorrentAddArgs, TorrentAddedOrDuplicate,
        TorrentGetField, TorrentStatus,
    },
    TransClient,
};
use trname::trname;

/// How long connecting to Transmission may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long one request to Transmission may take in total, from sending it to
/// reading the whole answer. `transmission-rpc` builds its client without any
/// timeout, so one hung request would otherwise wait forever, holding the
/// worker's lock. A `torrent-add` of a `.torrent` URL is the slow kind (Transmission
/// fetches the file before it answers); 30 seconds is far above what it takes
/// when the tracker responds. A request that times out fails the item, which the
/// next cycle retries.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// An HTTP client for `transmission-rpc` with the connect timeout and a
/// `request_timeout` for whole requests. (`transmission-rpc` uses reqwest 0.12,
/// hence the separately named crate.)
pub fn http_client(request_timeout: Duration) -> Result<reqwest012::Client, reqwest012::Error> {
    reqwest012::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT.min(request_timeout))
        .timeout(request_timeout)
        .build()
}

/// A client for the Transmission at `url` that uses `http`, so its requests
/// time out (see [`http_client`]).
pub fn client(url: url::Url, http: &reqwest012::Client) -> TransClient {
    TransClient::new_with_client(url, http.clone())
}

/// Label put on every torrent this program adds. Only labelled torrents are
/// ever stopped or removed by the program.
pub const BOT_LABEL: &str = "managed:transmission-rss";

pub fn has_label(labels: Option<&[String]>, x: &str) -> bool {
    labels.is_some_and(|labels| labels.iter().any(|label| label == x))
}

/// Session settings applied to Transmission before each collection run.
/// Unset options are left as they are in Transmission.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionConfig {
    pub download_dir: Option<String>,
    pub speed_limit_up: Option<i32>,
    pub speed_limit_down: Option<i32>,
    pub download_queue_size: Option<i32>,
    pub seed_queue_size: Option<i32>,
}

impl SessionConfig {
    pub fn to_args(&self) -> SessionSetArgs {
        SessionSetArgs {
            download_dir: self.download_dir.clone(),
            speed_limit_up_enabled: self.speed_limit_up.is_some().then_some(true),
            speed_limit_up: self.speed_limit_up,
            speed_limit_down_enabled: self.speed_limit_down.is_some().then_some(true),
            speed_limit_down: self.speed_limit_down,
            download_queue_enabled: self.download_queue_size.is_some().then_some(true),
            download_queue_size: self.download_queue_size,
            seed_queue_enabled: self.seed_queue_size.is_some().then_some(true),
            seed_queue_size: self.seed_queue_size,
            ..Default::default()
        }
    }
}

pub async fn get_torrents(
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

pub async fn get_torrent(
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

/// Why an item could not be added.
#[derive(Debug)]
pub enum AddError {
    /// The request itself failed (connection, protocol, decoding).
    Rpc(Box<dyn std::error::Error + Send + Sync>),
    /// Transmission answered but refused, with its result text.
    Rejected(String),
}

impl fmt::Display for AddError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AddError::Rpc(err) => write!(f, "{err}"),
            AddError::Rejected(result) => write!(f, "{result}"),
        }
    }
}

impl std::error::Error for AddError {}

impl From<Box<dyn std::error::Error + Send + Sync>> for AddError {
    fn from(err: Box<dyn std::error::Error + Send + Sync>) -> Self {
        AddError::Rpc(err)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddKind {
    /// Transmission did not have the torrent and took it.
    Added,
    /// Transmission already had the torrent.
    Duplicate,
}

/// A torrent Transmission holds for an item after [`add_item`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedTorrent {
    pub kind: AddKind,
    pub hash: String,
    pub name: String,
}

async fn add_torrent(
    transmission: &mut TransClient,
    link: &str,
    download_dir: &Path,
) -> Result<TorrentAddedOrDuplicate, AddError> {
    let mut res = transmission
        .torrent_add(TorrentAddArgs {
            filename: Some(link.to_owned()),
            labels: Some(vec![BOT_LABEL.to_owned()]),
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
            return Err(AddError::Rejected(res.result));
        }
    }

    Ok(res.arguments)
}

/// Adds `link` to Transmission, saving to `download_dir`.
///
/// A torrent that is already present is left in place, except that a
/// bot-labelled one that has finished (queued to seed or seeding) is stopped.
/// Prints `Added`, `Stopped` or `Already` with the torrent's name and hash.
pub async fn add_item(
    transmission: &mut TransClient,
    link: &str,
    download_dir: &Path,
    redactor: &Redactor,
) -> Result<AddedTorrent, AddError> {
    match add_torrent(transmission, link, download_dir).await? {
        TorrentAddedOrDuplicate::TorrentDuplicate(torrent) => {
            let hash = torrent.hash_string.as_deref().unwrap();

            match torrent.status.unwrap() {
                TorrentStatus::QueuedToSeed | TorrentStatus::Seeding
                    if has_label(torrent.labels.as_deref(), BOT_LABEL) =>
                {
                    // pause_torrent
                    transmission
                        .torrent_action(TorrentAction::Stop, vec![Id::Hash(hash.to_owned())])
                        .await
                        .inspect_err(|err| eprintln!("{}", redactor.apply(&err.to_string())))
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

            Ok(AddedTorrent {
                kind: AddKind::Duplicate,
                hash: torrent.hash_string.unwrap(),
                name: torrent.name.unwrap(),
            })
        }
        TorrentAddedOrDuplicate::TorrentAdded(torrent) => {
            let hash = torrent.hash_string.as_deref().unwrap();
            let name = torrent.name.as_deref().unwrap();

            println!("Added {} | {}", name, hash);

            Ok(AddedTorrent {
                kind: AddKind::Added,
                hash: torrent.hash_string.unwrap(),
                name: torrent.name.unwrap(),
            })
        }
        // `add_torrent` returns this case as `AddError::Rejected`.
        TorrentAddedOrDuplicate::Error => Err(AddError::Rejected(String::new())),
    }
}

/// Renames the torrent's single file to the `trname` name for `download_dir`
/// (`.../<title>/Season NN`), or removes the torrent and its data when the
/// name cannot be derived. Torrents with more than one file are left alone.
/// Returns the new name when the rename succeeded.
pub async fn rename_torrent(
    transmission: &mut TransClient,
    hash: &str,
    download_dir: &Path,
    starts_episode_at: isize,
) -> transmission_rpc::types::Result<Option<String>> {
    let Some(torrent) = get_torrent(transmission, hash).await? else {
        return Ok(None);
    };

    if torrent.file_count.unwrap() == 1 {
        let old_file_name = torrent.name.clone().unwrap();

        match trname(download_dir, &old_file_name, starts_episode_at) {
            Some(new_file_name) => {
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
            None => {
                let _res = transmission
                    .torrent_remove(vec![Id::Hash(hash.to_owned())], true)
                    .await?;
            }
        }
    }

    Ok(None)
}

/// How persistently a freshly added torrent is renamed: Transmission needs a
/// moment before a magnet link's metadata, and with it the file name, is known.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenamePolicy {
    /// Wait before each attempt.
    pub delay: Duration,
    /// Attempts before giving up.
    pub attempts: u32,
}

impl Default for RenamePolicy {
    /// The binary's behavior: an attempt every second, 17 attempts.
    fn default() -> Self {
        RenamePolicy {
            delay: Duration::from_secs(1),
            attempts: 17,
        }
    }
}

/// Calls [`rename_torrent`] until it succeeds or the attempts run out, printing
/// each error. Stops early, without a further attempt, once `cancel` fires;
/// the next collection run tries again because it meets the torrent again.
pub async fn rename_with_retries(
    transmission: &mut TransClient,
    hash: &str,
    save_path: &Path,
    episode: isize,
    policy: RenamePolicy,
    redactor: &Redactor,
    cancel: &CancellationToken,
) {
    for _ in 0..policy.attempts {
        tokio::select! {
            _ = sleep(policy.delay) => {}
            _ = cancel.cancelled() => break,
        }

        let res = rename_torrent(transmission, hash, save_path, episode)
            .await
            .inspect_err(|err| println!("{}", redactor.apply(&err.to_string())));

        if let Ok(Some(_name)) = res {
            break;
        }
    }
}

/// A torrent taken out of Transmission by [`remove_stale`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovedTorrent {
    pub name: String,
    pub hash: String,
}

/// Removes (keeping the downloaded data) every bot-labelled torrent for which
/// `is_kept(hash)` is false, i.e. torrents whose item is no longer in the
/// feeds. Prints one `Removed` line per torrent. A failure to list or remove
/// is printed and leaves things as they are.
pub async fn remove_stale(
    transmission: &mut TransClient,
    is_kept: impl Fn(&str) -> bool,
    redactor: &Redactor,
) -> Vec<RemovedTorrent> {
    match get_torrents(transmission).await {
        Ok(torrents) => {
            // remove oldest torrents
            let oldest_torrents = torrents
                .into_iter()
                .filter(|torrent| has_label(torrent.labels.as_deref(), BOT_LABEL))
                .filter(|torrent| !is_kept(torrent.hash_string.as_deref().unwrap()))
                .collect::<Vec<_>>();

            let mut removed = Vec::new();

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
                    .inspect_err(|err| eprintln!("{}", redactor.apply(&err.to_string())))
                    .ok();

                println!();

                for oldest_torrent in oldest_torrents {
                    println!(
                        "Removed {} | {}",
                        oldest_torrent.name.as_deref().unwrap(),
                        oldest_torrent.hash_string.as_deref().unwrap()
                    );
                    removed.push(RemovedTorrent {
                        name: oldest_torrent.name.unwrap(),
                        hash: oldest_torrent.hash_string.unwrap(),
                    });
                }
            }

            removed
        }
        Err(err) => {
            eprintln!("{}", redactor.apply(&err.to_string()));
            Vec::new()
        }
    }
}
