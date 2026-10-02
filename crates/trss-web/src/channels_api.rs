//! `/api/channels`: list, add, edit and delete RSS channels for the channels
//! tab of the collection screen.
//!
//! | call                                        | success                          |
//! | ------------------------------------------- | -------------------------------- |
//! | `GET /channels`                             | `200 { channels: [ChannelView] }`|
//! | `POST /channels`                            | `201 ChannelView`                |
//! | `GET /channels/{id}`                        | `200 ChannelView`                |
//! | `PUT /channels/{id}`                        | `200 ChannelView`                |
//! | `DELETE /channels/{id}?version=N&rules=M`   | `200 { removed_rules }`          |
//!
//! Failures use the shape in [`super::error`]. A version that is not the
//! stored one, or a rule count that no longer matches on delete, answers
//! `409` with the channel's current [`ChannelView`] as `current`.
//!
//! A channel has no folder of its own: torrents are saved under the app's
//! collect folder plus the rule's directory (`/api/settings/collection`), and a
//! request body that names `base_dir` is refused as an unknown field.
//!
//! # Secret values never leave the server
//!
//! A channel URL keeps its original query values in the database, but no
//! response carries them. [`ChannelView`] has only the masked URL (secret
//! values shown as `***`), an *edit URL* (secret values left blank, everything
//! else as stored) and, per query name, whether it is secret and whether a
//! value is stored. Request and error bodies are never echoed, so a secret a
//! client sends is not sent back either.
//!
//! # How an edit keeps secrets it never saw
//!
//! `PUT` carries a full URL. The client starts from `edit_url`, so secret
//! values are blank in it. The server merges by query name before saving:
//!
//! 1. For every query pair whose name is secret **in the stored channel** and
//!    whose submitted value is blank (or exactly `***`, the mask), the stored
//!    value is put back. Repeated names match by position: the second `t=` in
//!    the submitted URL takes the second `t=` of the stored one. A pair with a
//!    non-blank value replaces the stored value; that is how a secret is
//!    changed.
//! 2. Non-secret names are taken as submitted. A stored value that was shown
//!    to the client is the client's to change or blank.
//! 3. Which names are secret afterwards comes from the request's `secret` map
//!    (`name -> bool`) and the URL's query names: a name is secret unless the
//!    map says `false` for it. So a name that is new, or that the client did
//!    not mention, starts secret, and a name that left the URL is dropped from
//!    the flags.
//!
//! Rule 1 runs before rule 3, so clearing a name's secret flag while leaving
//! its value blank keeps the stored value and shows it from then on.
//! A secret cannot be emptied from the screen: to drop it, remove the query
//! pair from the URL. Because the merge only ever reads the stored channel at
//! the version the client sent (a different version is refused first), a
//! concurrent save cannot cause a secret from a newer state to be mixed into
//! the client's URL.

use std::collections::HashMap;

use axum::{
    extract::{
        rejection::{JsonRejection, QueryRejection},
        Path, Query, State,
    },
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{ApiError, AppState};
use trss_legacy::store::channels::{query_names, Channel, ChannelError, ChannelInput, MASK};

#[cfg(test)]
mod tests;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/channels", get(list_channels).post(create_channel))
        .route(
            "/channels/{id}",
            get(get_channel).put(update_channel).delete(delete_channel),
        )
}

// ---------------------------------------------------------------------------
// Response shapes
// ---------------------------------------------------------------------------

/// A channel as the screen sees it. There is deliberately no field with the
/// URL's secret values.
#[derive(Debug, Serialize)]
pub struct ChannelView {
    pub id: String,
    pub position: i64,
    /// Send back as `version` when saving or deleting.
    pub version: i64,
    /// The URL's host, the heading of a channel that has no `name`.
    pub host: String,
    /// The display name the user gave, `null` when left blank.
    pub name: Option<String>,
    /// The URL with secret values replaced by `***`; empty secrets stay empty.
    pub masked_url: String,
    /// The URL to start an edit from: secret values are blank.
    pub edit_url: String,
    /// The query names in order of appearance.
    pub query: Vec<QueryParamView>,
    pub excludes: Vec<String>,
    pub past_search: Option<String>,
    /// Rules of this channel, archived ones included. The delete confirmation
    /// names this number and the delete call must send it back.
    pub rule_count: usize,
}

#[derive(Debug, Serialize)]
pub struct QueryParamView {
    pub name: String,
    pub secret: bool,
    /// Whether a non-empty value is stored (a secret not entered yet is not).
    pub filled: bool,
}

fn view(channel: &Channel, rule_count: usize) -> ChannelView {
    let mut filled: HashMap<String, bool> = HashMap::new();
    rewrite_query(&channel.url, |name, _, raw| {
        *filled.entry(name.to_owned()).or_default() |= !raw.is_empty();
        None
    });
    ChannelView {
        id: channel.id.clone(),
        position: channel.position,
        version: channel.version,
        host: Url::parse(&channel.url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default(),
        name: channel.name.clone(),
        masked_url: channel.masked_url(),
        edit_url: rewrite_query(&channel.url, |name, _, _| {
            channel
                .secret_query
                .iter()
                .any(|s| s == name)
                .then(String::new)
        }),
        query: query_names(&channel.url)
            .into_iter()
            .map(|name| QueryParamView {
                secret: channel.secret_query.contains(&name),
                filled: filled.get(&name).copied().unwrap_or(false),
                name,
            })
            .collect(),
        excludes: channel.excludes.clone(),
        past_search: channel.past_search.clone(),
        rule_count,
    }
}

#[derive(Serialize)]
struct ChannelList {
    channels: Vec<ChannelView>,
}

#[derive(Serialize)]
struct Removed {
    removed_rules: usize,
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

// No `Debug`: these carry the URL with its secret values.

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBody {
    url: String,
    #[serde(default)]
    excludes: Vec<String>,
    /// `name -> is secret`; a name not listed here is secret.
    #[serde(default)]
    secret: HashMap<String, bool>,
    #[serde(default)]
    past_search: Option<String>,
    /// Optional display name; blank or absent means unnamed.
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateBody {
    /// The version the client saw.
    version: i64,
    url: String,
    #[serde(default)]
    excludes: Vec<String>,
    #[serde(default)]
    secret: HashMap<String, bool>,
    #[serde(default)]
    past_search: Option<String>,
    /// Optional display name; blank or absent means unnamed.
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
struct DeleteQuery {
    /// The version the client saw.
    version: i64,
    /// The rule count the client named in its confirmation.
    rules: usize,
}

/// The channel fields after trimming and checking, before secrets are merged.
struct Fields {
    url: String,
    excludes: Vec<String>,
    secret: HashMap<String, bool>,
    past_search: Option<String>,
    name: Option<String>,
}

/// Longest display name, in characters.
const NAME_MAX_CHARS: usize = 100;

const BAD_BODY: &str = "요청 내용을 읽지 못했어요. 화면을 새로고침한 뒤 다시 시도해 주세요.";

fn body<T>(parsed: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    // The rejection text can quote the offending value, so it is dropped.
    parsed
        .map(|Json(v)| v)
        .map_err(|_| ApiError::invalid(BAD_BODY))
}

impl Fields {
    fn check(
        url: String,
        excludes: Vec<String>,
        secret: HashMap<String, bool>,
        past_search: Option<String>,
        name: Option<String>,
    ) -> Result<Fields, ApiError> {
        let url = url.trim().to_owned();
        if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(ApiError::invalid(
                "RSS 주소에 공백이나 줄바꿈이 들어 있어요. 주소를 한 줄로 붙여 넣어 주세요.",
            ));
        }
        match Url::parse(&url) {
            Ok(parsed) if matches!(parsed.scheme(), "http" | "https") && parsed.host().is_some() => {}
            _ => {
                return Err(ApiError::invalid(
                    "RSS 주소가 올바르지 않아요. http:// 또는 https://로 시작하는 전체 주소를 넣어 주세요.",
                ))
            }
        }
        let excludes = excludes
            .into_iter()
            .map(|e| e.trim().to_owned())
            .filter(|e| !e.is_empty())
            .collect();
        let past_search = past_search
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        let name = name.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());
        if let Some(name) = &name {
            if name.chars().any(char::is_control) {
                return Err(ApiError::invalid("채널 이름은 한 줄로 입력해 주세요."));
            }
            if name.chars().count() > NAME_MAX_CHARS {
                return Err(ApiError::invalid(
                    "채널 이름이 너무 길어요. 100자 이하로 줄여 주세요.",
                ));
            }
        }
        Ok(Fields {
            url,
            excludes,
            secret,
            past_search,
            name,
        })
    }

    /// The store input for `url` (already merged); see the module docs, rule 3.
    fn into_input(self, url: String) -> ChannelInput {
        let secret_query = query_names(&url)
            .into_iter()
            .filter(|name| self.secret.get(name) != Some(&false))
            .collect();
        ChannelInput {
            url,
            excludes: self.excludes,
            secret_query,
            past_search: self.past_search,
            name: self.name,
        }
    }
}

// ---------------------------------------------------------------------------
// Query rewriting
// ---------------------------------------------------------------------------

/// Rebuilds `url` with some query values replaced, leaving every other byte as
/// it was. `f(name, n, raw_value)` is called for each `name=value` pair, where
/// `name` is the decoded query name and `n` counts that name's earlier pairs;
/// it returns the replacement raw value, or `None` to keep the pair.
fn rewrite_query(url: &str, mut f: impl FnMut(&str, usize, &str) -> Option<String>) -> String {
    let (rest, fragment) = match url.split_once('#') {
        Some((rest, fragment)) => (rest, Some(fragment)),
        None => (url, None),
    };
    let Some((base, query)) = rest.split_once('?') else {
        return url.to_owned();
    };

    let mut seen: HashMap<String, usize> = HashMap::new();
    let pairs: Vec<String> = query
        .split('&')
        .map(|pair| {
            let (raw_name, raw_value) = match pair.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (pair, None),
            };
            if raw_name.is_empty() && raw_value.is_none() {
                return pair.to_owned();
            }
            let name = url::form_urlencoded::parse(raw_name.as_bytes())
                .next()
                .map(|(name, _)| name.into_owned())
                .unwrap_or_default();
            let n = seen.entry(name.clone()).or_default();
            let index = *n;
            *n += 1;
            match raw_value.and_then(|v| f(&name, index, v)) {
                Some(value) => format!("{raw_name}={value}"),
                None => pair.to_owned(),
            }
        })
        .collect();

    let mut out = format!("{base}?{}", pairs.join("&"));
    if let Some(fragment) = fragment {
        out.push('#');
        out.push_str(fragment);
    }
    out
}

/// Module docs, rule 1: puts the stored value back for every secret pair of
/// the submitted URL that has a blank value (or the mask).
fn restore_secrets(submitted: &str, stored: &Channel) -> String {
    if stored.secret_query.is_empty() {
        return submitted.to_owned();
    }
    let mut stored_values: HashMap<(String, usize), String> = HashMap::new();
    rewrite_query(&stored.url, |name, n, raw| {
        if stored.secret_query.iter().any(|s| s == name) {
            stored_values.insert((name.to_owned(), n), raw.to_owned());
        }
        None
    });
    rewrite_query(submitted, |name, n, raw| {
        let blank = raw.is_empty() || raw == MASK;
        if blank && stored.secret_query.iter().any(|s| s == name) {
            Some(
                stored_values
                    .get(&(name.to_owned(), n))
                    .cloned()
                    .unwrap_or_default(),
            )
        } else {
            None
        }
    })
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Store errors, with the English `Invalid` reason replaced by a Korean one
/// (the checks in [`Fields::check`] make it unreachable in practice).
fn store_error(e: ChannelError) -> ApiError {
    match e {
        ChannelError::Invalid(_) => ApiError::invalid("입력한 값으로는 저장할 수 없어요."),
        e => e.into(),
    }
}

async fn load_view(state: &AppState, id: &str) -> Result<ChannelView, ApiError> {
    let channel = state
        .channels
        .get_channel(id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| ChannelError::NotFound {
            kind: "channel",
            id: id.to_owned(),
        })?;
    let rules = state.channels.list_rules(id).await.map_err(store_error)?;
    Ok(view(&channel, rules.len()))
}

/// 409 with the channel as it is now, or 404 if it is gone.
async fn conflict(state: &AppState, id: &str) -> ApiError {
    match load_view(state, id).await {
        Ok(current) => ApiError::conflict_with(current),
        Err(e) => e,
    }
}

async fn list_channels(State(state): State<AppState>) -> Result<Json<ChannelList>, ApiError> {
    let channels = state
        .channels
        .list_channels_with_rules()
        .await
        .map_err(store_error)?
        .iter()
        .map(|c| view(&c.channel, c.rules.len()))
        .collect();
    Ok(Json(ChannelList { channels }))
}

async fn get_channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ChannelView>, ApiError> {
    Ok(Json(load_view(&state, &id).await?))
}

async fn create_channel(
    State(state): State<AppState>,
    parsed: Result<Json<CreateBody>, JsonRejection>,
) -> Result<(StatusCode, Json<ChannelView>), ApiError> {
    let b = body(parsed)?;
    let fields = Fields::check(b.url, b.excludes, b.secret, b.past_search, b.name)?;
    let url = fields.url.clone();
    let created = state
        .channels
        .create_channel(fields.into_input(url))
        .await
        .map_err(store_error)?;
    Ok((StatusCode::CREATED, Json(view(&created, 0))))
}

async fn update_channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    parsed: Result<Json<UpdateBody>, JsonRejection>,
) -> Result<Json<ChannelView>, ApiError> {
    let b = body(parsed)?;
    let fields = Fields::check(b.url, b.excludes, b.secret, b.past_search, b.name)?;

    let stored = state
        .channels
        .get_channel(&id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| ChannelError::NotFound {
            kind: "channel",
            id: id.clone(),
        })?;
    // Merge only against the state the client saw; the store re-checks the
    // version atomically when writing.
    if stored.version != b.version {
        return Err(conflict(&state, &id).await);
    }
    let url = restore_secrets(&fields.url, &stored);

    match state
        .channels
        .update_channel(&id, b.version, fields.into_input(url))
        .await
    {
        Ok(_) => Ok(Json(load_view(&state, &id).await?)),
        Err(e) if e.is_conflict() => Err(conflict(&state, &id).await),
        Err(e) => Err(store_error(e)),
    }
}

async fn delete_channel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    parsed: Result<Query<DeleteQuery>, QueryRejection>,
) -> Result<Json<Removed>, ApiError> {
    let Query(q) = parsed.map_err(|_| ApiError::invalid(BAD_BODY))?;
    match state.channels.delete_channel(&id, q.version, q.rules).await {
        Ok(removed_rules) => Ok(Json(Removed { removed_rules })),
        Err(e) if e.is_conflict() => Err(conflict(&state, &id).await),
        Err(e) => Err(store_error(e)),
    }
}
