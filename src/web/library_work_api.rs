//! `GET /api/library/works/{id}`: one work for the work detail screen
//! (`docs/specs/library.md`, 작품 상세 화면), the video side: what the library
//! recorded for the work and the rules that collect into its folder.
//!
//! ```json
//! {
//!   "id": "…", "name": "Lycoris Recoil", "missing": false,
//!   "watch_folder": { "id": "…", "path": "/media/anime" },
//!   "folder_path": "/media/anime/Lycoris Recoil",
//!   "added_at": null,
//!   "seasons": [{
//!     "number": 1,
//!     "episodes": [{
//!       "episode": "01", "sort": 1.0,
//!       "video":    [{ "path": "Season 01/… S01E01.mkv", "added_at": null }],
//!       "subtitle": [{ "path": "Season 01/… S01E01.ko.ass", "added_at": 1760000100000 }]
//!     }]
//!   }],
//!   "unrecognized": [{ "path": "Extras/PV.mkv", "reason": "outside_season", "message": "…" }],
//!   "rules": [{
//!     "id": "…", "channel": { "id": "…", "name": null, "host": "example.org" },
//!     "match": "Lycoris", "directory": "Lycoris Recoil/Season 01",
//!     "save_path": "/media/anime/Lycoris Recoil/Season 01", "state": "active"
//!   }]
//! }
//! ```
//!
//! - `name` is the work's folder name and `folder_path` the folder itself (the
//!   watch folder's path and the name). A work whose folder is gone (`missing`)
//!   answers what was recorded last; the screen shows it as the last record,
//!   not as files that are there.
//! - `episodes` are in ascending order. `episode` is as written in the file
//!   names (`01` and `013`/`13` are one episode, written as the smallest form);
//!   `sort` is its number, `null` when it is no number (`SP`). `added_at` is
//!   Unix milliseconds, `null` when unknown (the file was there before the app
//!   first looked).
//! - `unrecognized` are the files that could not be attached to an episode,
//!   with the reason's code and a sentence for it.
//! - `rules` are the rules whose save folder is in this work's folder: the
//!   first part of the rule's `directory` below the collect folder is the work's
//!   folder name, and the work is in the collect folder or the archive folder
//!   (where `보관` moves it). Archived rules are listed with their `state`.
//!   Empty when the work is in some other watch folder or no collect folder is
//!   set.
//! - `404` for a work that is not in the library.

use std::path::Path as FsPath;

use axum::{
    extract::{Path, State},
    routing::get,
    Json, Router,
};
use serde::Serialize;
use url::Url;

use super::{ApiError, AppState};
use crate::{
    rss::save_path,
    store::{
        channels::ChannelWithRules,
        library::{EpisodeDetail, FileRecord, LibraryError, WorkDetail},
    },
    worker::commands::rule_archive::{work_folder, WorkFolder},
};

#[cfg(test)]
mod tests;

pub fn routes() -> Router<AppState> {
    Router::new().route("/library/works/{id}", get(show))
}

#[derive(Serialize)]
struct WatchFolderRef {
    id: String,
    path: String,
}

#[derive(Serialize)]
struct FileView {
    path: String,
    added_at: Option<i64>,
}

impl From<FileRecord> for FileView {
    fn from(file: FileRecord) -> Self {
        FileView {
            path: file.path,
            added_at: file.added_at,
        }
    }
}

#[derive(Serialize)]
struct EpisodeView {
    episode: String,
    sort: Option<f64>,
    video: Vec<FileView>,
    subtitle: Vec<FileView>,
}

impl From<EpisodeDetail> for EpisodeView {
    fn from(episode: EpisodeDetail) -> Self {
        EpisodeView {
            episode: episode.episode,
            sort: episode.number,
            video: episode.video.into_iter().map(FileView::from).collect(),
            subtitle: episode.subtitle.into_iter().map(FileView::from).collect(),
        }
    }
}

#[derive(Serialize)]
struct SeasonView {
    number: u32,
    episodes: Vec<EpisodeView>,
}

#[derive(Serialize)]
struct UnrecognizedView {
    path: String,
    reason: &'static str,
    message: &'static str,
}

#[derive(Debug, PartialEq, Serialize)]
struct ChannelRef {
    id: String,
    name: Option<String>,
    host: String,
}

#[derive(Debug, PartialEq, Serialize)]
struct RuleRef {
    id: String,
    channel: ChannelRef,
    r#match: Option<String>,
    directory: String,
    save_path: String,
    state: &'static str,
}

#[derive(Serialize)]
struct WorkDetailView {
    id: String,
    name: String,
    missing: bool,
    watch_folder: WatchFolderRef,
    folder_path: String,
    added_at: Option<i64>,
    seasons: Vec<SeasonView>,
    unrecognized: Vec<UnrecognizedView>,
    rules: Vec<RuleRef>,
}

fn host_of(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The rules that save into the work's folder (see the module docs), by
/// channel order and then rule order.
fn rules_of(
    work: &WorkDetail,
    collect: Option<&str>,
    archive: Option<&str>,
    channels: &[ChannelWithRules],
) -> Vec<RuleRef> {
    let Some(collect) = collect else {
        return Vec::new();
    };
    let watched = FsPath::new(&work.watch_folder_path);
    if watched != FsPath::new(collect) && archive.is_none_or(|a| watched != FsPath::new(a)) {
        return Vec::new();
    }
    let collect = FsPath::new(collect);
    let mut rules = Vec::new();
    for cwr in channels {
        for rule in &cwr.rules {
            if work_folder(collect, &rule.directory) != WorkFolder::Named(work.dir_name.clone()) {
                continue;
            }
            rules.push(RuleRef {
                id: rule.id.clone(),
                channel: ChannelRef {
                    id: cwr.channel.id.clone(),
                    name: cwr.channel.name.clone(),
                    host: host_of(&cwr.channel.url),
                },
                r#match: rule.r#match.clone(),
                directory: rule.directory.clone(),
                save_path: save_path(collect, FsPath::new(&rule.directory))
                    .to_string_lossy()
                    .into_owned(),
                state: rule.state.as_str(),
            });
        }
    }
    rules
}

async fn show(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<WorkDetailView>, ApiError> {
    let work = state
        .library
        .work_detail(&id)
        .await
        .map_err(|e| match e {
            LibraryError::Db(e) => ApiError::Internal(e.to_string()),
            LibraryError::Duplicate | LibraryError::Automatic | LibraryError::Changed => {
                ApiError::Internal(format!("unexpected: {e}"))
            }
        })?
        .ok_or_else(|| ApiError::not_found("이 작품을 찾지 못했어요."))?;
    let settings = state
        .settings
        .collection()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let channels = state.channels.list_channels_with_rules().await?;
    let rules = rules_of(
        &work,
        settings.as_ref().map(|s| s.folder.as_str()),
        settings.as_ref().and_then(|s| s.archive_folder.as_deref()),
        &channels,
    );

    let folder_path = FsPath::new(&work.watch_folder_path)
        .join(&work.dir_name)
        .to_string_lossy()
        .into_owned();
    Ok(Json(WorkDetailView {
        id: work.id,
        name: work.dir_name,
        missing: work.missing,
        watch_folder: WatchFolderRef {
            id: work.watch_folder_id,
            path: work.watch_folder_path,
        },
        folder_path,
        added_at: work.first_seen_at,
        seasons: work
            .seasons
            .into_iter()
            .map(|season| SeasonView {
                number: season.number,
                episodes: season.episodes.into_iter().map(EpisodeView::from).collect(),
            })
            .collect(),
        unrecognized: work
            .unrecognized
            .into_iter()
            .map(|u| UnrecognizedView {
                path: u.path,
                reason: u.reason.code(),
                message: u.reason.message(),
            })
            .collect(),
        rules,
    }))
}
