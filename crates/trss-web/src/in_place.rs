//! Whether the web can already tell that an episode's place holds the same or a
//! higher revision than the one `다시 받기` would receive, so that the screens
//! do not offer a button the worker is bound to refuse
//! ([`NotRetryable::InPlace`](trss_collect::commands::receive_once::NotRetryable::InPlace)).
//!
//! The web cannot ask Transmission (deployment trust boundary) and looks at a
//! media folder only for whether a file is there, so it tells only from what
//! it has, and only when it is sure: with
//! any doubt the button stays, and the worker, which stays the authority,
//! looks at the folder when the command runs and refuses as before. A revision
//! row waiting for `다시 받기` (its download stopped, its replacement ended
//! with no video, or `버전 미상`) is held when a file is at its episode name in
//! its folder (a plain look at the media folder; a missing folder or any read
//! error is no) and there is evidence of one of two kinds, for a revision of
//! the same release (the same stem) that is the same or higher:
//!
//! - a replacement of the same episode file (the row's folder and episode name)
//!   that is `done`: its video took the episode name;
//! - an ordinary item of the channel (history says `received`, with no
//!   replacement row) that the rule's own cycle would have named as that
//!   episode file in that folder, whose torrent is in the worker's last list
//!   of Transmission's torrents ([`StatusStore::torrent_listing`]). That list
//!   has to be recent ([`LISTING_FRESH_FOR`]) and taken after the item got its
//!   result, or it cannot say the torrent is still there. The file at the
//!   episode name is taken to be the item's because the rule's cycle names it
//!   so; the web does not tell which torrent holds it.
//!
//! [`StatusStore::torrent_listing`]: trss_collect::store::status::StatusStore::torrent_listing

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use super::{commands_api::now_millis, ApiError, AppState};
use trss_collect::{
    plan::rule_destination,
    release_name::ReleaseName,
    revision,
    revisions::{episode_name, same_folder},
    store::{
        history::{HistoryItem, HistoryResult},
        revisions::{Revision, RevisionState},
        status::TorrentListing,
    },
};
use trss_core::Millis;

#[cfg(test)]
mod tests;

/// How old the worker's list of Transmission's torrents may be to still say a
/// torrent is there. The worker writes it every cycle; one older than this is
/// the worker not running or not reaching Transmission, and the torrent may
/// be gone since.
pub const LISTING_FRESH_FOR: Millis = 30 * 60 * 1000;

/// An episode whose place holds a revision as high as the one to receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InPlace {
    /// The highest such revision told.
    pub version: u32,
}

impl InPlace {
    /// The sentence for the person: why `다시 받기` is not offered or not
    /// accepted.
    pub fn message(&self) -> String {
        format!(
            "이미 같거나 더 높은 수정본({})이 있어서 다시 받지 않아요.",
            revision::label(self.version)
        )
    }
}

/// What the web knows of the places of episodes, read once for the revisions
/// of one request.
pub struct Evidence<'a> {
    state: &'a AppState,
    /// The worker's last list of Transmission's torrents, when it is recent.
    listing: Option<TorrentListing>,
    /// The collect folder, which the rules' folders are under.
    collect: Option<PathBuf>,
    /// The titles of the channels asked about, by channel.
    titles: HashMap<String, Vec<(i64, String)>>,
}

/// Whether a file is at `path`: a read error (a folder that is not there, a
/// mount that is away) is no.
async fn has_file(path: &Path) -> bool {
    tokio::fs::symlink_metadata(path)
        .await
        .is_ok_and(|meta| meta.is_file())
}

fn internal(err: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(err.to_string())
}

impl<'a> Evidence<'a> {
    pub async fn load(state: &'a AppState) -> Result<Evidence<'a>, ApiError> {
        let now = now_millis();
        let listing = state
            .status
            .torrent_listing()
            .await
            .map_err(internal)?
            .filter(|listing| now.saturating_sub(listing.taken_at) <= LISTING_FRESH_FOR);
        let collect = state
            .settings
            .collection()
            .await
            .map_err(internal)?
            .map(|collect| PathBuf::from(collect.folder));
        Ok(Evidence {
            state,
            listing,
            collect,
            titles: HashMap::new(),
        })
    }

    /// Whether the place of `row`, the replacement of `item` that waits for
    /// `다시 받기`, is known to hold the revision of `item` or a higher one of
    /// its release.
    pub async fn held(
        &mut self,
        item: &HistoryItem,
        row: &Revision,
    ) -> Result<Option<InPlace>, ApiError> {
        // Nothing is told of a place with no file at the episode name, or one
        // that cannot be looked at: the worker may find what the web cannot.
        if !has_file(&Path::new(&row.folder).join(&row.episode_name)).await {
            return Ok(None);
        }
        let release = ReleaseName::read(&item.title);
        let mut best = self.done_replacement(item, row, &release).await?;
        if let Some(placed) = self.placed_item(item, row, &release, best).await? {
            best = Some(placed);
        }
        Ok(best.map(|version| InPlace { version }))
    }

    /// The highest revision of the release whose replacement of the episode
    /// is `done`, if it is at least `release`'s.
    async fn done_replacement(
        &self,
        item: &HistoryItem,
        row: &Revision,
        release: &ReleaseName,
    ) -> Result<Option<u32>, ApiError> {
        let rows = self
            .state
            .revisions
            .of_episode(row.folder.clone(), row.episode_name.clone())
            .await
            .map_err(internal)?;
        let mut best: Option<u32> = None;
        for done in rows {
            if done.state != RevisionState::Done
                || done.item_id == item.id
                || done.new_version < release.version
                || best.is_some_and(|best| best >= done.new_version)
            {
                continue;
            }
            let Some(other) = self
                .state
                .history
                .get(done.item_id)
                .await
                .map_err(internal)?
            else {
                continue;
            };
            if ReleaseName::read(&other.title).stem == release.stem {
                best = Some(done.new_version);
            }
        }
        Ok(best)
    }

    /// The highest revision above `already` of an ordinary item whose torrent
    /// Transmission holds and whose file is, by the rule's naming, the
    /// episode file of `row`.
    async fn placed_item(
        &mut self,
        item: &HistoryItem,
        row: &Revision,
        release: &ReleaseName,
        already: Option<u32>,
    ) -> Result<Option<u32>, ApiError> {
        let (Some(listing), Some(collect)) = (self.listing.clone(), self.collect.clone()) else {
            return Ok(None);
        };
        if !self.titles.contains_key(&item.channel_id) {
            let titles = self
                .state
                .history
                .titles_of_channel(item.channel_id.clone())
                .await
                .map_err(internal)?;
            self.titles.insert(item.channel_id.clone(), titles);
        }
        let mut candidates: Vec<(u32, i64)> = self.titles[&item.channel_id]
            .iter()
            .filter(|(id, _)| *id != item.id)
            .map(|(id, title)| (*id, ReleaseName::read(title)))
            .filter(|(_, other)| other.stem == release.stem && other.version >= release.version)
            .map(|(id, other)| (other.version, id))
            .collect();
        candidates.sort_by(|a, b| b.cmp(a));
        for (version, id) in candidates {
            if already.is_some_and(|already| already >= version) {
                break;
            }
            if self.is_placed(id, row, &listing, &collect).await? {
                return Ok(Some(version));
            }
        }
        Ok(None)
    }

    /// Whether the item `id` is received without a replacement row, its torrent
    /// is in `listing`, and the rule's cycle names its file as the episode file
    /// of `row`.
    async fn is_placed(
        &self,
        id: i64,
        row: &Revision,
        listing: &TorrentListing,
        collect: &Path,
    ) -> Result<bool, ApiError> {
        let Some(other) = self.state.history.get(id).await.map_err(internal)? else {
            return Ok(false);
        };
        let held = other.result == HistoryResult::Received
            && other.result_at <= listing.taken_at
            && other
                .torrent_hash
                .as_deref()
                .is_some_and(|hash| listing.holds(hash));
        let Some(rule_id) = other.rule_id.as_deref().filter(|_| held) else {
            return Ok(false);
        };
        // A replacement row tells where its video is; an item with one is not
        // named as the episode by its cycle.
        if self
            .state
            .revisions
            .by_item(other.id)
            .await
            .map_err(internal)?
            .is_some()
        {
            return Ok(false);
        }
        let Some(rule) = self.state.channels.get_rule(rule_id).await? else {
            return Ok(false);
        };
        let (save_path, episode) = rule_destination(collect, &rule);
        Ok(episode_name(&save_path, &other.title, episode).as_deref()
            == Some(row.episode_name.as_str())
            && same_folder(&save_path, Path::new(&row.folder)))
    }
}

/// The `버전 미상` items of `items` whose place is known to hold their
/// revision already, with the sentence for each, by history item.
pub async fn version_unknown_held(
    state: &AppState,
    items: &[HistoryItem],
) -> Result<HashMap<i64, String>, ApiError> {
    let mut held = HashMap::new();
    let mut evidence = None;
    for item in items
        .iter()
        .filter(|item| item.result == HistoryResult::VersionUnknown)
    {
        let Some(row) = state
            .revisions
            .by_item(item.id)
            .await
            .map_err(internal)?
            .filter(|row| row.state == RevisionState::Unknown)
        else {
            continue;
        };
        if evidence.is_none() {
            evidence = Some(Evidence::load(state).await?);
        }
        if let Some(place) = evidence.as_mut().unwrap().held(item, &row).await? {
            held.insert(item.id, place.message());
        }
    }
    Ok(held)
}
