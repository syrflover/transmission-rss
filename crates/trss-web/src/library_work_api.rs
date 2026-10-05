//! `GET /api/library/works/{id}`: one work for the work detail screen
//! (`docs/specs/library.md`, 작품 상세 화면), the video side: what the library
//! recorded for the work and the rules that collect into its folder; with the
//! cleanup of its stored files (보관 파일의 정리) and `GET /api/library/storage`
//! for the settings' 파일 용량과 정리.
//!
//! ```json
//! {
//!   "id": "…", "name": "Lycoris Recoil", "missing": false,
//!   "watch_folder": { "id": "…", "path": "/media/anime" },
//!   "folder_path": "/media/anime/Lycoris Recoil",
//!   "added_at": null,
//!   "native_title": "リコリス・リコイル",
//!   "korean_title": "리코리스 리코일",
//!   "seasons": [{
//!     "number": 1,
//!     "info": { "version": 2, "entries": [ … ], … },
//!     "episodes": [{
//!       "episode": "01", "sort": 1.0, "air_at": null,
//!       "video":    [{ "path": "Season 01/… S01E01.mkv", "added_at": null }],
//!       "subtitle": [{ "path": "Season 01/… S01E01.ko.ass", "added_at": 1760000100000,
//!                      "creator": { "source_id": "…", "name": "하느", "anime_no": 3441 },
//!                      "creator_version": 1 }],
//!       "revision": { "from": "v1", "to": "v2", "replaced_at": 1760000200000 },
//!       "failure": null,
//!       "stored": [{ "id": "…", "name": "Show - 02.ass", "creator": "하느", "format": "ass",
//!                    "stored_at": 1760000300000, "can_apply": true,
//!                    "awaiting_video": false, "approval_job": null }]
//!     }]
//!   }],
//!   "unrecognized": [{ "path": "Extras/PV.mkv", "reason": "outside_season", "message": "…" }],
//!   "cover_url": "/api/library/works/…/artwork/image?v=…",
//!   "cover_pending": false,
//!   "rules": [{
//!     "id": "…", "channel": { "id": "…", "name": null, "host": "example.org" },
//!     "match": "Lycoris", "directory": "Lycoris Recoil/Season 01",
//!     "save_path": "/media/anime/Lycoris Recoil/Season 01", "state": "active"
//!   }],
//!   "storage": {
//!     "total": 1843200,
//!     "cleanable": [{
//!       "id": "…", "name": "Show - 02.ass", "season": 1, "episode": 2, "creator": "하느",
//!       "format": "ass", "size": 40960, "stored_at": 1760000300000, "kind": "past",
//!       "blocked": null,
//!       "with": [{ "id": "…", "name": "Show - 02.ass", "kind": "subtitle", "size": 40960 },
//!                { "id": "…", "name": "Font.ttf", "kind": "font", "size": 802816 }],
//!       "kept": [{ "id": "…", "name": "Shared.otf", "kind": "font",
//!                  "reason": "다른 자막도 이 파일을 써요" }]
//!     }],
//!     "cleaning": [{ "id": "…", "name": "Show - 01.ass", "state": "held",
//!                    "reason": "작품 폴더를 찾지 못했어요" }]
//!   }
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
//! - A subtitle file's `creator` is the creator the user named for it
//!   (`null` is `제작자 알 수 없음`, which is what every file found in a watch
//!   folder is until then), and `creator_version` the version a change of it
//!   names ([`super::subtitle_creator_api`]).
//! - `revision` is the version line of an episode whose video was replaced by
//!   a higher revision of the same release (the latest such replacement;
//!   `from` is `null` when the old video's revision was not known), `null`
//!   otherwise. `failure` is a replacement of the episode's video that failed
//!   (`받기 실패`), in the shape of [`super::todo_api`]'s `revision` items
//!   (`at`, `history_item_id`, `reason`, `files`, and `다시 받기` with
//!   `can_retry`, `retry_blocked`, `command`), `null` otherwise; an episode
//!   left with no file under its name by such a failure still has a row,
//!   with no files. Both
//!   come from [`trss_collect::store::revisions`], for season folders of the work.
//! - `stored` are the stored subtitles on the episode with no applied copy of
//!   them beside a video (보관만 한 자막: another episode of a package, a
//!   format the order did not take; or one waiting for the episode's video),
//!   oldest first. `can_apply` says whether `적용` can ask for it: a format the app
//!   applies, received by a job whose record is there. `awaiting_video` says
//!   a job applies it once the episode's video comes (`영상 대기`): no
//!   `적용` is asked for it. `approval_job` is the job that waits for a
//!   person to approve replacing the episode's subtitle with it (`교체
//!   승인`, [`trss_jobs::place::replace`]), whose detail compares the two.
//!   An episode with only such subtitles has a row of its own with no files.
//! - `POST /api/library/works/{id}/stored/{stored_id}/apply` applies one: the
//!   job that stored it applies it beside the episode's video, as its first
//!   apply does, and the worker is woken. `202` `{ "job_id" }`; `404` for a
//!   stored subtitle that is not the work's; `409` (`conflict`, no `current`)
//!   with why not in `message` (an episode with a subtitle, whose change is a
//!   replacement's; a format the app does not apply; a job that runs or is
//!   held). `trss_jobs::place::records::choose_stored` has the rules.
//! - `storage` is the work's stored files (보관 파일의 정리,
//!   [`trss_jobs::place::cleanup`] has the rules). `total` is the length in
//!   bytes of the files the app keeps for the work and has not removed
//!   (subtitles, fonts, attachments and companion files; the cover is in
//!   `GET /api/library/storage`). `cleanable` are its stored subtitles that
//!   have no applied copy beside a video, by season, episode (`null`: on no
//!   episode, last) and when stored: `kind` is `past` (지난 수정본),
//!   `awaiting_video` (영상 대기), `unplaced` (회차에 붙지 않음) or `stored`
//!   (보관만 함); `blocked` is why it cannot be cleaned now (the work folder
//!   is not on disk now, or a job that runs, is held or waits uses it),
//!   `null` when it can. `with` are the files
//!   that would be removed with it now, its own first (`kind`: `subtitle`,
//!   `font`, `attachment`, `companion`), and `kept` the ones it uses that
//!   stay, with why. A file the app did not record is in neither.
//!   `cleaning` are the cleanups the worker has not carried out yet
//!   (`asked`) and the ones it held (`held`, with why).
//! - `POST /api/library/works/{id}/stored/{stored_id}/clean` with
//!   `{ "assets": [<the IDs the dialog showed in "with">] }` cleans one: the
//!   stored subtitle is no stored copy from then (no list shows it, no job
//!   applies it), what waited for its video is settled, and the worker,
//!   woken, removes the files that are still unused when it comes to them.
//!   `202` `{ "cleanup_id" }`; `404` for a stored subtitle that is not the
//!   work's or was cleaned; `409` (`conflict`) with why in `message` when it
//!   cannot be cleaned now, or, with the entry as it is now in `current`,
//!   when the files that would go are not the ones named.
//! - `GET /api/library/storage` is what each work keeps, for the settings'
//!   파일 용량과 정리:
//!   `{ "works": [{ "id", "name", "total", "kinds": [{ "kind", "count", "size" }],
//!   "cleanable" }] }`, the works with a file kept or a cover image, the
//!   largest `total` first. `kind` is `subtitle`, `font`, `attachment`
//!   (attachments and companion files) or `cover` (the work's current cover
//!   image, its length read from its file: `0` when the file is not there);
//!   `cleanable` is how many stored subtitles of the work can be cleaned now
//!   (`blocked` is `null`). The screen goes to the work's page from a line;
//!   nothing is cleaned from there.
//! - `native_title` is the first (lowest-numbered, not season 0) season's first
//!   linked AniList entry's native title, `null` without one.
//! - `korean_title` is the Anissia title (`subject`) of the anime the work's
//!   first (lowest-numbered) season with a subscription is connected to, `null`
//!   when no subscription is connected to a season of the work.
//! - `subscriptions` are the rules whose subscription is connected to a season
//!   of this work (the connection is made from the videos the rule received, see
//!   [`trss_collect::season_link`]), by season: the rule's ID, version and
//!   state, the anime (`anime_no`, `subject`), the subtitle mode and the creator
//!   followed. The head shows the selected season's creator and changes it
//!   through `PUT /api/rules/{id}/creator`.
//! - `anissia` is the season's link to an Anissia anime: its version, the anime
//!   (name, status and Anissia's page) and the subscription that holds the
//!   season, if one does (see [`super::seasons_anissia_api`]).
//! - `info` is the season's info, the linked AniList entries taken together
//!   (see [`super::seasons_api`]); a season with no link has version 0 and no
//!   values, which the screen shows as unknown. `air_at` is when AniList
//!   schedules the episode (Unix milliseconds), only while the entry is
//!   releasing and has a per-episode schedule, `null` otherwise.
//! - `unrecognized` are the files that could not be attached to an episode,
//!   with the reason's code and a sentence for it.
//! - `rules` are the rules whose save folder is in this work's folder: the
//!   first part of the rule's `directory` below the collect folder is the work's
//!   folder name, and the work is in the collect folder or the archive folder
//!   (where `보관` moves it). Archived rules are listed with their `state`.
//!   Empty when the work is in some other watch folder or no collect folder is
//!   set.
//! - `cover_url` is where the work's cover is served while it has an image
//!   reference, `null` otherwise; the cover view reads the rest from
//!   [`super::artwork_api`]. `cover_pending` is true while the worker still has
//!   to receive the cover's image (a cover that follows a season's link shows
//!   the old image until then), so the screen reads the work again until it
//!   is false.
//! - `404` for a work that is not in the library.

use std::{collections::HashMap, path::Path as FsPath};

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{
    artwork_api::image_url,
    seasons_anissia_api::{link_views, AnissiaLinkView},
    seasons_api::{season_view, work_infos, SeasonInfoView},
    subtitle_creator_api::CreatorView,
    todo_api::{failure_of, retry_offers, RetryOffer, RevisionFailure},
    ApiError, AppState,
};
use trss_collect::{
    commands::rule_archive::{work_folder, WorkFolder},
    revision::{season_episode, Release},
    rss::save_path,
    store::{channels::ChannelWithRules, revisions::Revision},
};
use trss_jobs::place::cleanup;
use trss_library::store::{
    artwork::JobKind,
    library::{EpisodeDetail, FileRecord, LibraryError, WorkDetail},
};

#[cfg(test)]
mod tests;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/library/works/{id}", get(show))
        .route(
            "/library/works/{id}/stored/{stored_id}/apply",
            post(apply_stored),
        )
        .route(
            "/library/works/{id}/stored/{stored_id}/clean",
            post(clean_stored),
        )
        .route("/library/storage", get(storage))
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

/// A subtitle file, with the creator the user named for it.
#[derive(Serialize)]
struct SubtitleView {
    path: String,
    added_at: Option<i64>,
    /// The creator the user named; `null` is `제작자 알 수 없음`.
    creator: Option<CreatorView>,
    /// The version of the file's creator, which a change of it names
    /// (see [`super::subtitle_creator_api`]).
    creator_version: i64,
}

impl From<FileRecord> for SubtitleView {
    fn from(file: FileRecord) -> Self {
        SubtitleView {
            path: file.path,
            added_at: file.added_at,
            creator: file.creator.map(CreatorView::from),
            creator_version: file.creator_version,
        }
    }
}

#[derive(Serialize)]
struct EpisodeView {
    episode: String,
    sort: Option<f64>,
    air_at: Option<i64>,
    video: Vec<FileView>,
    subtitle: Vec<SubtitleView>,
    /// The episode's video was replaced by a higher revision: the version line.
    revision: Option<RevisionView>,
    /// The replacement of the episode's video failed (`받기 실패`).
    failure: Option<RevisionFailure>,
    /// Stored subtitles on the episode with no applied copy (`보관본 있음`).
    stored: Vec<StoredView>,
}

/// A stored subtitle on an episode with no applied copy of it.
#[derive(Serialize)]
struct StoredView {
    id: String,
    name: String,
    creator: Option<String>,
    /// `ass`, `srt`, `smi` or `other`.
    format: &'static str,
    stored_at: i64,
    can_apply: bool,
    /// A job applies it once the episode's video comes (`영상 대기`).
    awaiting_video: bool,
    /// The job that waits for a person to approve replacing the episode's
    /// subtitle with it (`교체 승인`), whose detail compares the two.
    approval_job: Option<String>,
}

impl From<trss_jobs::place::records::StoredOnly> for StoredView {
    fn from(stored: trss_jobs::place::records::StoredOnly) -> Self {
        StoredView {
            can_apply: stored.format.extension().is_some() && stored.job_id.is_some(),
            id: stored.id,
            name: stored.name,
            creator: stored.creator,
            format: stored.format.code(),
            stored_at: stored.stored_at,
            awaiting_video: stored.awaiting_video,
            approval_job: stored.awaiting_approval,
        }
    }
}

#[derive(Serialize)]
struct RevisionView {
    /// `v1`; `null` when the old video's revision was not known.
    from: Option<String>,
    /// `v2`.
    to: String,
    /// When the new video took the episode name, Unix milliseconds.
    replaced_at: i64,
}

impl From<EpisodeDetail> for EpisodeView {
    fn from(episode: EpisodeDetail) -> Self {
        EpisodeView {
            episode: episode.episode,
            sort: episode.number,
            air_at: None,
            video: episode.video.into_iter().map(FileView::from).collect(),
            subtitle: episode
                .subtitle
                .into_iter()
                .map(SubtitleView::from)
                .collect(),
            revision: None,
            failure: None,
            stored: Vec::new(),
        }
    }
}

#[derive(Serialize)]
struct SeasonView {
    number: u32,
    info: SeasonInfoView,
    /// The Anissia anime the season is linked to (see [`super::seasons_anissia_api`]).
    anissia: AnissiaLinkView,
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

/// A subscription rule connected to a season of the work.
#[derive(Serialize)]
struct WorkSubscription {
    season: u32,
    rule_id: String,
    rule_version: i64,
    /// `active`, `paused` or `archived`.
    rule_state: &'static str,
    anime_no: i64,
    /// The Anissia title; `null` when the app has no snapshot of the anime.
    subject: Option<String>,
    /// `follow`, `undecided` or `none`.
    subtitles: &'static str,
    creator: Option<String>,
}

#[derive(Serialize)]
struct WorkDetailView {
    id: String,
    name: String,
    missing: bool,
    watch_folder: WatchFolderRef,
    folder_path: String,
    added_at: Option<i64>,
    native_title: Option<String>,
    korean_title: Option<String>,
    subscriptions: Vec<WorkSubscription>,
    seasons: Vec<SeasonView>,
    unrecognized: Vec<UnrecognizedView>,
    rules: Vec<RuleRef>,
    cover_url: Option<String>,
    cover_pending: bool,
    storage: StorageView,
}

/// The work's stored files and their cleanup.
#[derive(Serialize)]
struct StorageView {
    total: u64,
    cleanable: Vec<CleanableView>,
    cleaning: Vec<CleaningView>,
}

/// A file that would go with a stored subtitle.
#[derive(Serialize)]
struct FileGoingView {
    id: String,
    name: String,
    kind: &'static str,
    size: u64,
}

/// A file a stored subtitle uses that stays, and why.
#[derive(Serialize)]
struct FileStayingView {
    id: String,
    name: String,
    kind: &'static str,
    reason: &'static str,
}

/// A stored subtitle a person may clean.
#[derive(Serialize)]
struct CleanableView {
    id: String,
    name: String,
    season: u32,
    episode: Option<i64>,
    creator: Option<String>,
    format: &'static str,
    size: u64,
    stored_at: i64,
    /// `past`, `awaiting_video`, `unplaced` or `stored`.
    kind: &'static str,
    blocked: Option<&'static str>,
    with: Vec<FileGoingView>,
    kept: Vec<FileStayingView>,
}

impl From<cleanup::Cleanable> for CleanableView {
    fn from(entry: cleanup::Cleanable) -> Self {
        CleanableView {
            id: entry.id,
            name: entry.name,
            season: entry.season,
            episode: entry.episode,
            creator: entry.creator,
            format: entry.format.code(),
            size: entry.size,
            stored_at: entry.stored_at,
            kind: entry.kind.code(),
            blocked: entry.blocked,
            with: entry
                .with
                .into_iter()
                .map(|f| FileGoingView {
                    id: f.id,
                    name: f.name,
                    kind: f.kind.code(),
                    size: f.size,
                })
                .collect(),
            kept: entry
                .kept
                .into_iter()
                .map(|f| FileStayingView {
                    id: f.id,
                    name: f.name,
                    kind: f.kind.code(),
                    reason: f.reason,
                })
                .collect(),
        }
    }
}

/// A cleanup the worker has not carried out, or held.
#[derive(Serialize)]
struct CleaningView {
    id: String,
    name: String,
    /// `asked` or `held`.
    state: String,
    reason: Option<String>,
}

impl From<cleanup::WorkFiles> for StorageView {
    fn from(files: cleanup::WorkFiles) -> Self {
        StorageView {
            total: files.total,
            cleanable: files
                .cleanable
                .into_iter()
                .map(CleanableView::from)
                .collect(),
            cleaning: files
                .cleaning
                .into_iter()
                .map(|k| CleaningView {
                    id: k.id,
                    name: k.name,
                    state: k.state,
                    reason: k.reason,
                })
                .collect(),
        }
    }
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

/// Puts each stored subtitle with no applied copy on its episode's row,
/// making a row with no files for an episode that has none.
fn attach_stored(seasons: &mut [SeasonView], stored: Vec<trss_jobs::place::records::StoredOnly>) {
    for one in stored {
        let Some(season) = seasons.iter_mut().find(|s| s.number == one.season) else {
            continue;
        };
        let number = one.episode as f64;
        let index = match season.episodes.iter().position(|e| e.sort == Some(number)) {
            Some(index) => index,
            None => {
                let index = season
                    .episodes
                    .iter()
                    .position(|e| e.sort.is_some_and(|s| s > number))
                    .unwrap_or(season.episodes.len());
                season.episodes.insert(
                    index,
                    EpisodeView {
                        episode: format!("{:02}", one.episode),
                        sort: Some(number),
                        air_at: None,
                        video: Vec::new(),
                        subtitle: Vec::new(),
                        revision: None,
                        failure: None,
                        stored: Vec::new(),
                    },
                );
                index
            }
        };
        season.episodes[index].stored.push(StoredView::from(one));
    }
}

/// Puts each replacement of the work on its episode's row: the version line
/// of the latest one that is done, and a failure. A failure whose episode has
/// no row (its old video is gone and the new one does not have the episode
/// name yet) gets a row of its own with no files. Rows whose folder is not a
/// season folder of the work, or whose name is no episode, are left out.
fn attach_revisions(
    seasons: &mut [SeasonView],
    rows: Vec<Revision>,
    work_folder: &FsPath,
    mut offers: HashMap<i64, RetryOffer>,
) {
    for row in rows {
        let Some((season, episode)) = season_episode(&row.episode_name) else {
            continue;
        };
        if FsPath::new(&row.folder).parent() != Some(work_folder) {
            continue;
        }
        let number = episode.parse::<f64>().ok();
        let same = |e: &EpisodeView| match (e.sort, number) {
            (Some(a), Some(b)) => a == b,
            _ => e.episode == episode,
        };
        let failure = row.is_failure();
        let Some(at) = seasons.iter().position(|s| s.number == season) else {
            continue;
        };
        let episodes = &mut seasons[at].episodes;
        let index = match episodes.iter().position(same) {
            Some(index) => index,
            None if failure => {
                let index = episodes
                    .iter()
                    .position(|e| matches!((e.sort, number), (Some(a), Some(b)) if a > b))
                    .unwrap_or(episodes.len());
                episodes.insert(
                    index,
                    EpisodeView {
                        episode: episode.clone(),
                        sort: number,
                        air_at: None,
                        video: Vec::new(),
                        subtitle: Vec::new(),
                        revision: None,
                        failure: None,
                        stored: Vec::new(),
                    },
                );
                index
            }
            None => continue,
        };
        let view = &mut episodes[index];
        if failure {
            let mut shown = failure_of(&row, Some(work_folder));
            shown.retry = offers.remove(&row.item_id).unwrap_or_default();
            view.failure = Some(shown);
        } else if let Some(replaced_at) = row.replaced_at {
            if view
                .revision
                .as_ref()
                .is_none_or(|r| r.replaced_at <= replaced_at)
            {
                view.revision = Some(RevisionView {
                    from: row.old_version.map(Release::label),
                    to: Release::label(row.new_version),
                    replaced_at,
                });
            }
        }
    }
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

    let (cover_url, cover_pending) = match state.artwork.store.selection(&work.id).await {
        Ok(selection) => (
            selection.image.map(|image| image_url(&work.id, &image.id)),
            selection.job.is_some_and(|job| job.kind == JobKind::Fetch),
        ),
        Err(e) => return Err(ApiError::Internal(e.to_string())),
    };

    let connected = state.channels.subscriptions_of_work(&work.id).await?;
    let numbers: Vec<u32> = work.seasons.iter().map(|s| s.number).collect();
    let mut anissia_links =
        link_views(&state, &work.id, &work.dir_name, &numbers, &connected).await?;
    let animes = state
        .anissia_store
        .animes(
            connected
                .iter()
                .filter_map(|(_, rule)| rule.subscription.as_ref().map(|s| s.anissia_anime_no))
                .collect(),
        )
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut subscriptions: Vec<WorkSubscription> = connected
        .into_iter()
        .filter_map(|(season, rule)| {
            let subscription = rule.subscription.as_ref()?;
            Some(WorkSubscription {
                season,
                rule_id: rule.id.clone(),
                rule_version: rule.version,
                rule_state: rule.state.as_str(),
                anime_no: subscription.anissia_anime_no,
                subject: animes
                    .get(&subscription.anissia_anime_no)
                    .map(|a| a.subject.clone()),
                subtitles: subscription.subtitles.as_str(),
                creator: subscription.creator.clone(),
            })
        })
        .collect();
    // Stable: rules of one season keep the channel and rule order.
    subscriptions.sort_by_key(|s| s.season);
    let korean_title = subscriptions.iter().find_map(|s| s.subject.clone());

    let (links, first) = work_infos(&state, &work.id, &numbers).await?;
    let native_title = first
        .and_then(|first| links.get(&first))
        .and_then(|link| link.entries.first())
        .and_then(|entry| entry.native.clone());
    let seasons: Vec<SeasonView> = work
        .seasons
        .into_iter()
        .map(|season| {
            let link = &links[&season.number];
            let previous = season
                .number
                .checked_sub(1)
                .filter(|p| *p >= 1)
                .and_then(|p| links.get(&p))
                // Only a season the work has can be followed from.
                .filter(|_| numbers.contains(&(season.number - 1)));
            let air_times = trss_library::seasons::combine::air_times(&link.entries);
            let info = season_view(link, previous, first);
            SeasonView {
                number: season.number,
                info,
                anissia: anissia_links
                    .remove(&season.number)
                    .expect("a link view is made for every season of the work"),
                episodes: season
                    .episodes
                    .into_iter()
                    .map(|episode| {
                        let air_at = episode
                            .number
                            .filter(|n| n.fract() == 0.0 && *n >= 1.0 && *n <= f64::from(u32::MAX))
                            .and_then(|n| air_times.get(&(n as u32)).copied());
                        EpisodeView {
                            air_at,
                            ..EpisodeView::from(episode)
                        }
                    })
                    .collect(),
            }
        })
        .collect();

    let folder_path = FsPath::new(&work.watch_folder_path)
        .join(&work.dir_name)
        .to_string_lossy()
        .into_owned();
    let revisions = state
        .revisions
        .in_work_folder(folder_path.clone())
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let failed: Vec<Revision> = revisions
        .iter()
        .filter(|row| row.is_failure())
        .cloned()
        .collect();
    let offers = retry_offers(&state, &failed).await?;
    let mut seasons = seasons;
    attach_revisions(&mut seasons, revisions, FsPath::new(&folder_path), offers);
    let stored = state
        .jobs
        .stored_only(&work.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    attach_stored(&mut seasons, stored);
    let files = state
        .jobs
        .work_files(&work.id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
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
        native_title,
        korean_title,
        subscriptions,
        seasons,
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
        cover_url,
        cover_pending,
        storage: StorageView::from(files),
    }))
}

#[derive(Serialize)]
struct Applying {
    job_id: String,
}

async fn apply_stored(
    State(state): State<AppState>,
    Path((id, stored_id)): Path<(String, String)>,
) -> Result<(StatusCode, Json<Applying>), ApiError> {
    use trss_jobs::place::records::StoredChoice;
    let now = super::commands_api::now_millis();
    match state
        .jobs
        .choose_stored(&id, &stored_id, now)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        StoredChoice::Queued(job_id) => {
            if let Some(path) = &state.worker_wake {
                trss_core::wake::wake_worker(path);
            }
            Ok((StatusCode::ACCEPTED, Json(Applying { job_id })))
        }
        StoredChoice::NotFound => Err(ApiError::not_found("이 보관본을 찾지 못했어요.")),
        StoredChoice::Refused(message) => Err(ApiError::Conflict {
            message: message.to_owned(),
            current: None,
        }),
    }
}

/// The files the dialog showed would go with the stored subtitle.
#[derive(Deserialize)]
struct CleanBody {
    assets: Vec<String>,
}

#[derive(Serialize)]
struct Cleaning {
    cleanup_id: String,
}

const BAD_BODY: &str = "요청 내용을 읽지 못했어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";
const FILES_CHANGED: &str = "정리할 파일이 바뀌었어요. 다시 확인해 주세요.";

async fn clean_stored(
    State(state): State<AppState>,
    Path((id, stored_id)): Path<(String, String)>,
    parsed: Result<Json<CleanBody>, JsonRejection>,
) -> Result<(StatusCode, Json<Cleaning>), ApiError> {
    let Json(body) = parsed.map_err(|_| ApiError::invalid(BAD_BODY))?;
    let now = super::commands_api::now_millis();
    match state
        .jobs
        .clean_stored(&id, &stored_id, body.assets, now)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?
    {
        cleanup::Asked::Asked(cleanup_id) => {
            if let Some(path) = &state.worker_wake {
                trss_core::wake::wake_worker(path);
            }
            Ok((StatusCode::ACCEPTED, Json(Cleaning { cleanup_id })))
        }
        cleanup::Asked::NotFound => Err(ApiError::not_found("이 보관본을 찾지 못했어요.")),
        cleanup::Asked::Refused(message) => Err(ApiError::Conflict {
            message: message.to_owned(),
            current: None,
        }),
        cleanup::Asked::Changed(entry) => Err(ApiError::Conflict {
            message: FILES_CHANGED.to_owned(),
            current: serde_json::to_value(CleanableView::from(*entry)).ok(),
        }),
    }
}

/// How many files of one kind a work keeps, and their length.
#[derive(Serialize)]
struct KindView {
    /// `subtitle`, `font`, `attachment` or `cover`.
    kind: &'static str,
    count: u64,
    size: u64,
}

#[derive(Serialize)]
struct WorkStorageView {
    id: String,
    name: String,
    total: u64,
    kinds: Vec<KindView>,
    cleanable: usize,
}

#[derive(Serialize)]
struct StorageList {
    works: Vec<WorkStorageView>,
}

/// The length of the cover image at `relative` in the app data folder: `0`
/// when nothing is there or it is no file.
async fn cover_size(state: &AppState, relative: &str) -> u64 {
    let Some(app) = state.artwork.app_data() else {
        return 0;
    };
    let path = relative
        .split('/')
        .fold(app.root().to_path_buf(), |path, part| path.join(part));
    match tokio::fs::symlink_metadata(&path).await {
        Ok(meta) if meta.is_file() => meta.len(),
        _ => 0,
    }
}

async fn storage(State(state): State<AppState>) -> Result<Json<StorageList>, ApiError> {
    let kept = state
        .jobs
        .storage()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let covers = state
        .artwork
        .store
        .image_ids()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let works = state
        .library
        .overview()
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    let mut listed = Vec::new();
    for work in works {
        let files = kept.iter().find(|k| k.work_id == work.id);
        let cover = match covers.contains_key(&work.id) {
            true => match state.artwork.store.selection(&work.id).await {
                Ok(selection) => selection.image.map(|image| image.relative_path),
                Err(e) => return Err(ApiError::Internal(e.to_string())),
            },
            false => None,
        };
        if files.is_none() && cover.is_none() {
            continue;
        }
        let mut kinds: Vec<KindView> = files
            .map(|f| {
                f.kinds
                    .iter()
                    .map(|k| KindView {
                        kind: k.kind,
                        count: k.count,
                        size: k.size,
                    })
                    .collect()
            })
            .unwrap_or_default();
        if let Some(relative) = cover {
            kinds.push(KindView {
                kind: "cover",
                count: 1,
                size: cover_size(&state, &relative).await,
            });
        }
        listed.push(WorkStorageView {
            id: work.id,
            name: work.dir_name,
            total: kinds.iter().map(|k| k.size).sum(),
            kinds,
            cleanable: files.map_or(0, |f| f.cleanable),
        });
    }
    listed.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.name.cmp(&b.name)));
    Ok(Json(StorageList { works: listed }))
}
