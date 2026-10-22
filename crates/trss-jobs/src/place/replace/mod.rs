//! Replacing the subtitle an episode has, once a person approves it
//! (`docs/requirements/subtitles.md`, 교체 비교와 승인 and 승인 증거와 반영 직전
//! 검사; `docs/requirements/jobs.md`, 체크포인트와 중단 복구).
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
//! it), and an episode with two files to take off whose names differ in case
//! only.
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
//! ends ([`crate::store::JobRun::settle`]).
//! Each run looks at an open plan again ([`Placer::changed`]): one whose
//! evidence no longer holds goes `stale` and the row's next plan is made, so
//! the person compares again. A file is still the one the plan saw by its
//! inode ([`same_recorded_file`]), not its device number, which a machine restarted
//! meanwhile may have changed.
//!
//! # Carrying out
//!
//! An approved plan ([`Placer::carry_out`]) records nothing before it
//! changes a file: every run checks it again from the start ([`Placer::changed`]),
//! and a change makes it stale, so the next plan is made. Then:
//!
//! 1. The new copy is written whole to the work folder's staging folder
//!    ([`files::Staging`]), synced and read back. Nothing beside the video
//!    changed yet: a failure (no room) fails the plan, the stored copy not
//!    being the recorded bytes makes it stale.
//! 2. The plan is checked again.
//! 3. A file the app did not manage at the path to replace is imported from
//!    that path, as the stored subtitle of the creator nobody named, by a
//!    store's write: copied and read back with the length and SHA-256 the
//!    plan saw, then published by a rename that replaces nothing (or found
//!    there with its bytes, or found recorded already). Nothing beside the
//!    video changed yet: one that cannot be imported fails the plan, and a
//!    file changed or gone meanwhile makes it stale. A plan that stops after
//!    the import (the swap of 4 swaps back, say) leaves the import stored: a
//!    copy more of that file under the creator nobody named.
//! 4. A plan that `add`s publishes the new copy by a rename that replaces
//!    nothing; a file there meanwhile makes it stale. A plan that `replace`s
//!    moves the new copy into `.trss/aside/<ID>` ([`files::ASIDE_DIR`]), by a
//!    rename that replaces nothing, and swaps it with the file at the path in
//!    one step ([`aside::swap`]): the path is never empty. The file the swap
//!    took off, now aside, must be the approved one (its length, SHA-256 and
//!    inode, as the plan saw it); one that is not is swapped back, so the
//!    changed file is in its place again, the new copy goes and the plan is
//!    stale. A path whose name differs in case from the target is renamed to
//!    the target's name once swapped.
//! 5. Each earlier applied copy at another path is renamed into
//!    `.trss/aside/<ID>`, replacing nothing, and checked the same way. One
//!    that is not the approved file goes back; that one, or one that cannot
//!    be moved, holds the plan: the new copy stays applied, the removal is not
//!    taken as done, and nothing is rolled back.
//! 6. One synced transaction records the new copy applied, the applied
//!    copies taken off removed, and the plan `done` (or `held`, by 5). A
//!    record the database refuses holds the plan: the copies of 5 go back to
//!    their paths, and the file the new copy took the place of stays aside.
//! 7. The files this run moved aside go. A held plan's stay.
//!
//! Something this run cannot tell the outcome of (a folder sync that failed,
//! a swap back that failed) holds the plan with every file where it is.
//!
//! # Doing it again
//!
//! A run cut short (a process stopped, or a run that ended in an error and
//! that the same process takes up again) leaves the plan approved, and the
//! next run carries it out again from its first check. What a cut leaves,
//! by where it fell:
//!
//! | Cut | On disk | The next run |
//! | --- | --- | --- |
//! | before an import's publish, or before the new copy is aside when nothing is imported | staged copies at most | empties the staging folder and carries the plan out anew |
//! | after an import's publish, before its record | the same, and the import's file, unrecorded, under the name it took | carries the plan out anew; its import records that file (`Choice::Adopt`) |
//! | after an import's record, before the new copy is aside | the same, and the import recorded | carries the plan out anew; its import finds that stored subtitle (`Choice::Reuse`) and records nothing more |
//! | after the new copy is aside, before the swap | that, and a copy of the new bytes in `.trss/aside/` | carries the plan out anew; that copy stays |
//! | after the swap or the `add`'s publish, before the transaction of 6 | the new copy at its path; the file it took the place of (imported by then when the app did not manage it) in `.trss/aside/` | finds the path changed: the plan is stale, and the comparison records the new copy as applied ([`ADOPTED`]) |
//! | after an earlier applied copy is aside | the same, and that copy in `.trss/aside/` | the same; its applied record is not ended |
//! | after the transaction of 6 | the plan done, its files in `.trss/aside/` | nothing: the row is applied |
//!
//! An import names no applied copy and nothing beside the video, so the
//! check that starts the next run does not see it.
//!
//! Nothing removes what a run cut short left in `.trss/aside/`: nothing
//! records whose file it is. A plan stale after a cut leaves the paths it did
//! not reach to the next comparison of that episode. Releases before lighter-structure/0153
//! kept a replacement's files in `.trss/tmp/`, which nothing empties either.

pub mod diff;
pub mod lines;
pub mod records;

use std::{
    io,
    ops::ControlFlow,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use trss_core::{
    file_id::same_recorded_file,
    files::{create_dir_all_synced, rename_noreplace, rename_noreplace_synced, sync_renamed},
};

use super::{
    aside::{self, Check, Expected, Moved, Swapped},
    blocking, files,
    files::{Copied, Published},
    joined,
    package::extension,
    records::{
        self as place_records, Adopted, JobFacts, NewApplied, Placed, PlanRow, PublishedCopy,
    },
    row_label, video_parts, Choice, Placer, SUBTITLE_EXTENSIONS,
};
use crate::{
    area::{object_of, read_facts},
    model::{AssetKind, Chosen, Outcome, PathAction, PlanAction, PlanState, SubtitleFormat},
    store::JobError,
};
use records::{Compared, Comparison, FileSeen, Imported, ImportedAsset, Plan, PlanPath, VideoSeen};

/// Why a plan stops before anything beside the video changed: it fails
/// (`Ok`, the reason) or goes stale (`Err`, what changed).
type Stop = Result<String, String>;

/// The error number of a rename of a file that is not there.
const ENOENT: i32 = 2;

/// The reason a plan goes stale for a newer revision (`새 수정본 발견`).
pub const NEW_REVISION: &str = "같은 출처의 새 수정본이 들어왔어요";
/// What a row says when the subtitle beside the video has the stored copy's
/// bytes and is recorded as its applied copy.
pub const ADOPTED: &str = "이 회차에 같은 자막이 이미 있어 그 파일을 적용본으로 기록했어요";
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

/// A file a run took off a path beside the video, into `.trss/aside/`.
#[derive(Debug, Clone)]
struct TakenOff {
    /// The path it was at, relative to the work folder.
    path: String,
    /// Where it is now, relative to the work folder.
    aside: String,
    /// The applied copy it was.
    applied_id: Option<String>,
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

/// What the file the plan saw is: its object and bytes.
fn expected(seen: &FileSeen) -> Expected<'_> {
    Expected {
        size: seen.size,
        sha256: &seen.sha256,
        object: Some(&seen.object),
    }
}

/// The same video as the plan saw: its path, object, length and change time.
fn same_video(found: &VideoSeen, seen: &VideoSeen) -> bool {
    found.path == seen.path
        && same_recorded_file(&found.object, &seen.object)
        && found.size == seen.size
        && found.mtime == seen.mtime
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
        staging: &files::Staging,
    ) -> Result<bool, JobError> {
        match plan.state {
            PlanState::Open => match self.changed(facts, &plan).await? {
                None => Ok(true),
                // A plan decided meanwhile is the next run's.
                Some(reason) => Ok(!self.stale(row, &plan, &reason).await?),
            },
            PlanState::Approved => self.carry_out(facts, row, plan, staging).await,
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
        // A file beside the video with the stored copy's bytes: the one at the
        // target's name first (the case-only match too), else the first in the
        // order the files were found.
        let identical: Vec<usize> = present
            .iter()
            .enumerate()
            .filter(|(_, p)| p.file.sha256 == asset.sha256)
            .map(|(i, _)| i)
            .collect();
        let same = identical
            .iter()
            .copied()
            .find(|i| Some(*i) == at_target)
            .or_else(|| identical.first().copied());
        let same_note = "이 회차에 같은 자막이 이미 있어 그대로 두고 보관만 했어요";
        let settled = match same {
            // The revision's own job compares it: it is not made the applied
            // copy.
            Some(_) if newer => Some(stored_only(same_note)),
            Some(i) => {
                // The stored copy is what the applied copy is a copy of: it
                // is read as the comparison reads it, and a file that is gone
                // or not the recorded bytes holds the row, so a person's file
                // is not recorded as managed while nothing else keeps its bytes.
                if !self.stored_as_recorded(folder, &asset).await {
                    let reason = "보관본이 기록과 달라 영상 옆 자막을 적용본으로 기록하지 못했어요"
                        .to_owned();
                    self.hold_row(row, &label, reason, &asset.relative_path)
                        .await?;
                    return Ok(true);
                }
                if self
                    .adopt(
                        facts,
                        row,
                        video,
                        &present[i],
                        stored_id.clone(),
                        placed.episode,
                    )
                    .await?
                {
                    return Ok(true);
                }
                // It is a copy applied already (of this stored subtitle, or of
                // another that a job's own apply does not take over).
                Some(stored_only(same_note))
            }
            None if newer => Some(stored_only(
                "같은 출처의 새 수정본이 들어와 이 자막은 보관만 했어요",
            )),
            None => None,
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
                self.hold_row(row, &label, reason, &asset.relative_path)
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
        // Two files to take off whose names differ in case only are one file
        // on a file system that does not tell case apart (SMB): which one a
        // removal takes is not known.
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
            Ok(seen) if same_video(&seen, &plan.video) => {}
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
                .filter(|v| trss_core::files::occupied(&files::within(&base, v)).unwrap_or(false))
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
                        || !same_recorded_file(&found.file.object, &seen.object) =>
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
    /// job's next run goes on with, see [`crate::store::JobRun::settle`]).
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
        staging: &files::Staging,
    ) -> Result<bool, JobError> {
        let label = row_label(row);
        if let Some(reason) = self.changed(facts, &plan).await? {
            return Ok(!self.stale(row, &plan, &reason).await?);
        }
        self.event(
            &row.job_id,
            format!("{label}: 승인한 교체를 반영해요"),
            None,
        )
        .await?;

        // 1. The new copy, staged: nothing beside the video changed yet.
        let folder = PathBuf::from(&plan.folder);
        let staged = files::within(&folder, &self.staged(staging, &plan.folder, true).await);
        let (source, at) = (files::within(&folder, &plan.asset_path), staged.clone());
        let (size, sha) = (plan.asset_size, plan.asset_sha256.clone());
        let object = match blocking(move || files::copy_to_temp(&source, &at, size, &sha)).await {
            Copied::Ready(object) => object,
            Copied::SourceChanged(_) => {
                let changed = "새 자막의 보관 파일이 기록과 달라요".to_owned();
                return self.back_out(row, &plan, Err(changed)).await;
            }
            Copied::Failed(err) => {
                self.discard(&staged).await;
                let reason = format!("새 자막을 준비하지 못해 교체하지 않았어요: {err}");
                return self.back_out(row, &plan, Ok(reason)).await;
            }
        };
        // 2. Checked again right before anything beside the video changes.
        if let Some(changed) = self.changed(facts, &plan).await? {
            self.discard(&staged).await;
            return self.back_out(row, &plan, Err(changed)).await;
        }

        // 3. A file the app did not manage is kept as a stored subtitle,
        // from its path, before anything takes it off: nothing beside the
        // video changed yet when it cannot be.
        let replaced = plan.paths.iter().find(|p| p.action == PathAction::Replace);
        if let Some(unmanaged) = replaced.filter(|p| p.applied_id.is_none()) {
            if let Err(why) = self.import(facts, &plan, &unmanaged.path, staging).await? {
                self.discard(&staged).await;
                let why = why.map(|reason| {
                    format!("관리하지 않던 자막을 들이지 못해 교체하지 않았어요: {reason}")
                });
                return self.back_out(row, &plan, why).await;
            }
        }

        // 4. The new copy takes its path.
        let placed = match replaced {
            None => self.publish_added(row, &plan, &staged, &object).await?,
            Some(replaced) => self.swap_in(row, &plan, replaced, &staged).await?,
        };
        let (path, displaced) = match placed {
            ControlFlow::Continue(placed) => placed,
            ControlFlow::Break(settled) => return Ok(settled),
        };

        // 5. The earlier applied copies at other paths go aside.
        let mut removed: Vec<TakenOff> = Vec::new();
        let mut held: Option<String> = None;
        for old in plan.paths.iter().filter(|p| p.action == PathAction::Remove) {
            match self.set_aside_removed(&plan, old).await {
                Ok(taken) => removed.push(taken),
                Err(reason) => {
                    held = Some(format!(
                        "새 자막은 적용했지만 이전 적용본을 지우지 못했어요 ({}): {reason}",
                        old.path
                    ));
                    break;
                }
            }
        }

        // 6. What was done, in one transaction.
        let published = PublishedCopy {
            job_id: plan.job_id.clone(),
            position: plan.position,
            video: Some(plan.video.path.clone()),
            path,
            size: plan.asset_size,
            sha256: plan.asset_sha256.clone(),
        };
        let copy = NewApplied {
            work_id: plan.work_id.clone(),
            stored_id: plan.stored_id.clone(),
            season: plan.season,
            episode: plan.episode,
        };
        let taken_off: Vec<String> = displaced
            .iter()
            .chain(&removed)
            .filter_map(|t| t.applied_id.clone())
            .collect();
        let (p, h, now) = (plan.clone(), held.clone(), self.now());
        let note = "기존 자막을 새 자막으로 교체했어요";
        match self
            .write(move |c| {
                records::replaced(
                    c,
                    &p,
                    &published,
                    &copy,
                    note,
                    &taken_off,
                    h.as_deref(),
                    now,
                )
            })
            .await
        {
            Ok(()) => {}
            Err(err) if super::is_constraint(&err) => {
                // Nothing of it is on record: the earlier applied copies go
                // back, and the file the new copy took the place of stays
                // aside.
                let mut reason = format!("공개한 파일을 기록하지 못했어요: {err}");
                let stuck = self.put_back(&plan, &removed).await;
                if !stuck.is_empty() {
                    reason.push_str(&format!(
                        " (옮겨 둔 이전 적용본을 되돌리지 못했어요: {})",
                        stuck.join(", ")
                    ));
                }
                self.hold_plan(&plan, &reason).await?;
                return Ok(true);
            }
            Err(err) => return Err(err),
        }
        if let Some(reason) = held {
            self.event(
                &plan.job_id,
                format!("{}화: 보류했어요", plan.episode),
                Some(reason),
            )
            .await?;
            return Ok(true);
        }
        self.event(
            &plan.job_id,
            format!("{}화: 새 자막으로 교체했어요", plan.episode),
            Some(plan.target.clone()),
        )
        .await?;

        // 7. What this run moved aside goes.
        let asides: Vec<PathBuf> = displaced
            .iter()
            .chain(&removed)
            .map(|t| files::within(&folder, &t.aside))
            .collect();
        let gone = blocking(move || {
            let paths: Vec<&Path> = asides.iter().map(PathBuf::as_path).collect();
            aside::remove(&paths)
        })
        .await;
        if let Err(err) = gone {
            self.event(
                &plan.job_id,
                format!("{}화: 옮겨 둔 기존 자막을 지우지 못했어요", plan.episode),
                Some(format!("{}: {err}", files::ASIDE_DIR)),
            )
            .await?;
        }
        Ok(true)
    }

    /// Step 4 of carrying out a plan that `add`s: the new copy, staged at
    /// `staged` (the file `object`), published at the plan's target. The
    /// path it is at, or whether the row is settled when the plan ended.
    async fn publish_added(
        &self,
        row: &PlanRow,
        plan: &Plan,
        staged: &Path,
        object: &str,
    ) -> Result<ControlFlow<bool, (String, Option<TakenOff>)>, JobError> {
        let target = files::within(Path::new(&plan.folder), &plan.target);
        let from = staged.to_owned();
        let ended = match blocking(move || files::publish(&from, &target)).await {
            Published::Done => return Ok(ControlFlow::Continue((plan.target.clone(), None))),
            Published::Occupied => {
                self.discard(staged).await;
                let changed = format!("새 자막을 둘 자리에 파일이 생겼어요 ({})", plan.target);
                self.back_out(row, plan, Err(changed)).await?
            }
            Published::Failed(err) => {
                // Nothing moved while the new copy is where it was staged.
                let at = staged.to_owned();
                let still = blocking(move || files::facts(&at)).await;
                if !matches!(&still, Ok(Some((_, _, o))) if same_recorded_file(object, o)) {
                    let reason = format!("새 자막을 공개한 결과를 확인하지 못했어요: {err}");
                    self.hold_plan(plan, &reason).await?;
                    return Ok(ControlFlow::Break(true));
                }
                self.discard(staged).await;
                let reason = format!("새 자막을 공개하지 못해 교체하지 않았어요: {err}");
                self.back_out(row, plan, Ok(reason)).await?
            }
        };
        Ok(ControlFlow::Break(ended))
    }

    /// Step 4 of carrying out a plan that `replace`s `replaced`: the new
    /// copy, staged at `staged`, swapped with the file there. The path the
    /// new copy is at and the file it took off, or whether the row is
    /// settled when the plan ended.
    async fn swap_in(
        &self,
        row: &PlanRow,
        plan: &Plan,
        replaced: &PlanPath,
        staged: &Path,
    ) -> Result<ControlFlow<bool, (String, Option<TakenOff>)>, JobError> {
        let ended = |settled| Ok(ControlFlow::Break(settled));
        let Some(seen) = replaced.file.clone() else {
            self.discard(staged).await;
            self.hold_plan(plan, "교체할 기존 자막의 기록이 없어요")
                .await?;
            return ended(true);
        };
        let folder = PathBuf::from(&plan.folder);
        let aside = format!("{}/{}", files::ASIDE_DIR, uuid::Uuid::new_v4());
        let aside_at = files::within(&folder, &aside);
        let path_at = files::within(&folder, &replaced.path);
        let shown = replaced.path.clone();

        // The new copy waits where nothing empties: once swapped, the file it
        // takes off is there.
        let (from, to) = (staged.to_owned(), aside_at.clone());
        let moved = blocking(move || -> io::Result<io::Result<()>> {
            if let Some(parent) = to.parent() {
                create_dir_all_synced(parent)?;
            }
            rename_noreplace(&from, &to)?;
            Ok(sync_renamed(&from, &to))
        })
        .await;
        match moved {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                let reason = format!("새 자막을 옮겨 둔 뒤 폴더를 동기화하지 못했어요: {err}");
                self.hold_plan(plan, &reason).await?;
                return ended(true);
            }
            Err(err) => {
                self.discard(staged).await;
                let reason = format!("새 자막을 옮겨 두지 못해 교체하지 않았어요: {err}");
                return ended(self.back_out(row, plan, Ok(reason)).await?);
            }
        }

        let (a, b) = (aside_at.clone(), path_at.clone());
        let swapped = blocking(move || aside::swap(&a, &b, &expected(&seen))).await;
        let unsure = match swapped {
            Swapped::Expected => None,
            Swapped::NotSwapped(err) => {
                self.discard(&aside_at).await;
                let why = match err.kind() {
                    io::ErrorKind::NotFound => Err(format!("기존 자막이 없어졌어요 ({shown})")),
                    _ => Ok(format!(
                        "기존 자막과 맞바꾸지 못해 교체하지 않았어요: {err}"
                    )),
                };
                return ended(self.back_out(row, plan, why).await?);
            }
            Swapped::SwappedBack => {
                self.discard(&aside_at).await;
                let changed = format!("기존 자막이 비교한 뒤 바뀌었어요 ({shown})");
                return ended(self.back_out(row, plan, Err(changed)).await?);
            }
            Swapped::Unsynced(err) => Some(format!(
                "기존 자막과 맞바꾼 뒤 폴더를 동기화하지 못했어요: {err}"
            )),
            Swapped::SwappedBackUnsynced(err) => Some(format!(
                "비교한 것과 다른 기존 자막을 되돌린 뒤 폴더를 동기화하지 못했어요 ({shown}): {err}"
            )),
            Swapped::NotSwappedBack(err) => Some(format!(
                "기존 자막이 비교한 것과 달라 되돌리려 했지만 하지 못했어요 ({shown}): {err}"
            )),
        };
        if let Some(reason) = unsure {
            self.hold_plan(plan, &reason).await?;
            return ended(true);
        }

        // The new copy goes by the target's name, which no file has (the
        // plan would replace that one).
        let mut path = replaced.path.clone();
        if path != plan.target {
            let (from, to) = (path_at, files::within(&folder, &plan.target));
            let renamed = blocking(move || {
                rename_noreplace(&from, &to)
                    .ok()
                    .map(|()| sync_renamed(&from, &to))
            })
            .await;
            match renamed {
                // It stays applied under the name it took.
                None => {}
                Some(Ok(())) => path = plan.target.clone(),
                Some(Err(err)) => {
                    let reason =
                        format!("새 자막의 이름을 바꾼 뒤 폴더를 동기화하지 못했어요: {err}");
                    self.hold_plan(plan, &reason).await?;
                    return ended(true);
                }
            }
        }
        let displaced = TakenOff {
            path: replaced.path.clone(),
            aside,
            applied_id: replaced.applied_id.clone(),
        };
        Ok(ControlFlow::Continue((path, Some(displaced))))
    }

    /// Step 5 of carrying out: the earlier applied copy at `path` renamed
    /// into `.trss/aside/`, replacing nothing, and checked to be the one the
    /// plan saw (`Err`: why not; the file is back on its path then, unless
    /// the reason says otherwise).
    async fn set_aside_removed(&self, plan: &Plan, path: &PlanPath) -> Result<TakenOff, String> {
        let Some(seen) = path.file.clone() else {
            return Err("옮길 기존 자막의 기록이 없어요".to_owned());
        };
        let folder = PathBuf::from(&plan.folder);
        let aside = format!("{}/{}", files::ASIDE_DIR, uuid::Uuid::new_v4());
        let (from, to) = (
            files::within(&folder, &path.path),
            files::within(&folder, &aside),
        );
        let moved = blocking(move || {
            if let Some(parent) = to.parent() {
                if let Err(err) = create_dir_all_synced(parent) {
                    return Moved::refused(err);
                }
            }
            aside::set_aside(&from, &to, &expected(&seen))
        })
        .await;
        let not_put_back = |err: io::Error| {
            format!("옮긴 기존 자막이 비교한 것과 달라 되돌리려 했지만 하지 못했어요: {err}")
        };
        match moved {
            Moved::Checked(Check::Expected) => Ok(TakenOff {
                path: path.path.clone(),
                aside,
                applied_id: path.applied_id.clone(),
            }),
            Moved::Missing => Err("기존 자막이 없어졌어요".to_owned()),
            Moved::NotMoved(err) => Err(err.to_string()),
            Moved::Unsynced(err) | Moved::Checked(Check::PutBackUnsynced(err)) => Err(format!(
                "기존 자막을 옮긴 뒤 폴더를 동기화하지 못했어요: {err}"
            )),
            Moved::Checked(Check::PutBack) => Err("기존 자막이 비교한 뒤 바뀌었어요".to_owned()),
            Moved::Checked(Check::NotPutBack(err)) => Err(not_put_back(err)),
            // Taken away after the rename: there is nothing to put back.
            Moved::Checked(Check::Gone) => Err(not_put_back(io::Error::from_raw_os_error(ENOENT))),
        }
    }

    /// Puts the earlier applied copies this run took off back on their
    /// paths, replacing nothing; the paths of those it could not.
    async fn put_back(&self, plan: &Plan, taken: &[TakenOff]) -> Vec<String> {
        let mut stuck = Vec::new();
        for t in taken {
            let folder = Path::new(&plan.folder);
            let (from, to) = (
                files::within(folder, &t.aside),
                files::within(folder, &t.path),
            );
            if blocking(move || rename_noreplace_synced(&from, &to))
                .await
                .is_err()
            {
                stuck.push(t.path.clone());
            }
        }
        stuck
    }

    /// Stops a plan before anything beside the video changed (or after it
    /// was put back) for `why` ([`Stop`]). Whether the row is settled for
    /// this run.
    async fn back_out(&self, row: &PlanRow, plan: &Plan, why: Stop) -> Result<bool, JobError> {
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

    /// Holds the plan and its row with the reason; every file stays where
    /// it is.
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

    /// Step 3 of carrying out: imports the bytes of the file at `path` the
    /// app did not manage, from that path, as the stored subtitle of the
    /// creator nobody named on the plan's episode. The bytes copied are the
    /// ones the plan saw, or nothing is imported. An import done already (a
    /// run cut short after it) is found, not made again. `Err`: why not,
    /// as [`Placer::back_out`] takes it (the plan fails, or goes stale for a
    /// file changed meanwhile); nothing of it is left then but, at worst, an
    /// unrecorded file of its bytes under the name it took, which the next
    /// import records.
    async fn import(
        &self,
        facts: &JobFacts,
        plan: &Plan,
        path: &str,
        staging: &files::Staging,
    ) -> Result<Result<(), Stop>, JobError> {
        let Some(seen) = plan.path(path).and_then(|p| p.file.clone()) else {
            return Ok(Err(Ok("계획에 없는 파일이에요".to_owned())));
        };
        let unknown = JobFacts {
            creator: None,
            ..facts.clone()
        };
        let dir = self
            .creator_dir(&unknown, "work", files::SUBTITLES_DIR.to_owned())
            .await?;
        let folder = PathBuf::from(&plan.folder);
        let from = files::within(&folder, path);
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
                let at = from.clone();
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
        // The copy staged for publishing, once written.
        let mut staged: Option<PathBuf> = None;
        for _ in 0..=super::MAX_RETARGETS {
            let choice = self
                .choose_name(&unknown, &plan.folder, &dir, &probe)
                .await?;
            let name = match choice {
                Choice::New(name) => name,
                Choice::Reuse(_, asset) => {
                    if let Some(staged) = &staged {
                        self.discard(staged).await;
                    }
                    let (w, now) = (what.clone(), self.now());
                    self.write(move |c| {
                        records::imported(c, &w, ImportedAsset::Reused(&asset), now)
                    })
                    .await?;
                    return Ok(Ok(()));
                }
                // An import cut short after publishing left the file: it is
                // recorded as the file an import publishes is.
                Choice::Adopt(name) => {
                    if let Some(staged) = &staged {
                        self.discard(staged).await;
                    }
                    return self
                        .imported(plan, &what, format!("{dir}/{name}"), &seen)
                        .await
                        .map(Ok);
                }
                Choice::Full => {
                    let reason = "보관할 이름을 정하지 못했어요".to_owned();
                    return self.import_failed(staged, Ok(reason)).await;
                }
                Choice::Unreadable(err) => {
                    let reason = format!("보관 폴더를 읽지 못했어요: {err}");
                    return self.import_failed(staged, Ok(reason)).await;
                }
            };
            let target = format!("{dir}/{name}");
            let temp = match staged.take() {
                Some(temp) => temp,
                None => {
                    let temp =
                        files::within(&folder, &self.staged(staging, &plan.folder, true).await);
                    let (source, at) = (from.clone(), temp.clone());
                    let (size, sha) = (seen.size, seen.sha256.clone());
                    // The file the plan saw, or the plan is stale.
                    let stop = match blocking(move || files::copy_to_temp(&source, &at, size, &sha))
                        .await
                    {
                        Copied::Ready(_) => None,
                        Copied::SourceChanged(_) => {
                            Some(Err(format!("기존 자막이 비교한 뒤 바뀌었어요 ({path})")))
                        }
                        Copied::Failed(err) if err.kind() == io::ErrorKind::NotFound => {
                            Some(Err(format!("기존 자막이 없어졌어요 ({path})")))
                        }
                        Copied::Failed(err) => Some(Ok(err.to_string())),
                    };
                    match stop {
                        None => temp,
                        Some(stop) => return self.import_failed(Some(temp), stop).await,
                    }
                }
            };
            let (at, to) = (temp.clone(), files::within(&folder, &target));
            match blocking(move || files::publish(&at, &to)).await {
                Published::Done => {
                    return self.imported(plan, &what, target, &seen).await.map(Ok);
                }
                Published::Occupied => staged = Some(temp),
                Published::Failed(err) => {
                    let reason = format!("보관본을 쓰지 못했어요: {err}");
                    return self.import_failed(Some(temp), Ok(reason)).await;
                }
            }
        }
        let reason = "보관할 이름이 계속 다른 파일에 먼저 쓰였어요".to_owned();
        self.import_failed(staged, Ok(reason)).await
    }

    /// An import that did not happen: its staged copy, if it has one, goes
    /// (`Err`, why).
    async fn import_failed(
        &self,
        staged: Option<PathBuf>,
        why: Stop,
    ) -> Result<Result<(), Stop>, JobError> {
        if let Some(staged) = staged {
            self.discard(&staged).await;
        }
        Ok(Err(why))
    }

    /// Records an import whose file is at `relative` with the bytes the plan
    /// saw, and tells the job's log.
    async fn imported(
        &self,
        plan: &Plan,
        what: &Imported,
        relative: String,
        seen: &FileSeen,
    ) -> Result<(), JobError> {
        let (w, at, now) = (what.clone(), relative.clone(), self.now());
        let (size, sha256) = (seen.size, seen.sha256.clone());
        self.write(move |c| {
            let asset = ImportedAsset::New {
                relative_path: &at,
                size,
                sha256: &sha256,
            };
            records::imported(c, &w, asset, now)
        })
        .await?;
        self.event(
            &plan.job_id,
            format!(
                "{}화: 관리하지 않던 자막을 '{}' 보관본으로 들였어요",
                plan.episode,
                super::UNKNOWN_CREATOR
            ),
            Some(relative),
        )
        .await
    }

    /// Whether the stored copy's file is there with the length and SHA-256
    /// recorded for it.
    async fn stored_as_recorded(&self, folder: &str, asset: &place_records::Asset) -> bool {
        let at = files::within(Path::new(folder), &asset.relative_path);
        let (size, sha256) = (asset.size, asset.sha256.clone());
        matches!(
            blocking(move || read_facts(&at)).await,
            Ok((n, sha, _)) if n == size && sha == sha256
        )
    }

    /// Holds the row with `reason` and tells the job's log, naming `path`.
    async fn hold_row(
        &self,
        row: &PlanRow,
        label: &str,
        reason: String,
        path: &str,
    ) -> Result<(), JobError> {
        self.settle_row(row, Outcome::Held, reason.clone()).await?;
        self.event(
            &row.job_id,
            format!("{label}: 보류했어요"),
            Some(format!("{reason} ({path})")),
        )
        .await
    }

    /// Records the subtitle beside the video, whose bytes are the row's stored
    /// copy's, as that copy's applied one, the file left as it is: whether it
    /// did (else the row is stored only). A row a person chose takes the file
    /// over from another stored subtitle of the same bytes; a job's own first
    /// apply does not.
    async fn adopt(
        &self,
        facts: &JobFacts,
        row: &PlanRow,
        video: &str,
        file: &Present,
        stored_id: String,
        episode: i64,
    ) -> Result<bool, JobError> {
        let what = Adopted {
            work_id: facts.work_id.clone().unwrap_or_default(),
            stored_id,
            season: facts.season.unwrap_or(0),
            episode,
            video: video.to_owned(),
            path: file.path.clone(),
            size: file.file.size,
            sha256: file.file.sha256.clone(),
            job_id: row.job_id.clone(),
            position: row.position,
            take_over: row.chosen.is_some(),
        };
        let now = self.now();
        match self
            .write(move |c| place_records::adopt(c, &what, ADOPTED, now))
            .await
        {
            Ok(Some(_)) => {
                self.event(
                    &row.job_id,
                    format!("{}: {ADOPTED}", row_label(row)),
                    Some(file.path.clone()),
                )
                .await?;
                Ok(true)
            }
            Ok(None) => Ok(false),
            // A record the database refuses: the row stays stored only.
            Err(err) if super::is_constraint(&err) => Ok(false),
            Err(err) => Err(err),
        }
    }
}
