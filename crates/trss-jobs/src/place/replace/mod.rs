//! Replacing the subtitle an episode has, once a person approves it
//! (`docs/specs/subtitles.md`, 교체 비교와 승인 and 승인 증거와 반영 직전
//! 검사; `docs/specs/jobs.md`, 체크포인트와 중단 복구).
//!
//! # Plan
//!
//! A row to apply whose episode has a subtitle beside its one video (one the
//! library recorded, a file under the video's stem with a subtitle extension,
//! or a copy the app applied, each while it is on the disk) is not applied: a
//! plan is made ([`records::Plan`]) and the job waits for a person (`교체 승인`).
//! The plan's evidence never changes: the target and what put the row there,
//! the video's object, length and change time, the stored subtitle's asset as
//! recorded, and what happens to each path: the new copy's path (the video's
//! stem and the format's extension) is `replace`d when a file is there (else
//! one whose name differs in case only) and `add`ed when none is; a copy the
//! app applied at another path, while its bytes are the ones applied, is
//! `remove`d; any other subtitle stays (`keep`). Not planned, and stored
//! only: an episode beside whose video the new bytes are already, a row whose
//! source has a newer revision of the episode stored (that one's job compares
//! it), an episode whose paths another effect under way is changing, and one
//! with two files to take off whose names differ in case only.
//!
//! A row a person chose to apply (`chosen`, [`records::choose_stored`](place_records::choose_stored))
//! is planned the same way, with two differences. The newer revision of its
//! source does not keep it stored only, here nor in [`Placer::changed`]: the
//! person chose this copy over it, which is how a past revision is restored.
//! And a row chosen to `add` the creator's other format takes nothing off:
//! every other subtitle beside the video stays (`keep`), a file at the new
//! copy's path is `replace`d, and when none is (nor one whose name differs in
//! case only) the copy is applied at once as for an episode with no subtitle,
//! with no plan and no approval.
//!
//! The plan keeps what differs between its current subtitle ([`records::Plan::current`])
//! and the new one, made with it ([`diff`]): the worker reads both files, never
//! a web request.
//!
//! The person keeps the current subtitle (the row is stored only) or
//! approves the plan ([`records::decide`]); either puts the job waiting for
//! it back in line, and a job running meanwhile goes back in line as its run
//! ends ([`crate::store::JobStore::settle`]).
//! Each run looks at an open plan again ([`Placer::changed`]): one whose
//! evidence no longer holds goes `stale` and the row's next plan is made, so
//! the person compares again.
//!
//! # Carrying out
//!
//! An approved plan ([`Placer::carry_out`]) is checked again first: a change
//! makes it stale, and the next plan is made. Then:
//!
//! 1. Its effects are claimed in one transaction (`intended`): the new copy's
//!    apply and a removal for each path it takes off. Another effect under
//!    way on one of those paths stops it: the plan goes stale and the row is
//!    stored only.
//! 2. The new copy is written to its temporary file (`prepared`); each file
//!    to take off is copied to its protective copy in `.trss/tmp/` and read
//!    back as the plan saw it (`prepared`); a file the app did not manage is
//!    imported from that copy as the stored subtitle of the creator nobody
//!    named. Nothing beside the video changed yet: a failure (no space) fails
//!    the plan, a change found makes it stale.
//! 3. The plan is checked again. The file at the new copy's path is renamed
//!    aside to `.trss/tmp/<ID>.aside`, replacing nothing, and must be the
//!    plan's object and bytes there (`set_aside`); one that is not is put
//!    back and the plan goes stale.
//! 4. The new copy is published by a rename that replaces nothing and
//!    recorded applied. A record the database refuses holds the plan, and
//!    nothing more is done to it.
//! 5. Each earlier applied copy is set aside the same way and recorded
//!    removed. One that cannot be holds the plan, the new copy applied: the
//!    removal is not taken as done, and nothing is rolled back.
//! 6. The plan is `done`; then the protective copies and the files set aside
//!    go, and the removals are `done`.
//!
//! # Restart
//!
//! An approved plan's unfinished effects are compared with the disk before
//! anything else of the row ([`Placer::recover_plan`]):
//!
//! | Effects | On disk | Then |
//! | --- | --- | --- |
//! | a removal `prepared` | the file at its path, nothing aside | not set aside yet |
//! | a removal `prepared` | nothing at its path, the plan's object aside | set aside: recorded so |
//! | the new copy `prepared` | its temporary file gone, its object at its path | published: recorded applied |
//! | an import `prepared` | its object at its target | published: recorded |
//! | nothing set aside, the new copy not published | | the temporary files go, the effects are abandoned and the plan is carried out anew, from its check |
//! | something set aside or the new copy applied | | carried on from there (steps 3 to 6) |
//! | a `done` plan | | step 6 |
//! | anything else | | the plan, its effects and its row are held; every file stays |

pub mod diff;
pub mod lines;
pub mod records;

use std::{
    io,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use trss_core::files::rename_noreplace;

use super::{
    blocking, files,
    files::{Copied, Published},
    joined,
    package::extension,
    records::{self as place_records, Effect, JobFacts, NewApplied, Placed, PlanRow},
    row_label, video_parts, Choice, Placer, SUBTITLE_EXTENSIONS,
};
use crate::{
    area::{object_of, read_facts, sync_dir},
    model::{
        AssetKind, Chosen, EffectKind, EffectState, Outcome, PathAction, PlanAction, PlanState,
        SubtitleFormat,
    },
    store::JobError,
};
use records::{Claimed, Compared, Comparison, FileSeen, Imported, Plan, PlanPath, VideoSeen};

/// The reason a plan goes stale for a newer revision (`새 수정본 발견`).
pub const NEW_REVISION: &str = "같은 출처의 새 수정본이 들어왔어요";
/// A row whose plan waits for the person.
pub const AWAITING_APPROVAL: &str = "이 회차에 자막이 있어 교체 승인을 기다려요";

/// A subtitle file beside the video.
#[derive(Debug, Clone)]
struct Present {
    path: String,
    file: FileSeen,
    applied_id: Option<String>,
}

/// What a look at the subtitles beside a video found.
enum Scan {
    Files(Vec<Present>),
    Unreadable(String),
}

/// What came of setting a file aside.
enum Aside {
    Done,
    /// Nothing changed beside the video (the file was put back, or was not
    /// there): why.
    Untouched(String),
    /// The rename failed: nothing moved.
    Failed(io::Error),
    /// What is on disk is not known: why.
    Unsure(String),
}

/// The change time of a file, in nanoseconds since the epoch.
fn mtime_of(meta: &std::fs::Metadata) -> io::Result<i64> {
    let at = meta.modified()?;
    let since = at
        .duration_since(UNIX_EPOCH)
        .map_err(|_| io::Error::other("변경 시각이 1970년보다 앞이에요"))?;
    Ok(since.as_nanos() as i64)
}

/// A subtitle file as a plan sees it; `None` when nothing is at `path`.
fn seen_file(path: &Path) -> io::Result<Option<FileSeen>> {
    let meta = match std::fs::symlink_metadata(path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
        Ok(meta) => meta,
    };
    let (size, sha256, object) = read_facts(path)?;
    let lines = match extension(&path.to_string_lossy()) {
        Some(ext) => std::fs::read(path)
            .ok()
            .and_then(|bytes| lines::count(&bytes, &ext)),
        None => None,
    };
    Ok(Some(FileSeen {
        size,
        sha256,
        object,
        mtime: mtime_of(&meta)?,
        lines,
    }))
}

/// The video at `path` (`relative` to the work folder): a regular file, its
/// object, length and change time.
fn seen_video(path: &Path, relative: &str) -> io::Result<VideoSeen> {
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() {
        return Err(io::Error::other("영상이 일반 파일이 아니에요"));
    }
    Ok(VideoSeen {
        path: relative.to_owned(),
        object: object_of(&meta),
        size: meta.len(),
        mtime: mtime_of(&meta)?,
    })
}

/// The same file as the plan saw: its object and bytes.
fn same_file(found: &Option<(u64, String, String)>, seen: &FileSeen) -> bool {
    found
        .as_ref()
        .is_some_and(|(n, s, o)| *n == seen.size && *s == seen.sha256 && *o == seen.object)
}

/// The name `path` ends in.
fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

impl Placer {
    /// The subtitles beside `video` (relative to `folder`): the library's of
    /// the episode, the files under the video's stem with a subtitle
    /// extension, and the copies the app applied, while they are there.
    async fn scan(
        &self,
        facts: &JobFacts,
        folder: &str,
        episode: i64,
        video: &str,
    ) -> Result<Scan, JobError> {
        let work = facts.work_id.clone().unwrap_or_default();
        let season = facts.season.unwrap_or(0);
        let (library, applied) = self
            .read(move |c| {
                Ok((
                    place_records::episode_files(c, &work, season, episode)?.1,
                    records::applied_on(c, &work, season, episode)?,
                ))
            })
            .await?;
        let (dir, stem) = video_parts(video);
        let dir_path = files::within(Path::new(folder), dir);
        let names = match blocking(move || files::names_in(&dir_path)).await {
            Ok(mut names) => {
                names.sort();
                names
            }
            Err(err) => {
                return Ok(Scan::Unreadable(format!(
                    "영상 폴더를 읽지 못했어요: {err}"
                )))
            }
        };
        let prefix = format!("{}.", stem.to_lowercase());
        let mut paths: Vec<String> = Vec::new();
        let beside = names.iter().filter(|n| {
            let lower = n.to_lowercase();
            lower.starts_with(&prefix)
                && extension(&lower).is_some_and(|e| SUBTITLE_EXTENSIONS.contains(&e.as_str()))
        });
        let others = library
            .into_iter()
            .chain(applied.iter().map(|(_, path, _)| path.clone()));
        for path in beside.map(|n| joined(dir, n)).chain(others) {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        let mut present = Vec::new();
        for path in paths {
            let at = files::within(Path::new(folder), &path);
            match blocking(move || seen_file(&at)).await {
                Ok(None) => {}
                Ok(Some(file)) => {
                    let applied_id = applied
                        .iter()
                        .find(|(_, p, sha)| *p == path && *sha == file.sha256)
                        .map(|(id, ..)| id.clone());
                    present.push(Present {
                        path,
                        file,
                        applied_id,
                    });
                }
                Err(err) => {
                    return Ok(Scan::Unreadable(format!(
                        "기존 자막을 읽지 못했어요 ({path}): {err}"
                    )))
                }
            }
        }
        Ok(Scan::Files(present))
    }

    /// The row's plan to decide or carry out, if it has one.
    pub(super) async fn live_plan(&self, row: &PlanRow) -> Result<Option<Plan>, JobError> {
        let (job, position) = (row.job_id.clone(), row.position);
        self.read(move |c| records::live_plan(c, &job, position))
            .await
    }

    /// Goes on with the row's live plan: whether the row is settled for this
    /// run (`false`: the plan went stale and the row is looked at anew).
    pub(super) async fn go_on(
        &self,
        facts: &JobFacts,
        row: &PlanRow,
        plan: Plan,
    ) -> Result<bool, JobError> {
        match plan.state {
            PlanState::Open => match self.changed(facts, &plan).await? {
                None => Ok(true),
                // A plan decided meanwhile is the next run's.
                Some(reason) => Ok(!self.stale(row, &plan, &reason).await?),
            },
            PlanState::Approved => self.carry_out(facts, row, plan).await,
            _ => Ok(true),
        }
    }

    /// Plans the replacement of the subtitle the episode of `video` has:
    /// whether the row is settled (`false`: the episode has none, so the row
    /// is applied as a first copy).
    pub(super) async fn compare(
        &self,
        facts: &JobFacts,
        folder: &str,
        row: &PlanRow,
        video: &str,
    ) -> Result<bool, JobError> {
        let (Some(placed), Some(stored_id), Some(ext)) = (
            row.placed.as_ref(),
            row.stored_id.clone(),
            row.format.and_then(SubtitleFormat::extension),
        ) else {
            return Ok(false);
        };
        let label = row_label(row);
        let present = match self.scan(facts, folder, placed.episode, video).await? {
            Scan::Unreadable(reason) => {
                self.settle_row(row, Outcome::Held, reason.clone()).await?;
                self.event(&row.job_id, format!("{label}: 보류했어요"), Some(reason))
                    .await?;
                return Ok(true);
            }
            Scan::Files(present) if present.is_empty() => return Ok(false),
            Scan::Files(present) => present,
        };
        let id = stored_id.clone();
        let Some(asset) = self
            .read(move |c| place_records::stored_asset(c, &id))
            .await?
        else {
            self.fail_row(row, "보관본의 기록을 찾지 못했어요".to_owned())
                .await?;
            return Ok(true);
        };
        let stored_only = |note: &str| (note.to_owned(), present[0].path.clone());
        // A copy a person chose is applied over the newer revision of its
        // source (a past revision restored): that one's job compares it.
        let id = stored_id.clone();
        let newer = row.chosen.is_none()
            && self
                .read(move |c| records::newer_revision(c, &id))
                .await?
                .is_some();
        let (dir, stem) = video_parts(video);
        let target = joined(dir, &format!("{stem}.{ext}"));
        // The file of the target's own name, else one that differs in case
        // only: the new copy is published under the target's name once that
        // file is set aside, so another variant there would take the name.
        let at_target = present.iter().position(|p| p.path == target).or_else(|| {
            present
                .iter()
                .position(|p| p.path.to_lowercase() == target.to_lowercase())
        });
        let settled = if present.iter().any(|p| p.file.sha256 == asset.sha256) {
            Some(stored_only(
                "이 회차에 같은 자막이 이미 있어 그대로 두고 보관만 했어요",
            ))
        } else if newer {
            Some(stored_only(
                "같은 출처의 새 수정본이 들어와 이 자막은 보관만 했어요",
            ))
        } else {
            let f = folder.to_owned();
            let busy = self
                .read(move |c| place_records::busy_targets(c, &f, None))
                .await?;
            let changing = std::iter::once(&target)
                .chain(present.iter().map(|p| &p.path))
                .find(|p| busy.contains(&p.to_lowercase()));
            changing.map(|path| {
                (
                    "다른 작업이 이 회차의 자막을 바꾸는 중이라 그대로 두고 보관만 했어요"
                        .to_owned(),
                    path.clone(),
                )
            })
        };
        if let Some((note, detail)) = settled {
            self.settle_row(row, Outcome::Existing, note.clone())
                .await?;
            self.event(&row.job_id, format!("{label}: {note}"), Some(detail))
                .await?;
            return Ok(true);
        }
        // An added format with nothing at its path is applied as a first
        // copy is, the subtitles beside the video left as they are.
        if row.chosen == Some(Chosen::Add) && at_target.is_none() {
            return Ok(false);
        }

        let video_at = files::within(Path::new(folder), video);
        let relative = video.to_owned();
        let video_seen = match blocking(move || seen_video(&video_at, &relative)).await {
            Ok(seen) => seen,
            Err(err) => {
                let reason = format!("영상을 확인하지 못했어요: {err}");
                self.settle_row(row, Outcome::Held, reason.clone()).await?;
                self.event(&row.job_id, format!("{label}: 보류했어요"), Some(reason))
                    .await?;
                return Ok(true);
            }
        };
        let asset_at = files::within(Path::new(folder), &asset.relative_path);
        let read = blocking(move || {
            let facts = read_facts(&asset_at)?;
            let bytes = std::fs::read(&asset_at)?;
            Ok::<_, io::Error>((facts, bytes))
        })
        .await;
        let (asset_lines, asset_bytes) = match read {
            Ok(((n, sha, _), bytes)) if n == asset.size && sha == asset.sha256 => {
                (lines::count(&bytes, ext), bytes)
            }
            _ => {
                let reason = "보관본이 기록과 달라 비교하지 못했어요".to_owned();
                self.settle_row(row, Outcome::Held, reason.clone()).await?;
                self.event(
                    &row.job_id,
                    format!("{label}: 보류했어요"),
                    Some(format!("{reason} ({})", asset.relative_path)),
                )
                .await?;
                return Ok(true);
            }
        };

        let mut paths = vec![match at_target {
            Some(i) => PlanPath {
                path: present[i].path.clone(),
                action: PathAction::Replace,
                file: Some(present[i].file.clone()),
                applied_id: present[i].applied_id.clone(),
            },
            None => PlanPath {
                path: target.clone(),
                action: PathAction::Add,
                file: None,
                applied_id: None,
            },
        }];
        for (i, p) in present.iter().enumerate() {
            if Some(i) == at_target {
                continue;
            }
            paths.push(PlanPath {
                path: p.path.clone(),
                // An added format takes off nothing.
                action: match (p.applied_id.as_ref(), row.chosen) {
                    (Some(_), Some(Chosen::Add)) | (None, _) => PathAction::Keep,
                    (Some(_), _) => PathAction::Remove,
                },
                file: Some(p.file.clone()),
                applied_id: p.applied_id.clone(),
            });
        }
        // Two files to take off whose names differ in case only would be two
        // removals of what the effects count as one path.
        let mut taken: Vec<String> = paths
            .iter()
            .filter(|p| matches!(p.action, PathAction::Replace | PathAction::Remove))
            .map(|p| p.path.to_lowercase())
            .collect();
        let count = taken.len();
        taken.sort();
        taken.dedup();
        if taken.len() != count {
            let note = "대소문자만 다른 자막 파일이 여럿이라 그대로 두고 보관만 했어요";
            self.settle_row(row, Outcome::Existing, note.to_owned())
                .await?;
            return self
                .event(&row.job_id, format!("{label}: {note}"), Some(target))
                .await
                .map(|_| true);
        }
        let plan = Plan {
            id: uuid::Uuid::new_v4().to_string(),
            job_id: row.job_id.clone(),
            position: row.position,
            version: 0,
            state: PlanState::Open,
            reason: None,
            work_id: facts.work_id.clone().unwrap_or_default(),
            season: facts.season.unwrap_or(0),
            episode: placed.episode,
            assignment: placed.assignment,
            basis: placed.basis,
            folder: folder.to_owned(),
            video: video_seen,
            stored_id,
            asset_id: asset.id,
            asset_path: asset.relative_path,
            asset_size: asset.size,
            asset_sha256: asset.sha256,
            asset_lines,
            target,
            created_at: 0,
            decided_at: None,
            paths,
        };
        let comparison = self.comparison(&plan, folder, asset_bytes, ext).await;
        let now = self.now();
        let p = plan.clone();
        self.write(move |c| records::make_plan(c, &p, comparison, AWAITING_APPROVAL, now))
            .await?;
        let current = plan.current().map(|(path, _)| path.path.clone());
        self.event(
            &row.job_id,
            format!("{label}: {AWAITING_APPROVAL}"),
            current,
        )
        .await?;
        Ok(true)
    }

    /// What differs between the plan's current subtitle and the new one
    /// (`new`, an `ext` file), made off the async runtime.
    async fn comparison(
        &self,
        plan: &Plan,
        folder: &str,
        new: Vec<u8>,
        ext: &'static str,
    ) -> Comparison {
        let Some((path, seen)) = plan.current() else {
            return Comparison {
                path: plan.target.clone(),
                result: Compared::Unreadable("현재 자막을 찾지 못했어요".to_owned()),
            };
        };
        let at = files::within(Path::new(folder), &path.path);
        let seen = seen.clone();
        let result = blocking(move || diff::compare_files(&at, &seen, &new, ext)).await;
        Comparison {
            path: path.path.clone(),
            result,
        }
    }

    /// Why the plan's evidence no longer holds, if it does not: the work
    /// folder, the row and its stored subtitle where the plan saw them, no
    /// newer revision of the source, the video and that it is the episode's
    /// only one, the subtitles beside it and which are the app's, and the new
    /// subtitle's bytes.
    pub(super) async fn changed(
        &self,
        facts: &JobFacts,
        plan: &Plan,
    ) -> Result<Option<String>, JobError> {
        let work = plan.work_id.clone();
        let folder = self
            .read(move |c| place_records::work_folder(c, &work))
            .await?;
        if folder.as_deref() != Some(plan.folder.as_str()) || !Path::new(&plan.folder).is_dir() {
            return Ok(Some("작품 폴더가 바뀌었어요".to_owned()));
        }
        let (job, position) = (plan.job_id.clone(), plan.position);
        let row = self
            .read(move |c| place_records::plan_row_at(c, &job, position))
            .await?;
        let link = Placed {
            episode: plan.episode,
            assignment: plan.assignment,
            basis: plan.basis,
        };
        let row_holds = row.as_ref().is_some_and(|r| {
            r.stored_id.as_deref() == Some(plan.stored_id.as_str())
                && r.action == PlanAction::Apply
                && link.held_by(
                    r.placed.as_ref().map(|p| p.episode),
                    r.placed.as_ref().map(|p| p.assignment),
                    r.placed.as_ref().and_then(|p| p.basis),
                )
        });
        let stored = plan.stored_id.clone();
        let place = self
            .read(move |c| records::stored_place(c, &stored))
            .await?;
        let stored_holds = place
            .as_ref()
            .is_some_and(|s| link.held_by(s.episode, s.assignment, s.basis));
        if !row_holds || !stored_holds {
            return Ok(Some("회차 대응이 바뀌었어요".to_owned()));
        }
        let asset_id = plan.asset_id.clone();
        let asset = self
            .read(move |c| place_records::asset(c, &asset_id))
            .await?;
        let recorded = place.is_some_and(|s| s.asset_id == plan.asset_id)
            && asset.is_some_and(|a| {
                a.relative_path == plan.asset_path
                    && a.size == plan.asset_size
                    && a.sha256 == plan.asset_sha256
            });
        if !recorded {
            return Ok(Some("새 자막의 보관 기록이 바뀌었어요".to_owned()));
        }
        // The copy a person chose stays over a newer revision of its source.
        let stored = plan.stored_id.clone();
        if !row.as_ref().is_some_and(|r| r.chosen.is_some())
            && self
                .read(move |c| records::newer_revision(c, &stored))
                .await?
                .is_some()
        {
            return Ok(Some(NEW_REVISION.to_owned()));
        }
        let video_at = files::within(Path::new(&plan.folder), &plan.video.path);
        let relative = plan.video.path.clone();
        match blocking(move || seen_video(&video_at, &relative)).await {
            Ok(seen) if seen == plan.video => {}
            Ok(_) => return Ok(Some("영상이 바뀌었어요".to_owned())),
            Err(err) => return Ok(Some(format!("영상을 확인하지 못했어요: {err}"))),
        }
        // Still the episode's one video, as the first copy counts them.
        let (work, season, episode) = (plan.work_id.clone(), plan.season, plan.episode);
        let (videos, _) = self
            .read(move |c| place_records::episode_files(c, &work, season, episode))
            .await?;
        let base = PathBuf::from(&plan.folder);
        let videos = blocking(move || {
            videos
                .into_iter()
                .filter(|v| files::occupied(&files::within(&base, v)).unwrap_or(false))
                .collect::<Vec<_>>()
        })
        .await;
        if videos != [plan.video.path.clone()] {
            return Ok(Some("이 회차의 영상이 바뀌었어요".to_owned()));
        }
        let present = match self
            .scan(facts, &plan.folder, plan.episode, &plan.video.path)
            .await?
        {
            Scan::Files(present) => present,
            Scan::Unreadable(reason) => return Ok(Some(reason)),
        };
        for path in &plan.paths {
            let found = present.iter().find(|p| p.path == path.path);
            match (&path.file, found) {
                (None, Some(_)) => {
                    return Ok(Some(format!(
                        "새 자막을 둘 자리에 파일이 생겼어요 ({})",
                        path.path
                    )))
                }
                (Some(_), None) => {
                    return Ok(Some(format!("기존 자막이 없어졌어요 ({})", path.path)))
                }
                (Some(seen), Some(found))
                    if found.file.size != seen.size
                        || found.file.sha256 != seen.sha256
                        || found.file.object != seen.object =>
                {
                    return Ok(Some(format!(
                        "기존 자막이 비교한 뒤 바뀌었어요 ({})",
                        path.path
                    )))
                }
                // Whether the app manages it decides what the plan does to it.
                (Some(_), Some(found)) if found.applied_id != path.applied_id => {
                    return Ok(Some(format!(
                        "기존 자막의 적용 기록이 바뀌었어요 ({})",
                        path.path
                    )))
                }
                _ => {}
            }
        }
        if let Some(other) = present.iter().find(|p| plan.path(&p.path).is_none()) {
            return Ok(Some(format!(
                "이 회차에 다른 자막이 생겼어요 ({})",
                other.path
            )));
        }
        let asset_at = files::within(Path::new(&plan.folder), &plan.asset_path);
        match blocking(move || read_facts(&asset_at)).await {
            Ok((n, sha, _)) if n == plan.asset_size && sha == plan.asset_sha256 => Ok(None),
            _ => Ok(Some("새 자막의 보관 파일이 기록과 달라요".to_owned())),
        }
    }

    /// The plan goes stale for `reason`, so the row is looked at anew;
    /// whether it did (a person may have decided on it meanwhile, which the
    /// job's next run goes on with, see [`crate::store::JobStore::settle`]).
    async fn stale(&self, row: &PlanRow, plan: &Plan, reason: &str) -> Result<bool, JobError> {
        let (id, from, r, now) = (plan.id.clone(), plan.state, reason.to_owned(), self.now());
        let moved = self
            .write(move |c| records::move_plan(c, &id, from, PlanState::Stale, Some(&r), now))
            .await?;
        if moved {
            self.event(
                &row.job_id,
                format!("{}: 다시 비교가 필요해요", row_label(row)),
                Some(reason.to_owned()),
            )
            .await?;
        }
        Ok(moved)
    }

    /// Carries out an approved plan (see the module docs): whether the row
    /// is settled for this run (`false`: the plan went stale).
    async fn carry_out(
        &self,
        facts: &JobFacts,
        row: &PlanRow,
        plan: Plan,
    ) -> Result<bool, JobError> {
        let label = row_label(row);
        if let Some(reason) = self.changed(facts, &plan).await? {
            return Ok(!self.stale(row, &plan, &reason).await?);
        }
        let apply = Effect {
            id: uuid::Uuid::new_v4().to_string(),
            job_id: plan.job_id.clone(),
            position: plan.position,
            kind: EffectKind::Apply,
            state: EffectState::Intended,
            folder: plan.folder.clone(),
            temp: format!("{}/{}", files::TEMP_DIR, uuid::Uuid::new_v4()),
            target: plan.target.clone(),
            video: Some(plan.video.path.clone()),
            size: plan.asset_size,
            sha256: plan.asset_sha256.clone(),
            object: None,
            reason: None,
            source: None,
            plan_id: Some(plan.id.clone()),
        };
        let removals: Vec<Effect> = plan
            .taken_off()
            .filter_map(|path| {
                let file = path.file.as_ref()?;
                let id = uuid::Uuid::new_v4().to_string();
                Some(Effect {
                    temp: format!("{}/{id}", files::TEMP_DIR),
                    target: format!("{}/{id}.aside", files::TEMP_DIR),
                    id,
                    kind: EffectKind::Remove,
                    video: None,
                    size: file.size,
                    sha256: file.sha256.clone(),
                    source: Some(path.path.clone()),
                    ..apply.clone()
                })
            })
            .collect();
        let mut all = vec![apply.clone()];
        all.extend(removals.iter().cloned());
        let (p, now) = (plan.clone(), self.now());
        match self
            .write(move |c| records::claim(c, &p, &all, now))
            .await?
        {
            Claimed::Yes => {}
            Claimed::NotApproved => return Ok(true),
            Claimed::Busy(path) => {
                let reason = "다른 작업이 이 회차의 자막을 바꾸는 중이에요";
                let (id, r, now) = (plan.id.clone(), reason.to_owned(), self.now());
                self.write(move |c| {
                    records::move_plan(c, &id, PlanState::Approved, PlanState::Stale, Some(&r), now)
                })
                .await?;
                let note = "다른 작업이 이 회차의 자막을 바꾸는 중이라 그대로 두고 보관만 했어요";
                self.settle_row(row, Outcome::Existing, note.to_owned())
                    .await?;
                self.event(&row.job_id, format!("{label}: {note}"), Some(path))
                    .await?;
                return Ok(true);
            }
        }
        self.event(
            &row.job_id,
            format!("{label}: 승인한 교체를 반영해요"),
            None,
        )
        .await?;

        // Nothing beside the video changes before every copy is ready.
        let folder = PathBuf::from(&plan.folder);
        let source = files::within(&folder, &plan.asset_path);
        let temp = files::within(&folder, &apply.temp);
        let (size, sha) = (apply.size, apply.sha256.clone());
        match blocking(move || files::copy_to_temp(&source, &temp, size, &sha)).await {
            Copied::Ready(object) => self.prepared(&apply.id, object).await?,
            Copied::SourceChanged(_) => {
                return self
                    .back_out(
                        row,
                        &plan,
                        Err("새 자막의 보관 파일이 기록과 달라요".to_owned()),
                    )
                    .await
            }
            Copied::Failed(err) => {
                let reason = format!("새 자막을 준비하지 못해 교체하지 않았어요: {err}");
                return self.back_out(row, &plan, Ok(reason)).await;
            }
        }
        for removal in &removals {
            let path = removal.source.clone().unwrap_or_default();
            let source = files::within(&folder, &path);
            let temp = files::within(&folder, &removal.temp);
            let (size, sha) = (removal.size, removal.sha256.clone());
            match blocking(move || files::copy_to_temp(&source, &temp, size, &sha)).await {
                Copied::Ready(object) => self.prepared(&removal.id, object).await?,
                Copied::SourceChanged(_) => {
                    let reason = format!("기존 자막이 비교한 뒤 바뀌었어요 ({path})");
                    return self.back_out(row, &plan, Err(reason)).await;
                }
                Copied::Failed(err) if err.kind() == io::ErrorKind::NotFound => {
                    let reason = format!("기존 자막이 없어졌어요 ({path})");
                    return self.back_out(row, &plan, Err(reason)).await;
                }
                Copied::Failed(err) => {
                    let reason = format!("기존 자막을 보호 복사하지 못해 교체하지 않았어요: {err}");
                    return self.back_out(row, &plan, Ok(reason)).await;
                }
            }
            let unmanaged = plan.path(&path).is_some_and(|p| p.applied_id.is_none());
            if unmanaged {
                if let Err(reason) = self.import(facts, &plan, &path, &removal.temp).await? {
                    let reason =
                        format!("관리하지 않던 자막을 들이지 못해 교체하지 않았어요: {reason}");
                    return self.back_out(row, &plan, Ok(reason)).await;
                }
            }
        }
        if let Some(reason) = self.changed(facts, &plan).await? {
            return self.back_out(row, &plan, Err(reason)).await;
        }
        self.finish_replacement(&plan).await?;
        Ok(true)
    }

    async fn prepared(&self, effect: &str, object: String) -> Result<(), JobError> {
        let (id, now) = (effect.to_owned(), self.now());
        self.write(move |c| place_records::prepared(c, &id, &object, now))
            .await
    }

    /// Stops a plan before anything beside the video changed: its
    /// temporary files go and its effects are abandoned; then the plan fails
    /// (`Ok`, the reason) or goes stale (`Err`, what changed). Whether the
    /// row is settled for this run.
    async fn back_out(
        &self,
        row: &PlanRow,
        plan: &Plan,
        why: Result<String, String>,
    ) -> Result<bool, JobError> {
        if let Some(unsure) = self.abandon_all(plan).await? {
            self.hold_plan(plan, &unsure).await?;
            return Ok(true);
        }
        match why {
            Ok(reason) => {
                let (id, r, now) = (plan.id.clone(), reason.clone(), self.now());
                self.write(move |c| {
                    records::move_plan(
                        c,
                        &id,
                        PlanState::Approved,
                        PlanState::Failed,
                        Some(&r),
                        now,
                    )
                })
                .await?;
                self.fail_row(row, reason).await?;
                Ok(true)
            }
            Err(changed) => Ok(!self.stale(row, plan, &changed).await?),
        }
    }

    /// Removes the temporary files of the plan's unfinished effects, none of
    /// which changed anything beside the video, and abandons them; why not,
    /// when one cannot be.
    async fn abandon_all(&self, plan: &Plan) -> Result<Option<String>, JobError> {
        let id = plan.id.clone();
        let effects = self.read(move |c| records::unfinished_of(c, &id)).await?;
        for effect in effects {
            if effect.state == EffectState::SetAside {
                return Ok(Some(
                    "옮겨 둔 기존 자막이 있어 처음부터 다시 할 수 없어요".to_owned(),
                ));
            }
            let temp = files::within(Path::new(&effect.folder), &effect.temp);
            let object = effect.object.clone();
            let gone = blocking(move || match files::facts(&temp)? {
                // Only this effect writes its temporary file; a prepared
                // one is removed while it is the one recorded.
                Some((_, _, found)) if object.as_ref().is_some_and(|o| *o != found) => {
                    Err(io::Error::other("임시 파일이 기록과 달라요"))
                }
                Some(_) => files::remove_known(&temp),
                None => Ok(()),
            })
            .await;
            if let Err(err) = gone {
                return Ok(Some(format!("임시 파일을 지우지 못했어요: {err}")));
            }
            let (id, now) = (effect.id.clone(), self.now());
            self.write(move |c| records::abandon(c, &id, now)).await?;
        }
        Ok(None)
    }

    /// Holds the plan, its unfinished effects and its row with the reason.
    async fn hold_plan(&self, plan: &Plan, reason: &str) -> Result<(), JobError> {
        let (p, r, now) = (plan.clone(), reason.to_owned(), self.now());
        self.write(move |c| records::hold_plan(c, &p, &r, now))
            .await?;
        self.event(
            &plan.job_id,
            format!("{}화: 보류했어요", plan.episode),
            Some(reason.to_owned()),
        )
        .await
    }

    /// Imports the bytes of the file at `path` the app did not manage, from
    /// its protective copy `protective`, as the stored subtitle of the
    /// creator nobody named on the plan's episode (`Err`: why not; nothing
    /// is left of it then).
    async fn import(
        &self,
        facts: &JobFacts,
        plan: &Plan,
        path: &str,
        protective: &str,
    ) -> Result<Result<(), String>, JobError> {
        let Some(seen) = plan.path(path).and_then(|p| p.file.clone()) else {
            return Ok(Err("계획에 없는 파일이에요".to_owned()));
        };
        let unknown = JobFacts {
            creator: None,
            ..facts.clone()
        };
        let dir = self
            .creator_dir(&unknown, "work", files::SUBTITLES_DIR.to_owned())
            .await?;
        let name = file_name(path).to_owned();
        let what = Imported {
            work_id: plan.work_id.clone(),
            season: plan.season,
            episode: plan.episode,
            job_id: plan.job_id.clone(),
            original_name: name.clone(),
            format: match extension(&name).as_deref() {
                Some("ass") => SubtitleFormat::Ass,
                Some("srt") => SubtitleFormat::Srt,
                Some("smi") => SubtitleFormat::Smi,
                _ => SubtitleFormat::Other,
            },
            encoding: {
                let at = files::within(Path::new(&plan.folder), protective);
                blocking(move || std::fs::read(at).ok().and_then(|b| super::encoding_of(&b))).await
            },
        };
        let probe = PlanRow {
            job_id: plan.job_id.clone(),
            position: plan.position,
            file_id: String::new(),
            member: None,
            name,
            kind: AssetKind::Subtitle,
            format: Some(what.format),
            size: seen.size,
            sha256: seen.sha256.clone(),
            item_id: None,
            anissia_episode: None,
            attachment_episode: None,
            placed: None,
            action: PlanAction::Store,
            question: None,
            stored_id: None,
            outcome: None,
            note: None,
            applied_id: None,
            asset_id: None,
            chosen: None,
        };
        let mut effect: Option<Effect> = None;
        for _ in 0..=super::MAX_RETARGETS {
            let except = effect.as_ref().map(|e| e.id.clone());
            let choice = self
                .choose_name(&unknown, &plan.folder, &dir, &probe, except.as_deref())
                .await?;
            let name = match choice {
                Choice::New(name) => name,
                Choice::Reuse(_, asset) => {
                    if let Some(effect) = &effect {
                        if let Some(unsure) = self.drop_temp(effect).await? {
                            return Ok(Err(unsure));
                        }
                    }
                    let (w, now) = (what.clone(), self.now());
                    self.write(move |c| records::imported(c, &w, None, Some(&asset), now))
                        .await?;
                    return Ok(Ok(()));
                }
                Choice::Full => {
                    return self
                        .import_failed(effect, "보관할 이름을 정하지 못했어요")
                        .await
                }
                Choice::Unreadable(err) => {
                    let reason = format!("보관 폴더를 읽지 못했어요: {err}");
                    return self.import_failed(effect, &reason).await;
                }
            };
            let target = format!("{dir}/{name}");
            let current = match effect.take() {
                Some(mut e) => {
                    e.target = target;
                    let (id, t, now) = (e.id.clone(), e.target.clone(), self.now());
                    self.write(move |c| place_records::retarget(c, &id, &t, now))
                        .await?;
                    e
                }
                None => {
                    let id = uuid::Uuid::new_v4().to_string();
                    let e = Effect {
                        id: id.clone(),
                        job_id: plan.job_id.clone(),
                        position: plan.position,
                        kind: EffectKind::Import,
                        state: EffectState::Intended,
                        folder: plan.folder.clone(),
                        temp: format!("{}/{id}", files::TEMP_DIR),
                        target,
                        video: None,
                        size: seen.size,
                        sha256: seen.sha256.clone(),
                        object: None,
                        reason: None,
                        source: Some(path.to_owned()),
                        plan_id: Some(plan.id.clone()),
                    };
                    let (copy, now) = (e.clone(), self.now());
                    self.write(move |c| place_records::intend(c, &copy, now))
                        .await?;
                    let from = files::within(Path::new(&plan.folder), protective);
                    let temp = files::within(Path::new(&plan.folder), &e.temp);
                    let (size, sha) = (e.size, e.sha256.clone());
                    match blocking(move || files::copy_to_temp(&from, &temp, size, &sha)).await {
                        Copied::Ready(object) => {
                            self.prepared(&e.id, object.clone()).await?;
                            Effect {
                                state: EffectState::Prepared,
                                object: Some(object),
                                ..e
                            }
                        }
                        Copied::SourceChanged(reason) => {
                            return self.import_failed(Some(e), &reason).await
                        }
                        Copied::Failed(err) => {
                            return self.import_failed(Some(e), &err.to_string()).await
                        }
                    }
                }
            };
            let temp = files::within(Path::new(&plan.folder), &current.temp);
            let to = files::within(Path::new(&plan.folder), &current.target);
            match blocking(move || files::publish(&temp, &to)).await {
                Published::Done => {
                    let (w, e, now) = (what.clone(), current.clone(), self.now());
                    self.write(move |c| records::imported(c, &w, Some(&e), None, now))
                        .await?;
                    self.event(
                        &plan.job_id,
                        format!(
                            "{}화: 관리하지 않던 자막을 '{}' 보관본으로 들였어요",
                            plan.episode,
                            super::UNKNOWN_CREATOR
                        ),
                        Some(current.target.clone()),
                    )
                    .await?;
                    return Ok(Ok(()));
                }
                Published::Occupied => effect = Some(current),
                Published::Failed(err) => {
                    return self
                        .import_failed(Some(current), &format!("보관본을 쓰지 못했어요: {err}"))
                        .await
                }
            }
        }
        self.import_failed(effect, "보관할 이름이 계속 다른 파일에 먼저 쓰였어요")
            .await
    }

    /// An import that did not happen: its temporary file goes (`Err`, why).
    async fn import_failed(
        &self,
        effect: Option<Effect>,
        reason: &str,
    ) -> Result<Result<(), String>, JobError> {
        if let Some(effect) = effect {
            if let Some(unsure) = self.drop_temp(&effect).await? {
                return Ok(Err(unsure));
            }
        }
        Ok(Err(reason.to_owned()))
    }

    /// Removes an unfinished effect's temporary file and abandons it; why
    /// not, when the file cannot go.
    async fn drop_temp(&self, effect: &Effect) -> Result<Option<String>, JobError> {
        let temp = files::within(Path::new(&effect.folder), &effect.temp);
        if let Err(err) = blocking(move || files::remove_known(&temp)).await {
            return Ok(Some(format!("임시 파일을 지우지 못했어요: {err}")));
        }
        let (id, now) = (effect.id.clone(), self.now());
        self.write(move |c| records::abandon(c, &id, now)).await?;
        Ok(None)
    }

    /// Steps 3 to 6 of carrying out (see the module docs), from wherever the
    /// plan's effects are.
    async fn finish_replacement(&self, plan: &Plan) -> Result<(), JobError> {
        let id = plan.id.clone();
        let effects = self.read(move |c| records::effects_of(c, &id)).await?;
        let removal_of = |path: &str| {
            effects.iter().find(|e| {
                e.kind == EffectKind::Remove
                    && e.source.as_deref() == Some(path)
                    && e.state != EffectState::Abandoned
            })
        };
        let apply = effects
            .iter()
            .find(|e| e.kind == EffectKind::Apply && e.state != EffectState::Abandoned);
        let Some(apply) = apply else {
            return self.hold_plan(plan, "교체할 새 자막의 기록이 없어요").await;
        };

        if apply.state != EffectState::Done {
            let replaced = plan.paths.iter().find(|p| p.action == PathAction::Replace);
            if let Some(replaced) = replaced {
                let Some(removal) = removal_of(&replaced.path) else {
                    return self
                        .hold_plan(plan, "교체할 기존 자막의 기록이 없어요")
                        .await;
                };
                if removal.state == EffectState::Prepared {
                    match self.set_aside(plan, removal, replaced).await? {
                        Aside::Done => {}
                        Aside::Untouched(changed) => {
                            return self.untouched(plan, Err(changed)).await
                        }
                        Aside::Failed(err) => {
                            let reason =
                                format!("기존 자막을 옮기지 못해 교체하지 않았어요: {err}");
                            return self.untouched(plan, Ok(reason)).await;
                        }
                        Aside::Unsure(reason) => return self.hold_plan(plan, &reason).await,
                    }
                }
            }
            let folder = PathBuf::from(&plan.folder);
            let temp = files::within(&folder, &apply.temp);
            let target = files::within(&folder, &apply.target);
            let published = blocking(move || files::publish(&temp, &target)).await;
            match (published, replaced.is_some()) {
                (Published::Done, _) => {
                    if !self.replaced(plan, apply).await? {
                        return Ok(());
                    }
                }
                // Nothing beside the video changed yet.
                (Published::Occupied, false) => {
                    let changed = format!("새 자막을 둘 자리에 파일이 생겼어요 ({})", plan.target);
                    return self.untouched(plan, Err(changed)).await;
                }
                (Published::Failed(err), false) => {
                    // Nothing moved while the temporary file is still there.
                    let temp = files::within(&folder, &apply.temp);
                    let still = blocking(move || files::facts(&temp)).await;
                    let kept =
                        matches!(&still, Ok(Some((_, _, o))) if Some(o) == apply.object.as_ref());
                    if !kept {
                        let reason = format!("새 자막을 공개한 결과를 확인하지 못했어요: {err}");
                        return self.hold_plan(plan, &reason).await;
                    }
                    let reason = format!("새 자막을 공개하지 못해 교체하지 않았어요: {err}");
                    return self.untouched(plan, Ok(reason)).await;
                }
                (Published::Occupied, true) => {
                    return self
                        .hold_plan(plan, "교체할 이름에 다른 파일이 먼저 생겨 보류했어요")
                        .await
                }
                (Published::Failed(err), true) => {
                    let reason = format!("새 자막을 공개하지 못해 보류했어요: {err}");
                    return self.hold_plan(plan, &reason).await;
                }
            }
        }

        for removed in plan.paths.iter().filter(|p| p.action == PathAction::Remove) {
            let Some(removal) = removal_of(&removed.path) else {
                return self
                    .hold_plan(plan, "지울 이전 적용본의 기록이 없어요")
                    .await;
            };
            if removal.state != EffectState::Prepared {
                continue;
            }
            let failed = match self.set_aside(plan, removal, removed).await? {
                Aside::Done => continue,
                Aside::Untouched(reason) | Aside::Unsure(reason) => reason,
                Aside::Failed(err) => err.to_string(),
            };
            let reason = format!(
                "새 자막은 적용했지만 이전 적용본을 지우지 못했어요 ({}): {failed}",
                removed.path
            );
            return self.hold_plan(plan, &reason).await;
        }

        let (id, now) = (plan.id.clone(), self.now());
        let done = self
            .write(move |c| {
                records::move_plan(c, &id, PlanState::Approved, PlanState::Done, None, now)
            })
            .await?;
        if !done {
            // Held meanwhile: its protective copies stay.
            return Ok(());
        }
        self.event(
            &plan.job_id,
            format!("{}화: 새 자막으로 교체했어요", plan.episode),
            Some(plan.target.clone()),
        )
        .await?;
        self.clean_up(plan).await
    }

    /// Stops a plan that changed nothing beside the video yet, as
    /// [`Placer::back_out`] does.
    async fn untouched(&self, plan: &Plan, why: Result<String, String>) -> Result<(), JobError> {
        let (job, position) = (plan.job_id.clone(), plan.position);
        let row = self
            .read(move |c| place_records::plan_row_at(c, &job, position))
            .await?;
        match row {
            Some(row) => self.back_out(&row, plan, why).await.map(|_| ()),
            None => Ok(()),
        }
    }

    /// Renames `path`'s file aside (to `removal`'s target) and checks it is
    /// the one the plan saw.
    async fn set_aside(
        &self,
        plan: &Plan,
        removal: &Effect,
        path: &PlanPath,
    ) -> Result<Aside, JobError> {
        let Some(seen) = path.file.clone() else {
            return Ok(Aside::Unsure("옮길 기존 자막의 기록이 없어요".to_owned()));
        };
        let folder = PathBuf::from(&plan.folder);
        let from = files::within(&folder, &path.path);
        let aside = files::within(&folder, &removal.target);
        let shown = path.path.clone();
        let moved = blocking(move || -> Result<Aside, io::Error> {
            match rename_noreplace(&from, &aside) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    return Ok(Aside::Untouched(format!("기존 자막이 없어졌어요 ({shown})")))
                }
                Err(err) => return Ok(Aside::Failed(err)),
            }
            for dir in [from.parent(), aside.parent()].into_iter().flatten() {
                sync_dir(dir)?;
            }
            let found = files::facts(&aside).ok().flatten();
            if same_file(&found, &seen) {
                return Ok(Aside::Done);
            }
            // Not the file the plan saw: it goes back where it was.
            match rename_noreplace(&aside, &from) {
                Ok(()) => {
                    for dir in [from.parent(), aside.parent()].into_iter().flatten() {
                        sync_dir(dir)?;
                    }
                    Ok(Aside::Untouched(format!(
                        "기존 자막이 비교한 뒤 바뀌었어요 ({shown})"
                    )))
                }
                Err(err) => Ok(Aside::Unsure(format!(
                    "옮긴 기존 자막이 비교한 것과 달라 되돌리려 했지만 하지 못했어요 ({shown}): {err}"
                ))),
            }
        })
        .await;
        let moved = match moved {
            Ok(moved) => moved,
            Err(err) => {
                return Ok(Aside::Unsure(format!(
                    "기존 자막을 옮긴 뒤 폴더를 동기화하지 못했어요: {err}"
                )))
            }
        };
        if let Aside::Done = moved {
            let (id, applied, now) = (removal.id.clone(), path.applied_id.clone(), self.now());
            self.write(move |c| records::set_aside(c, &id, applied.as_deref(), now))
                .await?;
        }
        Ok(moved)
    }

    /// Records the plan's new copy, published, as applied; whether it could
    /// (else the plan is held, and nothing more is done to it).
    async fn replaced(&self, plan: &Plan, apply: &Effect) -> Result<bool, JobError> {
        let copy = NewApplied {
            work_id: plan.work_id.clone(),
            stored_id: plan.stored_id.clone(),
            season: plan.season,
            episode: plan.episode,
        };
        let (e, now) = (apply.clone(), self.now());
        let object = apply.object.clone().unwrap_or_default();
        let note = Some("기존 자막을 새 자막으로 교체했어요".to_owned());
        match self
            .write(move |c| place_records::applied(c, &e, &copy, &object, note, now))
            .await
        {
            Ok(_) => Ok(true),
            Err(err) if super::is_constraint(&err) => {
                let reason = format!("공개한 파일을 기록하지 못했어요: {err}");
                self.hold_plan(plan, &reason).await?;
                Ok(false)
            }
            Err(err) => Err(err),
        }
    }

    /// Removes the protective copies and the files set aside of a done plan,
    /// each while it is the one recorded, and ends its removals `done` (or
    /// `held`, its files left, when one is not).
    async fn clean_up(&self, plan: &Plan) -> Result<(), JobError> {
        let id = plan.id.clone();
        let effects = self.read(move |c| records::unfinished_of(c, &id)).await?;
        for effect in effects {
            let folder = PathBuf::from(&effect.folder);
            let seen = effect
                .source
                .as_deref()
                .and_then(|s| plan.path(s))
                .and_then(|p| p.file.clone());
            let (state, reason) = match (effect.kind, effect.state, seen) {
                (EffectKind::Remove, EffectState::SetAside, Some(seen)) => {
                    let aside = files::within(&folder, &effect.target);
                    let protective = files::within(&folder, &effect.temp);
                    let object = effect.object.clone();
                    let gone = blocking(move || -> io::Result<bool> {
                        let a = files::facts(&aside)?;
                        let p = files::facts(&protective)?;
                        let ours_aside = a.is_none() || same_file(&a, &seen);
                        let ours_copy = p
                            .as_ref()
                            .is_none_or(|(_, _, o)| Some(o) == object.as_ref());
                        if !(ours_aside && ours_copy) {
                            return Ok(false);
                        }
                        files::remove_known(&aside)?;
                        files::remove_known(&protective)?;
                        Ok(true)
                    })
                    .await;
                    match gone {
                        Ok(true) => (EffectState::Done, None),
                        Ok(false) => (
                            EffectState::Held,
                            Some("정리할 보호 자료가 기록과 달라 남겨 뒀어요".to_owned()),
                        ),
                        Err(err) => (
                            EffectState::Held,
                            Some(format!("보호 자료를 정리하지 못했어요: {err}")),
                        ),
                    }
                }
                _ => (
                    EffectState::Held,
                    Some("교체를 마친 뒤 끝나지 않은 효과가 남았어요".to_owned()),
                ),
            };
            let (id, now) = (effect.id.clone(), self.now());
            self.write(move |c| place_records::end_effect(c, &id, state, reason, now))
                .await?;
        }
        Ok(())
    }

    /// Compares the unfinished effects of a plan an earlier start left with
    /// the disk (see the module docs).
    pub(super) async fn recover_plan(&self, plan_id: &str) -> Result<(), JobError> {
        let id = plan_id.to_owned();
        let Some(plan) = self.read(move |c| records::plan(c, &id)).await? else {
            return Ok(());
        };
        match plan.state {
            PlanState::Done => return self.clean_up(&plan).await,
            PlanState::Approved => {}
            _ => {
                // Nothing else leaves effects under way: they are held.
                let (p, now) = (plan.clone(), self.now());
                return self
                    .write(move |c| {
                        records::hold_plan(c, &p, "이전 작업의 효과가 끝나지 않았어요", now)
                    })
                    .await;
            }
        }
        let id = plan.id.clone();
        let effects = self.read(move |c| records::unfinished_of(c, &id)).await?;
        let folder = PathBuf::from(&plan.folder);
        for effect in &effects {
            let temp = files::within(&folder, &effect.temp);
            let target = files::within(&folder, &effect.target);
            let (t, g) = blocking(move || (files::facts(&temp), files::facts(&target))).await;
            let (Ok(t), Ok(g)) = (t, g) else {
                return self
                    .hold_plan(&plan, "이전 작업의 파일을 확인하지 못했어요")
                    .await;
            };
            let ours = |found: &Option<(u64, String, String)>| {
                found.as_ref().is_some_and(|(n, s, o)| {
                    *n == effect.size && *s == effect.sha256 && Some(o) == effect.object.as_ref()
                })
            };
            match (effect.kind, effect.state) {
                (EffectKind::Apply | EffectKind::Import, EffectState::Prepared)
                    if t.is_none() && ours(&g) =>
                {
                    // Published before the record.
                    match effect.kind {
                        EffectKind::Apply => {
                            if !self.replaced(&plan, effect).await? {
                                return Ok(());
                            }
                        }
                        _ => {
                            let name =
                                file_name(effect.source.as_deref().unwrap_or_default()).to_owned();
                            let at = g.clone();
                            let encoding = {
                                let target = files::within(&folder, &effect.target);
                                blocking(move || {
                                    std::fs::read(target)
                                        .ok()
                                        .and_then(|b| super::encoding_of(&b))
                                })
                                .await
                            };
                            let what = Imported {
                                work_id: plan.work_id.clone(),
                                season: plan.season,
                                episode: plan.episode,
                                job_id: plan.job_id.clone(),
                                format: match extension(&name).as_deref() {
                                    Some("ass") => SubtitleFormat::Ass,
                                    Some("srt") => SubtitleFormat::Srt,
                                    Some("smi") => SubtitleFormat::Smi,
                                    _ => SubtitleFormat::Other,
                                },
                                original_name: name,
                                encoding,
                            };
                            debug_assert!(at.is_some());
                            let (e, now) = (effect.clone(), self.now());
                            self.write(move |c| records::imported(c, &what, Some(&e), None, now))
                                .await?;
                        }
                    }
                }
                (EffectKind::Import, EffectState::Intended)
                | (EffectKind::Import, EffectState::Prepared)
                    if t.is_none() || ours(&t) || effect.state == EffectState::Intended =>
                {
                    // Imported anew when the plan is carried out again.
                    if let Some(unsure) = self.drop_temp(effect).await? {
                        return self.hold_plan(&plan, &unsure).await;
                    }
                }
                (EffectKind::Remove, EffectState::Prepared) => {
                    let Some(path) = effect.source.as_deref().and_then(|s| plan.path(s)) else {
                        return self
                            .hold_plan(&plan, "옮길 기존 자막의 기록이 없어요")
                            .await;
                    };
                    let Some(seen) = path.file.clone() else {
                        return self
                            .hold_plan(&plan, "옮길 기존 자막의 기록이 없어요")
                            .await;
                    };
                    let from = files::within(&folder, &path.path);
                    let found = blocking(move || files::facts(&from)).await;
                    let in_place = matches!(&found, Ok(f) if same_file(f, &seen));
                    let aside = same_file(&g, &seen);
                    match (in_place, aside, found) {
                        (true, false, _) => {}
                        (false, true, Ok(None)) => {
                            let (id, applied, now) =
                                (effect.id.clone(), path.applied_id.clone(), self.now());
                            self.write(move |c| {
                                records::set_aside(c, &id, applied.as_deref(), now)
                            })
                            .await?;
                        }
                        _ => {
                            return self
                                .hold_plan(
                                    &plan,
                                    "이전 작업이 옮기던 기존 자막이 기록과 달라 결과를 확인할 수 없어요",
                                )
                                .await
                        }
                    }
                }
                (EffectKind::Remove, EffectState::SetAside) => {
                    let seen = effect
                        .source
                        .as_deref()
                        .and_then(|s| plan.path(s))
                        .and_then(|p| p.file.clone());
                    if !seen.is_some_and(|s| same_file(&g, &s)) {
                        return self
                            .hold_plan(
                                &plan,
                                "옮겨 둔 기존 자막이 기록과 달라 결과를 확인할 수 없어요",
                            )
                            .await;
                    }
                }
                (EffectKind::Apply | EffectKind::Remove, EffectState::Intended) => {}
                (EffectKind::Apply, EffectState::Prepared) if ours(&t) => {}
                _ => {
                    return self
                        .hold_plan(
                            &plan,
                            "이전 작업이 쓰던 파일이 기록과 달라 결과를 확인할 수 없어요",
                        )
                        .await
                }
            }
        }

        let id = plan.id.clone();
        let effects = self.read(move |c| records::effects_of(c, &id)).await?;
        let applied = effects
            .iter()
            .any(|e| e.kind == EffectKind::Apply && e.state == EffectState::Done);
        let aside = effects
            .iter()
            .any(|e| e.kind == EffectKind::Remove && e.state == EffectState::SetAside);
        if !applied && !aside {
            // Nothing beside the video changed: the plan is carried out anew.
            if let Some(unsure) = self.abandon_all(&plan).await? {
                return self.hold_plan(&plan, &unsure).await;
            }
            return Ok(());
        }
        let unprepared = effects.iter().any(|e| {
            matches!(e.kind, EffectKind::Apply | EffectKind::Remove)
                && e.state == EffectState::Intended
        });
        if unprepared {
            return self
                .hold_plan(&plan, "준비하지 않은 효과가 있는데 기존 자막이 바뀌었어요")
                .await;
        }
        self.finish_replacement(&plan).await
    }
}
