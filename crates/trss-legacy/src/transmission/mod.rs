//! The Transmission side of collection: adding an item's torrent, handling one
//! that is already present, renaming the single file with `trname`, and
//! removing bot-labelled torrents that have left the feed.
//!
//! The logic here moved out of the former cron binary unchanged. Where that
//! binary printed an error, these functions print it through a [`Redactor`], so
//! the worker never logs a secret that an HTTP error quotes. Renaming takes a
//! [`RenameMode`]: the former binary renamed every torrent as one it had just
//! added, while the worker does that for its own new adds only.

mod redact;

use std::{fmt, path::Path, sync::LazyLock, time::Duration};

pub use redact::{Redactor, MIN_QUERY_SECRET_LEN, REDACTED};
use regex::Regex;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use transmission_rpc::{
    types::{
        Id, SessionSetArgs, Torrent, TorrentAction, TorrentAddArgs, TorrentAddedOrDuplicate,
        TorrentGetField, TorrentSetArgs, TorrentStatus,
    },
    TransClient,
};
use trname::trname_raw;

use crate::revision::Release;

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

/// Start of the label that says which feed item a torrent is for.
pub const ITEM_LABEL_PREFIX: &str = "trss-item:";

/// The label that says a torrent is for the item with `identity_key` in the
/// channel `channel_id`: `trss-item:<channel ID>:<identity key>`. The worker
/// puts it on the torrents it holds for an item, so the torrent itself tells
/// the cleanup which item it belongs to, whether or not history learned its
/// hash. Neither part carries a secret: the channel ID is a UUID and the
/// identity key a SHA-256 digest.
pub fn item_label(channel_id: &str, identity_key: &str) -> String {
    format!("{ITEM_LABEL_PREFIX}{channel_id}:{identity_key}")
}

/// The `(channel ID, identity key)` an [`item_label`] names.
pub fn item_of_label(label: &str) -> Option<(&str, &str)> {
    label.strip_prefix(ITEM_LABEL_PREFIX)?.split_once(':')
}

/// Start of the label that says which command added a torrent.
pub const COMMAND_LABEL_PREFIX: &str = "trss-cmd:";

/// The label a command's add puts on its torrent: `trss-cmd:<command ID>`.
/// Transmission keeps the labels of the add that put a torrent in and ignores
/// those of an add it answers `duplicate`, and nothing else puts this label on,
/// so a torrent carrying it is the one that command's add put in, even when
/// that add got no answer. The command takes it off once it has recorded the
/// torrent ([`remove_label`]).
pub fn command_label(command_id: &str) -> String {
    format!("{COMMAND_LABEL_PREFIX}{command_id}")
}

/// The labels an add puts on its torrent next to [`BOT_LABEL`].
#[derive(Debug, Clone, Copy, Default)]
pub struct AddLabels<'a> {
    /// An [`item_label`]; also put on a bot torrent the add finds already there.
    pub item: Option<&'a str>,
    /// A [`command_label`]; only ever on a torrent the add puts in.
    pub command: Option<&'a str>,
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
                TorrentGetField::LeftUntilDone,
                TorrentGetField::SizeWhenDone,
            ]),
            None,
        )
        .await?;

    Ok(res.arguments.torrents)
}

/// Whether Transmission has everything of `torrent` it was asked to download.
/// A torrent without metadata yet (size 0) or without the fields is not.
fn is_finished(torrent: &Torrent) -> bool {
    torrent.left_until_done == Some(0) && torrent.size_when_done.is_some_and(|size| size > 0)
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
                TorrentGetField::DownloadDir,
            ]),
            Some(vec![Id::Hash(hash.to_owned())]),
        )
        .await?;

    Ok(res.arguments.torrents.into_iter().next())
}

/// Where Transmission keeps one torrent's data (`download_dir`/`name`), and
/// what tells whether it can be moved now.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TorrentPlace {
    pub hash: String,
    pub name: String,
    /// As Transmission reports it.
    pub download_dir: String,
    /// Still to download or to verify: queued to or verifying, queued to or
    /// downloading, bytes left, or a magnet still fetching its metadata (which
    /// lists no files and nothing left). Its data may not be in
    /// `download_dir` yet (Transmission's incomplete folder, `.part` names)
    /// and Transmission still writes it.
    pub unfinished: bool,
    /// The text of a local error Transmission reports for it (`error` 3),
    /// such as a move that failed. Tracker errors are not local.
    pub local_error: Option<String>,
    /// Its files relative to `download_dir`, when asked for.
    pub files: Vec<TorrentFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TorrentFile {
    pub name: String,
    /// Its size in bytes, as the torrent says.
    pub length: i64,
    /// All of its bytes are there.
    pub complete: bool,
}

/// Every torrent's place, or those of `hashes` only; with their files when
/// `with_files`.
pub async fn torrent_places(
    transmission: &mut TransClient,
    hashes: Option<&[String]>,
    with_files: bool,
) -> transmission_rpc::types::Result<Vec<TorrentPlace>> {
    use transmission_rpc::types::{ErrorType, TorrentStatus};

    let mut fields = vec![
        TorrentGetField::Id,
        TorrentGetField::Name,
        TorrentGetField::HashString,
        TorrentGetField::DownloadDir,
        TorrentGetField::Status,
        TorrentGetField::LeftUntilDone,
        TorrentGetField::Error,
        TorrentGetField::ErrorString,
        TorrentGetField::MetadataPercentComplete,
    ];
    if with_files {
        fields.push(TorrentGetField::Files);
    }
    let res = transmission
        .torrent_get(
            Some(fields),
            hashes.map(|hashes| hashes.iter().cloned().map(Id::Hash).collect()),
        )
        .await?;
    Ok(res
        .arguments
        .torrents
        .into_iter()
        .filter_map(|torrent| {
            let busy = matches!(
                torrent.status,
                Some(
                    TorrentStatus::QueuedToVerify
                        | TorrentStatus::Verifying
                        | TorrentStatus::QueuedToDownload
                        | TorrentStatus::Downloading
                )
            );
            let without_metadata = torrent.metadata_percent_complete.unwrap_or(1.0) < 1.0;
            Some(TorrentPlace {
                hash: torrent.hash_string?,
                name: torrent.name.unwrap_or_default(),
                download_dir: torrent.download_dir?,
                unfinished: busy || without_metadata || torrent.left_until_done.unwrap_or(0) > 0,
                local_error: (torrent.error == Some(ErrorType::LocalError))
                    .then(|| torrent.error_string.unwrap_or_default()),
                files: torrent
                    .files
                    .unwrap_or_default()
                    .into_iter()
                    .map(|f| TorrentFile {
                        complete: f.bytes_completed >= f.length,
                        length: f.length,
                        name: f.name,
                    })
                    .collect(),
            })
        })
        .collect())
}

/// Asks Transmission to move the torrent `hash` to `location`, moving its
/// files with it (`torrent-set-location` with `move`). Transmission answers
/// before the files have moved; [`torrent_places`] shows the new folder once
/// they have. A refusal comes back as Transmission's result text.
pub async fn set_location(
    transmission: &mut TransClient,
    hash: &str,
    location: &Path,
) -> Result<(), String> {
    let Some(location) = location.to_str() else {
        return Err("the folder is not UTF-8".to_owned());
    };
    let res = transmission
        .torrent_set_location(
            vec![Id::Hash(hash.to_owned())],
            location.to_owned(),
            Some(true),
        )
        .await
        .map_err(|err| err.to_string())?;
    if res.is_ok() {
        Ok(())
    } else {
        Err(res.result)
    }
}

/// Why an item could not be added.
#[derive(Debug)]
pub enum AddError {
    /// No connection to Transmission could be made for the request, so it was
    /// never sent and Transmission cannot have taken the torrent.
    Unreachable(Box<dyn std::error::Error + Send + Sync>),
    /// The request failed once sent, or a later one did (no answer in time,
    /// protocol, decoding): Transmission may hold the torrent all the same.
    Rpc(Box<dyn std::error::Error + Send + Sync>),
    /// Transmission answered but refused, with its result text.
    Rejected(String),
}

impl fmt::Display for AddError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AddError::Unreachable(err) | AddError::Rpc(err) => write!(f, "{err}"),
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

impl AddError {
    /// Sorts the error of the `torrent-add` request itself: a failure to
    /// connect means it was never sent.
    fn of_add_request(err: Box<dyn std::error::Error + Send + Sync>) -> Self {
        match err.downcast_ref::<reqwest012::Error>() {
            Some(http) if http.is_connect() => AddError::Unreachable(err),
            _ => AddError::Rpc(err),
        }
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
    /// Where Transmission saves it, as Transmission reports it.
    pub download_dir: Option<String>,
    /// Its labels as Transmission reported them with the answer: for a
    /// duplicate, before any item label was put on.
    pub labels: Vec<String>,
}

impl AddedTorrent {
    pub fn has_label(&self, label: &str) -> bool {
        self.labels.iter().any(|l| l == label)
    }
}

async fn add_torrent(
    transmission: &mut TransClient,
    link: &str,
    download_dir: &Path,
    labels: AddLabels<'_>,
) -> Result<TorrentAddedOrDuplicate, AddError> {
    let labels = std::iter::once(BOT_LABEL)
        .chain(labels.item)
        .chain(labels.command)
        .map(str::to_owned)
        .collect();
    let mut res = transmission
        .torrent_add(TorrentAddArgs {
            filename: Some(link.to_owned()),
            labels: Some(labels),
            download_dir: download_dir.to_str().map(|x| x.to_owned()),
            ..Default::default()
        })
        .await
        .map_err(AddError::of_add_request)?;

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

/// Adds `link` to Transmission, saving to `download_dir`, with the bot's label
/// and the given [`AddLabels`].
///
/// A torrent that is already present is left in place, except that a
/// bot-labelled one that has finished (queued to seed or seeding) is stopped,
/// and a bot-labelled one without the item label gets it (Transmission ignores
/// the labels of an add it answers `duplicate`). The command label is never
/// put on a torrent that was already there. A person's torrent keeps its
/// labels. Prints `Added`, `Stopped` or `Already` with the torrent's name and
/// hash.
pub async fn add_item(
    transmission: &mut TransClient,
    link: &str,
    download_dir: &Path,
    labels: AddLabels<'_>,
    redactor: &Redactor,
) -> Result<AddedTorrent, AddError> {
    let item_label = labels.item;
    match add_torrent(transmission, link, download_dir, labels).await? {
        TorrentAddedOrDuplicate::TorrentDuplicate(torrent) => {
            let hash = torrent.hash_string.as_deref().unwrap();
            let labels = torrent.labels.as_deref();

            if let Some(item_label) =
                item_label.filter(|label| has_label(labels, BOT_LABEL) && !has_label(labels, label))
            {
                let labels = labels
                    .unwrap_or_default()
                    .iter()
                    .map(String::as_str)
                    .chain([item_label])
                    .map(str::to_owned)
                    .collect();
                transmission
                    .torrent_set(
                        TorrentSetArgs::new().labels(labels),
                        Some(vec![Id::Hash(hash.to_owned())]),
                    )
                    .await
                    .inspect_err(|err| eprintln!("{}", redactor.apply(&err.to_string())))
                    .ok();
            }

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
                labels: torrent.labels.clone().unwrap_or_default(),
                hash: torrent.hash_string.unwrap(),
                name: torrent.name.unwrap(),
                download_dir: torrent.download_dir,
            })
        }
        TorrentAddedOrDuplicate::TorrentAdded(torrent) => {
            let hash = torrent.hash_string.as_deref().unwrap();
            let name = torrent.name.as_deref().unwrap();

            println!("Added {} | {}", name, hash);

            Ok(AddedTorrent {
                kind: AddKind::Added,
                labels: torrent.labels.clone().unwrap_or_default(),
                hash: torrent.hash_string.unwrap(),
                name: torrent.name.unwrap(),
                download_dir: torrent.download_dir,
            })
        }
        // `add_torrent` returns this case as `AddError::Rejected`.
        TorrentAddedOrDuplicate::Error => Err(AddError::Rejected(String::new())),
    }
}

/// Takes `label` off the torrent `hash`, keeping its other labels. A torrent
/// that is gone or does not carry it is left alone; an error is printed and
/// otherwise ignored.
pub async fn remove_label(
    transmission: &mut TransClient,
    hash: &str,
    label: &str,
    redactor: &Redactor,
) {
    let torrent = match get_torrent(transmission, hash).await {
        Ok(Some(torrent)) => torrent,
        Ok(None) => return,
        Err(err) => return eprintln!("{}", redactor.apply(&err.to_string())),
    };
    let labels = torrent.labels.unwrap_or_default();
    if !labels.iter().any(|l| l == label) {
        return;
    }
    let kept = labels.into_iter().filter(|l| l != label).collect();
    if let Err(err) = transmission
        .torrent_set(
            TorrentSetArgs::new().labels(kept),
            Some(vec![Id::Hash(hash.to_owned())]),
        )
        .await
    {
        eprintln!("{}", redactor.apply(&err.to_string()));
    }
}

/// Which torrent [`rename_torrent`] is renaming, which decides how far it may go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameMode {
    /// A torrent the caller has just added. The single file is renamed, and the
    /// torrent is removed together with its data when its name cannot be
    /// derived.
    Added,
    /// A torrent Transmission already had. Its file is renamed only while it
    /// sits in `download_dir` and its name is not in the `trname` form yet (a
    /// rename that was cut short earlier). A name in that form is left alone,
    /// because applying the episode offset again would change the episode, and
    /// a torrent in another folder was named after another title. Nothing is
    /// ever removed.
    Existing,
}

/// What one [`rename_torrent`] call came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Renamed {
    /// The file has the new name now.
    To(String),
    /// Nothing more to do: the torrent was removed or is gone, the name it
    /// would take is taken by another file, or (in [`RenameMode::Existing`])
    /// its name is to be left as it is.
    Finished,
    /// Not now: the metadata or the rename is not there yet; try again later.
    NotYet,
}

/// Renames the torrent's single file to the `trname` name for `download_dir`
/// (`.../<title>/Season NN`). What happens when the name cannot be derived, or
/// is in that form already, depends on `mode`. Torrents with more than one
/// file are left alone, and so is a torrent whose new name is taken by a file
/// in its folder (looked up on this host's disk, which sees the folders at the
/// paths Transmission reports).
pub async fn rename_torrent(
    transmission: &mut TransClient,
    hash: &str,
    download_dir: &Path,
    starts_episode_at: isize,
    mode: RenameMode,
) -> transmission_rpc::types::Result<Renamed> {
    let Some(torrent) = get_torrent(transmission, hash).await? else {
        return Ok(match mode {
            RenameMode::Added => Renamed::NotYet,
            RenameMode::Existing => Renamed::Finished,
        });
    };

    if mode == RenameMode::Existing
        && torrent.download_dir.as_deref().map(Path::new) != Some(download_dir)
    {
        return Ok(Renamed::Finished);
    }

    let file_count = torrent.file_count.unwrap();
    if file_count == 1 {
        let old_file_name = torrent.name.clone().unwrap();

        if mode == RenameMode::Existing && looks_renamed(&old_file_name, download_dir) {
            return Ok(Renamed::Finished);
        }

        // Read without the revision marker: `trname` does not read `06v2`
        // as episode 6 in every name (Erai-raws' gives episode 34).
        let derived = trname_raw(
            download_dir,
            &Release::without_version(&old_file_name),
            starts_episode_at,
        );
        match (mode, derived) {
            (RenameMode::Existing, Some((_, file, _))) if file.already_formatted => {
                return Ok(Renamed::Finished);
            }
            (_, Some((_, _, new_file_name))) => {
                // Never onto a name that is taken: Transmission would answer
                // success and point the torrent at the other file, leaving its
                // own under the old name (libtransmission's `renamePath`
                // renames on disk only when the target is not there). That is
                // the old video when a higher revision of a received episode
                // comes in; its replacement is the worker's (see
                // `crate::worker::revisions`).
                let folder = torrent
                    .download_dir
                    .as_deref()
                    .map_or(download_dir, Path::new);
                if std::fs::symlink_metadata(folder.join(&new_file_name)).is_ok() {
                    println!(
                        "Not renaming {old_file_name}: {new_file_name} is taken in {}",
                        folder.display()
                    );
                    return Ok(Renamed::Finished);
                }
                let res = transmission
                    .torrent_rename_path(
                        vec![Id::Hash(hash.to_owned())],
                        old_file_name,
                        new_file_name.clone(),
                    )
                    .await?;

                if res.result == "success" {
                    return Ok(Renamed::To(new_file_name));
                }
            }
            (RenameMode::Added, None) => {
                let _res = transmission
                    .torrent_remove(vec![Id::Hash(hash.to_owned())], true)
                    .await?;
            }
            (RenameMode::Existing, None) => return Ok(Renamed::Finished),
        }
    } else if file_count > 1 && mode == RenameMode::Existing {
        return Ok(Renamed::Finished);
    }

    Ok(Renamed::NotYet)
}

/// Whether `name` is the name `trname` gives in `download_dir`
/// (`.../<title>/Season NN`): the folder's title and an episode, as in
/// `<title> S01E05.mkv`, `S01E05.5` or a three-digit `S01E105`, with the
/// title's case not counting. `trname` itself only accepts a two-digit
/// episode, so it would read `E105` as a release name and take `05` from it.
/// A release that merely ends in `SxxEyy` under another title is not one.
fn looks_renamed(name: &str, download_dir: &Path) -> bool {
    static EPISODE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^ S\d{2,}E\d{2,}(?:\.\d)?\.\w+$").unwrap());
    let Some(title) = download_dir
        .components()
        .rev()
        .nth(1)
        .and_then(|c| c.as_os_str().to_str())
    else {
        return false;
    };
    match name.get(..title.len()) {
        Some(head) if head.to_lowercase() == title.to_lowercase() => {
            EPISODE.is_match(&name[title.len()..])
        }
        _ => false,
    }
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
    /// An attempt every second, 17 attempts.
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
#[allow(clippy::too_many_arguments)]
pub async fn rename_with_retries(
    transmission: &mut TransClient,
    hash: &str,
    save_path: &Path,
    episode: isize,
    mode: RenameMode,
    policy: RenamePolicy,
    redactor: &Redactor,
    cancel: &CancellationToken,
) {
    for _ in 0..policy.attempts {
        tokio::select! {
            _ = sleep(policy.delay) => {}
            _ = cancel.cancelled() => break,
        }

        let res = rename_torrent(transmission, hash, save_path, episode, mode)
            .await
            .inspect_err(|err| println!("{}", redactor.apply(&err.to_string())));

        if let Ok(Renamed::To(_) | Renamed::Finished) = res {
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

/// Removes (keeping the downloaded data) every finished bot-labelled torrent
/// for which `is_kept(hash, labels)` is false, i.e. torrents whose item is no
/// longer in the feeds. One still downloading stays until it has finished, so a
/// Transmission that was down while its item left the feed does not leave the
/// episode half downloaded. Prints one `Removed` line per torrent. A failure to
/// list or remove is printed and leaves things as they are.
pub async fn remove_stale(
    transmission: &mut TransClient,
    is_kept: impl Fn(&str, &[String]) -> bool,
    redactor: &Redactor,
) -> Vec<RemovedTorrent> {
    match get_torrents(transmission).await {
        Ok(torrents) => {
            // remove oldest torrents
            let oldest_torrents = torrents
                .into_iter()
                .filter(|torrent| has_label(torrent.labels.as_deref(), BOT_LABEL))
                .filter(is_finished)
                .filter(|torrent| {
                    !is_kept(
                        torrent.hash_string.as_deref().unwrap(),
                        torrent.labels.as_deref().unwrap_or_default(),
                    )
                })
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

#[cfg(test)]
mod tests {
    use super::{item_label, item_of_label, looks_renamed};

    #[test]
    fn an_item_label_names_its_channel_and_identity_key() {
        let label = item_label("0b7c6a52-9d1e-4b8a-a3c1-5f2e8d9c7b10", "guid:ab12");
        assert_eq!(
            label,
            "trss-item:0b7c6a52-9d1e-4b8a-a3c1-5f2e8d9c7b10:guid:ab12"
        );
        assert_eq!(
            item_of_label(&label),
            Some(("0b7c6a52-9d1e-4b8a-a3c1-5f2e8d9c7b10", "guid:ab12"))
        );
        assert_eq!(item_of_label("managed:transmission-rss"), None);
        assert_eq!(item_of_label("trss-item:no-key"), None);
    }

    #[test]
    fn a_trname_name_is_told_apart_from_a_release_name() {
        let dir = std::path::Path::new("/media/anime/Slime/Season 04");
        for name in [
            "Slime S04E38.mkv",
            "SLIME S04E38.mkv",
            "Slime S04E105.mkv",
            "Slime S04E05.5.mp4",
        ] {
            assert!(looks_renamed(name, dir), "{name}");
        }
        for name in [
            "[SubsPlease] Tensei Shitara Slime Datta Ken - 62 (1080p) [AAAA0006].mkv",
            "Tensura S04E62.mkv",
            "S04E05.mkv",
            "Slime.S04E05.1080p.WEB.mkv",
            "Slime S04E05 (1080p).mkv",
            "SlimeS04E05.mkv",
            "Slime Special.mkv",
        ] {
            assert!(!looks_renamed(name, dir), "{name}");
        }
        assert!(!looks_renamed(
            "Slime S04E38.mkv",
            std::path::Path::new("/")
        ));
    }
}
