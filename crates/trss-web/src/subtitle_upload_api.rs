//! `POST /api/subtitle-jobs/upload`: the subtitles and fonts a person uploads
//! from the work detail (`docs/specs/subtitles.md`, 직접 찾기와 자막 올리기),
//! made into one job whose files are received and which waits for the worker
//! to unpack and analyse them for the person's 배치 확인; this wakes it
//! ([`trss_jobs::upload`]).
//!
//! The body is `multipart/form-data`, its parts in this order:
//!
//! | part        | content                                                     |
//! | ----------- | ----------------------------------------------------------- |
//! | `id`        | the ID the browser made for the action (up to 128 characters) |
//! | `work_id`   | the work                                                    |
//! | `season`    | the season's number                                         |
//! | `creator`   | one of the season's Anissia creators' source IDs; left out or empty: `제작자 알 수 없음` |
//! | `skipped`   | (any number) the name of a file the browser left out because its name says it is no subtitle or font; only the name is sent |
//! | `file`      | (any number) one file; its `filename` is its name, or a folder's relative path (`Show/01.ass`) |
//!
//! `id`, `work_id`, `season` and `creator` come before the first `file`, so a
//! request that names no such season is refused before any file is stored.
//!
//! - The server judges every file by its content, never its name or type
//!   ([`trss_subtitles::upload::judge`]): subtitles (ASS, SRT, SMI and the
//!   formats that are only stored), fonts and ZIPs are kept, anything else is
//!   dropped with its reason, and neither the files left out by the browser
//!   nor those dropped stop the rest from being kept. A ZIP stays whole in the
//!   package; what is inside waits for the package analysis.
//! - Files are streamed to the receive area's `.tmp/` folder and counted as
//!   they come, never held whole in memory. The upload is refused before more
//!   is stored when it has more than [`trss_jobs::upload::MAX_FILES`] files
//!   (or [`trss_jobs::upload::MAX_ENTRIES`] counting those left out), a file
//!   over [`trss_jobs::upload::MAX_FILE_BYTES`], or more than
//!   [`trss_jobs::upload::MAX_TOTAL_BYTES`] in all (a `Content-Length` past it
//!   is refused at once), and the answer says the limit. Reading the body may
//!   take [`trss_jobs::upload::BODY_TIMEOUT`] in all and may not stall for
//!   [`trss_jobs::upload::IDLE_TIMEOUT`], and after
//!   [`trss_jobs::upload::RATE_GRACE`] the bytes of every last such window
//!   must come to at least [`trss_jobs::upload::MIN_RATE`] a second (a sender
//!   that trickles bytes would hold a turn for the whole time, and bytes sent
//!   up front buy no time later). At most
//!   [`trss_jobs::upload::UPLOAD_SLOTS`] uploads are taken in at once; of
//!   the rest at most [`trss_jobs::upload::QUEUE_MAX`] wait for their turn,
//!   each for at most [`trss_jobs::upload::QUEUE_WAIT`], and another is told
//!   at once that the server is busy (`503`).
//! - The framing is bounded too. The multipart reader keeps a part's headers
//!   (and anything before the first part) whole in memory until they end, so
//!   [`Gauge`] counts the bytes the body has delivered and the bytes the
//!   handler has used, and the body fails when more than [`BACKLOG`] it
//!   delivered are not used yet (a header that never ends, a preamble of
//!   gigabytes). However the body is framed (a chunked one has no
//!   `Content-Length`), it fails once it delivered more than the total limit
//!   and [`FRAMING_ALLOWANCE`]. A part's name and a file's name are bounded.
//! - A refusal after the body started drains what is left of the body for a
//!   moment ([`DRAIN_BYTES`], [`DRAIN_TIME`]) before it answers, so the
//!   browser that is still sending reads the answer and not a reset
//!   connection; a body that is too big for that may still show as a network
//!   error, which the screen words as unconfirmed.
//! - A new job answers `202`:
//!   `{ "id", "kept": { "subtitles", "fonts", "archives" }, "dropped": [{ "name", "reason" }] }`.
//!   The same `id` with the same upload answers `200 { "id" }` and stores
//!   nothing more; another upload under the same `id` is a `409` with the job
//!   in `current`. An upload with nothing to keep makes no job and answers
//!   `400` that there is nothing to receive.

use std::{
    collections::VecDeque,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

use axum::{
    body::Body,
    extract::{
        multipart::{Field, Multipart, MultipartError},
        DefaultBodyLimit, FromRequest, Request, State,
    },
    http::{header, StatusCode},
    routing::post,
    Json, Router,
};
use bytes::Bytes;
use futures::Stream;
use serde_json::json;

use trss_jobs::{
    upload::{Dropped, Limits, UploadError, UploadRequest},
    Finished,
};

use super::{
    commands_api::now_millis,
    jobs_api::{linked_anime, season_link},
    ApiError, AppState,
};

#[cfg(test)]
mod tests;

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/subtitle-jobs/upload",
        post(upload).layer(DefaultBodyLimit::disable()),
    )
}

/// What a text part may be at most, in bytes.
const TEXT_MAX: usize = 4096;
/// The longest command ID.
pub(crate) const ID_MAX: usize = 128;
/// What the multipart framing and the text parts may add to the files'
/// bytes in a `Content-Length` that is believed to be within the limit.
const FRAMING_ALLOWANCE: u64 = 8 * 1024 * 1024;
/// How many bytes the body may have delivered that the handler has not used
/// yet. Part headers and the bytes before the first part are held whole by
/// the multipart reader until they end, so this bounds that memory.
/// How many buckets a rate window is counted in.
const BUCKETS: u32 = 30;

const BACKLOG: u64 = 256 * 1024;
/// The longest name of a part, and the longest file name (a folder's
/// relative path included).
const PART_NAME_MAX: usize = 64;
const FILE_NAME_MAX: usize = 4096;
/// How much of a refused body is read and thrown away, and for how long.
const DRAIN_BYTES: u64 = 8 * 1024 * 1024;
const DRAIN_TIME: Duration = Duration::from_secs(2);

fn internal(e: &dyn std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}

/// A size as the limits' sentences say it (`1 GB`, `200 MB`, the screen's own form).
fn size_text(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 30 && b % (1 << 30) == 0 => format!("{} GB", b >> 30),
        b if b >= 1 << 20 && b % (1 << 20) == 0 => format!("{} MB", b >> 20),
        b if b >= 1 << 10 && b % (1 << 10) == 0 => format!("{} KB", b >> 10),
        b => format!("{b}바이트"),
    }
}

/// A refusal for an upload that passes a limit, saying the limit.
fn upload_error(error: UploadError, limits: &Limits) -> ApiError {
    match error {
        UploadError::TooManyFiles(n) => ApiError::invalid(format!(
            "한 번에 파일 {n}개까지 올릴 수 있어요. 나눠서 올려 주세요."
        )),
        UploadError::TooManyEntries(n) => ApiError::invalid(format!(
            "한 번에 고를 수 있는 파일은 올리지 않는 파일까지 모두 {n}개예요. 나눠서 올려 주세요."
        )),
        UploadError::TotalTooLarge(n) => ApiError::invalid(format!(
            "한 번에 모두 합쳐 {}까지 올릴 수 있어요. 나눠서 올려 주세요.",
            size_text(n)
        )),
        UploadError::FileTooLarge(n) => ApiError::invalid(format!(
            "파일 하나는 {}까지 올릴 수 있어요.",
            size_text(n.min(limits.file_bytes))
        )),
        UploadError::NoFile => ApiError::invalid("올리는 내용의 순서가 맞지 않아요."),
        UploadError::Busy => busy(),
        UploadError::Io(e) => internal(&e),
        UploadError::Job(e) => internal(&e),
    }
}

fn unreadable() -> ApiError {
    ApiError::invalid("올리는 파일을 끝까지 받지 못했어요. 다시 올려 주세요.")
}

fn too_slow_rate(limits: &Limits) -> ApiError {
    ApiError::invalid(format!(
        "올리는 속도가 너무 느려서 멈췄어요. 평균 {} KB/초 이상이어야 해요. 연결을 확인하고 다시 올려 주세요.",
        (limits.min_rate >> 10).max(1)
    ))
}

fn stalled(limits: &Limits) -> ApiError {
    ApiError::invalid(format!(
        "{}초 동안 아무것도 오지 않아 올리기를 멈췄어요. 다시 올려 주세요.",
        limits.idle_timeout.as_secs().max(1)
    ))
}

fn too_slow(limits: &Limits) -> ApiError {
    let secs = limits.body_timeout.as_secs();
    let time = match secs >= 60 && secs.is_multiple_of(60) {
        true => format!("{}분", secs / 60),
        false => format!("{secs}초"),
    };
    ApiError::invalid(format!(
        "올리는 데 {time}을 넘게 걸려 멈췄어요. 파일을 나눠서 다시 올려 주세요."
    ))
}

/// Why the body failed, when the body itself refused to go on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Refusal {
    /// More delivered than used: a part header or a preamble that goes on.
    Backlog = 1,
    /// More delivered than the limits and the framing allow.
    Total = 2,
    /// Slower than the least average speed.
    Slow = 3,
}

/// What the body has delivered and what the handler has used, shared between
/// the body ([`Resting`]) and the handler ([`Reader`]); and why the body
/// stopped, when it did.
#[derive(Default)]
struct Gauge {
    delivered: AtomicU64,
    used: AtomicU64,
    refused: AtomicU8,
}

impl Gauge {
    fn used(&self, bytes: usize) {
        self.used.fetch_add(bytes as u64, Ordering::SeqCst);
    }

    /// The reader has what it was waiting for (a part's headers were read):
    /// what is buffered past them is at most the one chunk that held them.
    fn caught_up(&self) {
        self.used
            .store(self.delivered.load(Ordering::SeqCst), Ordering::SeqCst);
    }

    fn backlog(&self) -> u64 {
        self.delivered
            .load(Ordering::SeqCst)
            .saturating_sub(self.used.load(Ordering::SeqCst))
    }

    fn refuse(&self, why: Refusal) {
        self.refused.store(why as u8, Ordering::SeqCst);
    }

    fn refusal(&self) -> Option<Refusal> {
        match self.refused.load(Ordering::SeqCst) {
            1 => Some(Refusal::Backlog),
            2 => Some(Refusal::Total),
            3 => Some(Refusal::Slow),
            _ => None,
        }
    }
}

/// Reads the multipart body for the handler: every wait is bounded, every
/// byte used is counted, and a failure says why.
struct Reader<'a> {
    gauge: &'a Gauge,
    limits: &'a Limits,
}

impl Reader<'_> {
    fn fail(&self, _: MultipartError) -> ApiError {
        match self.gauge.refusal() {
            Some(Refusal::Backlog) => ApiError::invalid(
                "올리는 내용의 머리글이 너무 길거나 형식이 맞지 않아요. 화면을 새로고침한 뒤 다시 올려 주세요.",
            ),
            Some(Refusal::Total) => {
                upload_error(UploadError::TotalTooLarge(self.limits.total_bytes), self.limits)
            }
            Some(Refusal::Slow) => too_slow_rate(self.limits),
            None => unreadable(),
        }
    }

    async fn next_field<'m>(
        &self,
        multipart: &'m mut Multipart,
    ) -> Result<Option<Field<'m>>, ApiError> {
        let field = tokio::time::timeout(self.limits.idle_timeout, multipart.next_field())
            .await
            .map_err(|_| stalled(self.limits))?
            .map_err(|e| self.fail(e))?;
        self.gauge.caught_up();
        Ok(field)
    }

    async fn chunk(&self, field: &mut Field<'_>) -> Result<Option<Bytes>, ApiError> {
        let chunk = tokio::time::timeout(self.limits.idle_timeout, field.chunk())
            .await
            .map_err(|_| stalled(self.limits))?
            .map_err(|e| self.fail(e))?;
        if let Some(chunk) = &chunk {
            self.gauge.used(chunk.len());
        }
        Ok(chunk)
    }

    /// A text part, at most [`TEXT_MAX`] bytes.
    async fn text(&self, field: &mut Field<'_>) -> Result<String, ApiError> {
        let mut bytes = Vec::new();
        while let Some(chunk) = self.chunk(field).await? {
            if bytes.len() + chunk.len() > TEXT_MAX {
                return Err(ApiError::invalid("올리는 내용의 글이 너무 길어요."));
            }
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes).map_err(|_| ApiError::invalid("올리는 내용의 글을 읽지 못했어요."))
    }
}

/// The upload's own parts, as they come before the first file.
#[derive(Default)]
struct Head {
    id: Option<String>,
    work_id: Option<String>,
    season: Option<String>,
    creator: Option<String>,
}

/// What the upload is for, once its season and creator are checked.
struct Target {
    request: UploadRequest,
}

async fn target_of(state: &AppState, head: Head) -> Result<Target, ApiError> {
    let id = head.id.map(|s| s.trim().to_owned()).unwrap_or_default();
    if id.is_empty() {
        return Err(ApiError::invalid("요청 ID가 비어 있어요."));
    }
    if id.chars().count() > ID_MAX {
        return Err(ApiError::invalid("요청 ID가 너무 길어요."));
    }
    // The app's own receipts and revisions take these IDs.
    if trss_jobs::is_app_command(&id) {
        return Err(ApiError::invalid("이 요청 ID는 쓸 수 없어요."));
    }
    let work_id = head.work_id.unwrap_or_default();
    let season: u32 = head
        .season
        .as_deref()
        .and_then(|s| s.trim().parse().ok())
        .ok_or_else(|| ApiError::invalid("시즌 번호가 올바르지 않아요."))?;
    if work_id.is_empty() {
        return Err(ApiError::invalid("작품이 빠졌어요."));
    }
    let link = season_link(state, &work_id, season).await?;
    let wanted = head.creator.filter(|c| !c.is_empty());
    let (source_id, creator) = match wanted {
        None => (None, None),
        Some(source_id) => {
            let anime_no = linked_anime(
                &link,
                "이 시즌은 Anissia 작품에 연결돼 있지 않아서 제작자를 고를 수 없어요.",
            )?;
            match state
                .anissia_store
                .newest_of_creator(anime_no, source_id.clone())
                .await
                .map_err(|e| internal(&e))?
            {
                Some(found) => (Some(source_id), Some(found.creator)),
                None => {
                    return Err(ApiError::invalid(
                        "고른 제작자가 이 시즌의 제작자가 아니에요. 화면을 새로고침해 주세요.",
                    ))
                }
            }
        }
    };
    Ok(Target {
        request: UploadRequest {
            command_id: id,
            work_id,
            season: i64::from(season),
            anime_no: link.anime_no,
            source_id,
            creator,
        },
    })
}

/// The body the multipart reader reads. It rests after every chunk it gives:
/// it answers `Pending` once (and wakes itself) before it reads the next. The
/// multipart reader keeps reading its stream for as long as the stream is
/// ready, so a sender faster than the reader would be buffered whole before a
/// byte reached the limits; the rest hands the chunk to the reader first.
///
/// It also holds the body to the [`Gauge`]: it fails, once, when a chunk
/// comes while more than [`BACKLOG`] bytes it delivered are not used, when it
/// delivered more than the limits allow, or when the last
/// [`Limits::rate_grace`] brought less than the least speed over that time. The source is shared so the handler can drain what is left
/// of it after a refusal.
struct Resting<S> {
    inner: Arc<Mutex<S>>,
    rest: bool,
    gauge: Arc<Gauge>,
    limits: Limits,
    started: Option<Instant>,
    /// The bytes of the last window, in buckets of a thirtieth of it:
    /// `(bucket number since the start, bytes)`, at most 31 of them.
    recent: VecDeque<(u64, u64)>,
    failed: bool,
}

impl<S> Resting<S> {
    fn new(inner: Arc<Mutex<S>>, gauge: Arc<Gauge>, limits: Limits) -> Resting<S> {
        Resting {
            inner,
            rest: false,
            gauge,
            limits,
            started: None,
            recent: VecDeque::new(),
            failed: false,
        }
    }

    /// Whether the body must stop with the next chunk of `len` bytes.
    fn judge(&mut self, len: u64) -> Option<Refusal> {
        let started = *self.started.get_or_insert_with(Instant::now);
        if self.gauge.backlog() > BACKLOG {
            return Some(Refusal::Backlog);
        }
        let delivered = self.gauge.delivered.fetch_add(len, Ordering::SeqCst) + len;
        if delivered > self.limits.total_bytes.saturating_add(FRAMING_ALLOWANCE) {
            return Some(Refusal::Total);
        }
        let elapsed = started.elapsed();
        let window = self.limits.rate_grace;
        let width = (window / BUCKETS).max(Duration::from_millis(1));
        let bucket = (elapsed.as_nanos() / width.as_nanos()) as u64;
        match self.recent.back_mut() {
            Some((at, bytes)) if *at == bucket => *bytes += len,
            _ => self.recent.push_back((bucket, len)),
        }
        while self
            .recent
            .front()
            .is_some_and(|(at, _)| at + (BUCKETS as u64) < bucket)
        {
            self.recent.pop_front();
        }
        if elapsed > window {
            let in_window: u64 = self.recent.iter().map(|(_, bytes)| bytes).sum();
            if (in_window as f64) < self.limits.min_rate as f64 * window.as_secs_f64() {
                return Some(Refusal::Slow);
            }
        }
        None
    }
}

impl<S> Stream for Resting<S>
where
    S: Stream<Item = Result<Bytes, axum::Error>> + Unpin,
{
    type Item = Result<Bytes, axum::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.failed {
            return Poll::Ready(None);
        }
        self.started.get_or_insert_with(Instant::now);
        if self.rest {
            self.rest = false;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        let polled = {
            let mut inner = self.inner.lock().expect("the body is not poisoned");
            Pin::new(&mut *inner).poll_next(cx)
        };
        match polled {
            Poll::Ready(Some(Ok(chunk))) => {
                if let Some(why) = self.judge(chunk.len() as u64) {
                    self.gauge.refuse(why);
                    self.failed = true;
                    return Poll::Ready(Some(Err(axum::Error::new(std::io::Error::other(
                        "the upload's body was refused",
                    )))));
                }
                self.rest = true;
                Poll::Ready(Some(Ok(chunk)))
            }
            other => other,
        }
    }
}

/// Reads and drops what is left of a refused body, for a moment, so the
/// sender is still being read when the answer goes out.
async fn drain<S>(source: &Mutex<S>)
where
    S: Stream<Item = Result<Bytes, axum::Error>> + Unpin,
{
    let mut left = DRAIN_BYTES;
    let _ = tokio::time::timeout(DRAIN_TIME, async {
        while left > 0 {
            let next = futures::future::poll_fn(|cx| {
                let mut inner = source.lock().expect("the body is not poisoned");
                Pin::new(&mut *inner).poll_next(cx)
            })
            .await;
            match next {
                Some(Ok(chunk)) => left = left.saturating_sub(chunk.len() as u64),
                _ => break,
            }
        }
    })
    .await;
}

async fn upload(
    State(state): State<AppState>,
    request: Request,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let (parts, body) = request.into_parts();
    let declared = parts
        .headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    let limits = state.uploads.limits();
    let gauge = Arc::new(Gauge::default());
    let source = Arc::new(Mutex::new(body.into_data_stream()));
    let body = Body::from_stream(Resting::new(
        Arc::clone(&source),
        Arc::clone(&gauge),
        limits,
    ));
    let outcome = receive(
        &state,
        Request::from_parts(parts, body),
        declared,
        &limits,
        &gauge,
    )
    .await;
    // A body that broke the rules is not read on; one that was refused for
    // what it holds is drained a little, so its sender can read the answer.
    if outcome.is_err() && gauge.refusal().is_none() {
        drain(&source).await;
    }
    outcome
}

async fn receive(
    state: &AppState,
    request: Request,
    declared: Option<u64>,
    limits: &Limits,
    gauge: &Gauge,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let mut multipart = Multipart::from_request(request, state).await.map_err(|_| {
        ApiError::invalid("올리는 내용을 읽지 못했어요. 화면을 새로고침한 뒤 다시 올려 주세요.")
    })?;
    let reader = Reader { gauge, limits };
    // Past the limit by its own length: refused before anything is stored.
    if declared.is_some_and(|n| n > limits.total_bytes.saturating_add(FRAMING_ALLOWANCE)) {
        return Err(upload_error(
            UploadError::TotalTooLarge(limits.total_bytes),
            limits,
        ));
    }

    // The turn comes first, so only so many uploads are taken in at once.
    let mut staging = match state.uploads.begin().await {
        Ok(staging) => staging,
        Err(UploadError::Busy) => return Err(busy()),
        Err(e) => return Err(upload_error(e, limits)),
    };

    let target = tokio::time::timeout(limits.body_timeout, async {
        let mut head = Head::default();
        let mut target: Option<Target> = None;
        while let Some(mut field) = reader.next_field(&mut multipart).await? {
            let name = field.name().unwrap_or_default();
            if name.len() > PART_NAME_MAX {
                return Err(ApiError::invalid("올리는 내용에 알 수 없는 항목이 있어요."));
            }
            let name = name.to_owned();
            match name.as_str() {
                "id" | "work_id" | "season" | "creator" => {
                    if target.is_some() {
                        return Err(ApiError::invalid("올리는 내용의 순서가 맞지 않아요."));
                    }
                    let text = reader.text(&mut field).await?;
                    match name.as_str() {
                        "id" => head.id = Some(text),
                        "work_id" => head.work_id = Some(text),
                        "season" => head.season = Some(text),
                        _ => head.creator = Some(text),
                    }
                }
                "skipped" => {
                    let text = reader.text(&mut field).await?;
                    staging.skip(&text).map_err(|e| upload_error(e, limits))?;
                }
                "file" => {
                    if target.is_none() {
                        // The season and the creator are checked before the
                        // first byte of a file is stored.
                        target = Some(target_of(state, std::mem::take(&mut head)).await?);
                    }
                    let file_name = field.file_name().unwrap_or_default();
                    if file_name.len() > FILE_NAME_MAX {
                        return Err(ApiError::invalid(format!(
                            "파일 이름이 너무 길어요. {FILE_NAME_MAX}바이트까지예요."
                        )));
                    }
                    let file_name = file_name.to_owned();
                    staging
                        .start(&file_name)
                        .await
                        .map_err(|e| upload_error(e, limits))?;
                    while let Some(chunk) = reader.chunk(&mut field).await? {
                        staging
                            .write(&chunk)
                            .await
                            .map_err(|e| upload_error(e, limits))?;
                    }
                    staging.end().await.map_err(|e| upload_error(e, limits))?;
                }
                _ => return Err(ApiError::invalid("올리는 내용에 알 수 없는 항목이 있어요.")),
            }
        }
        match target {
            Some(target) => Ok(target),
            // No file at all: the season and the creator are still checked.
            None => target_of(state, head).await,
        }
    })
    .await
    .map_err(|_| too_slow(limits))??;

    let finished = state
        .uploads
        .finish(staging, target.request, now_millis())
        .await
        .map_err(|e| upload_error(e, limits))?;
    match finished {
        Finished::Created {
            job_id,
            counts,
            dropped,
        } => {
            // What it kept is the worker's to unpack and analyse.
            if let Some(path) = &state.worker_wake {
                trss_core::wake::wake_worker(path);
            }
            Ok((
            StatusCode::ACCEPTED,
            Json(json!({
                "id": job_id,
                "kept": {
                    "subtitles": counts.subtitles,
                    "fonts": counts.fonts,
                    "archives": counts.archives,
                },
                "dropped": dropped_view(&dropped),
            })),
        ))
        }
        Finished::Existing(id) => Ok((StatusCode::OK, Json(json!({ "id": id })))),
        Finished::Mismatch(id) => Err(ApiError::Conflict {
            message: "같은 요청 ID로 다른 파일을 올린 적이 있어요. 화면을 새로고침해 주세요."
                .to_owned(),
            current: Some(json!({ "id": id })),
        }),
        Finished::Nothing { dropped } => Err(ApiError::invalid(match dropped.len() {
            0 => "올린 파일이 없어요. 받을 자막이나 폰트가 없어서 작업을 만들지 않았어요."
                .to_owned(),
            n => format!(
                "받을 자막이나 폰트가 없어요. 올린 파일 {n}개는 모두 자막이나 폰트가 아니라서 작업을 만들지 않았어요."
            ),
        })),
    }
}

/// The server has no turn to give: nothing was stored.
fn busy() -> ApiError {
    ApiError::Busy("지금 다른 자막 올리기를 받고 있어요. 잠시 뒤 다시 올려 주세요.".to_owned())
}

fn dropped_view(dropped: &[Dropped]) -> Vec<serde_json::Value> {
    dropped
        .iter()
        .map(|d| json!({ "name": d.name, "reason": d.reason }))
        .collect()
}
