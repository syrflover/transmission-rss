//! The reads the web makes of what a job placed and stored, and the decisions a
//! person makes on it: each one calls the SQL of this module's siblings.

use std::collections::HashSet;

use trss_core::{Db, Millis};

use crate::store::JobError;

/// Whether the work folder `folder` keeps its stored subtitles on disk now:
/// its `.trss/subtitles` is a folder. The work folder alone is not enough,
/// since a share not mounted can leave an empty folder at its place.
async fn folder_is_dir(folder: Option<String>) -> bool {
    match folder {
        Some(folder) => {
            let kept = crate::place::files::within(
                std::path::Path::new(&folder),
                crate::place::files::SUBTITLES_DIR,
            );
            tokio::fs::metadata(kept)
                .await
                .is_ok_and(|meta| meta.is_dir())
        }
        None => false,
    }
}

/// What the web reads of what a job placed and stored, and the decisions a
/// person makes on it, over the same database as the job records. Cheap to
/// clone.
#[derive(Clone)]
pub struct PlaceStore {
    db: Db,
}

impl PlaceStore {
    pub fn new(db: Db) -> PlaceStore {
        PlaceStore { db }
    }

    /// The latest replacement plan of each row of the job, for its detail
    /// ([`crate::place::replace`]).
    pub async fn replacements(
        &self,
        job_id: &str,
    ) -> Result<Vec<crate::place::replace::records::PlanView>, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| Ok(crate::place::replace::records::views(c, &id)?))
            .await
    }

    /// The lines of the difference a replacement plan of the job was made
    /// with, as JSON text (`{"dialogue": [...], "timing": [...]}`, see
    /// [`crate::place::replace::records::comparison_lines`]); `None` when
    /// the plan is not the job's or its contents were not compared.
    pub async fn replacement_lines(
        &self,
        job_id: &str,
        plan_id: &str,
    ) -> Result<Option<String>, JobError> {
        let (job, plan) = (job_id.to_owned(), plan_id.to_owned());
        self.db
            .run(move |c| {
                Ok(crate::place::replace::records::comparison_lines(
                    c, &job, &plan,
                )?)
            })
            .await
    }

    /// A person's decision on a replacement plan
    /// ([`crate::place::replace::records::decide`]).
    pub async fn decide_replacement(
        &self,
        job_id: &str,
        plan_id: &str,
        version: i64,
        replace: bool,
        now: Millis,
    ) -> Result<crate::place::replace::records::Decided, JobError> {
        let (job, plan) = (job_id.to_owned(), plan_id.to_owned());
        self.db
            .run(move |c| {
                crate::place::replace::records::decide(c, &job, &plan, version, replace, now)
            })
            .await
    }

    /// A person's decisions on several replacement plans of the job at once
    /// ([`crate::place::replace::records::decide_all`]).
    pub async fn decide_replacements(
        &self,
        job_id: &str,
        decisions: Vec<crate::place::replace::records::Decision>,
        now: Millis,
    ) -> Result<Vec<crate::place::replace::records::Decided>, JobError> {
        let job = job_id.to_owned();
        self.db
            .run(move |c| crate::place::replace::records::decide_all(c, &job, &decisions, now))
            .await
    }

    /// The job's placement plan ([`crate::place`]), in order.
    pub async fn plan(
        &self,
        job_id: &str,
    ) -> Result<Vec<crate::place::records::PlanRow>, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| Ok(crate::place::records::plan(c, &id)?))
            .await
    }

    /// The members the job's received archives were unpacked to
    /// ([`crate::place::unpack`]), in order.
    pub async fn members(
        &self,
        job_id: &str,
    ) -> Result<Vec<crate::place::unpack::MemberRow>, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| Ok(crate::place::unpack::job_members(c, &id)?))
            .await
    }

    /// The work's stored subtitles on an episode with no applied copy.
    pub async fn stored_only(
        &self,
        work_id: &str,
    ) -> Result<Vec<crate::place::records::StoredOnly>, JobError> {
        let id = work_id.to_owned();
        self.db
            .run(move |c| Ok(crate::place::records::stored_only(c, &id)?))
            .await
    }

    /// The work's stored subtitles on an episode, applied or not, with what a
    /// person can ask of each ([`crate::place::records::work_copies`]).
    pub async fn work_copies(
        &self,
        work_id: &str,
    ) -> Result<Vec<crate::place::records::StoredCopy>, JobError> {
        let id = work_id.to_owned();
        self.db
            .run(move |c| Ok(crate::place::records::work_copies(c, &id)?))
            .await
    }

    /// The formats in the order the work's first apply takes them
    /// ([`crate::place::records::format_order`]).
    pub async fn format_order(
        &self,
        work_id: &str,
    ) -> Result<Vec<crate::model::SubtitleFormat>, JobError> {
        let id = work_id.to_owned();
        self.db
            .run(move |c| Ok(crate::place::records::format_order(c, &id)?))
            .await
    }

    /// Asks the job that stored `stored_id` to apply it as `mode` says
    /// ([`crate::place::records::choose_stored`]).
    pub async fn choose_stored(
        &self,
        work_id: &str,
        stored_id: &str,
        mode: crate::model::Chosen,
        now: Millis,
    ) -> Result<crate::place::records::StoredChoice, JobError> {
        let (work, stored) = (work_id.to_owned(), stored_id.to_owned());
        self.db
            .run(move |c| crate::place::records::choose_stored(c, &work, &stored, mode, now))
            .await
    }

    /// What the work keeps and what a person may clean of it
    /// ([`crate::place::cleanup`]).
    pub async fn work_files(
        &self,
        work_id: &str,
    ) -> Result<crate::place::cleanup::WorkFiles, JobError> {
        use crate::place::cleanup;
        let there = self.work_folder_there(work_id).await?;
        let id = work_id.to_owned();
        self.db
            .run(move |c| {
                Ok(cleanup::WorkFiles {
                    total: cleanup::total(c, &id)?,
                    cleanable: cleanup::cleanable(c, &id, there)?,
                    cleaning: cleanup::cleaning(c, &id)?,
                })
            })
            .await
    }

    /// A person's confirming of a stored subtitle's cleanup with the files
    /// `assets` they were shown ([`crate::place::cleanup::ask`]).
    pub async fn clean_stored(
        &self,
        work_id: &str,
        stored_id: &str,
        assets: Vec<String>,
        now: Millis,
    ) -> Result<crate::place::cleanup::Asked, JobError> {
        // The folder is looked at before the transaction, not in it; one
        // that goes away before the worker's pass holds the cleanup there.
        let there = self.work_folder_there(work_id).await?;
        let (work, stored) = (work_id.to_owned(), stored_id.to_owned());
        self.db
            .run(move |c| crate::place::cleanup::ask(c, &work, &stored, &assets, there, now))
            .await
    }

    /// Whether the work's folder keeps its stored subtitles on disk now (a
    /// share not mounted, a work moved: not; see [`folder_is_dir`]).
    async fn work_folder_there(&self, work_id: &str) -> Result<bool, JobError> {
        let id = work_id.to_owned();
        let folder = self
            .db
            .run(move |c| Ok::<_, JobError>(crate::place::records::work_folder(c, &id)?))
            .await?;
        Ok(folder_is_dir(folder).await)
    }

    /// What each work with a file not removed keeps
    /// ([`crate::place::cleanup::storage`]).
    pub async fn storage(&self) -> Result<Vec<crate::place::cleanup::WorkStorage>, JobError> {
        use crate::place::cleanup;
        let folders = self
            .db
            .run(|c| Ok::<_, JobError>(cleanup::work_folders(c)?))
            .await?;
        let mut there = HashSet::new();
        for (work, folder) in folders {
            if folder_is_dir(folder).await {
                there.insert(work);
            }
        }
        self.db
            .run(move |c| Ok(cleanup::storage(c, &|work| there.contains(work))?))
            .await
    }

    /// Where each row of the job's plan has its files, by position.
    pub async fn plan_paths(
        &self,
        job_id: &str,
    ) -> Result<Vec<(i64, crate::place::records::RowPaths)>, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| Ok(crate::place::records::row_paths(c, &id)?))
            .await
    }

    /// The rows of the job a person places at its 배치 확인, and whether
    /// they are its whole plan ([`crate::place::records::placeable`]);
    /// `None` for no such job.
    pub async fn placeable(
        &self,
        job_id: &str,
    ) -> Result<Option<(Vec<crate::place::records::PlanRow>, bool)>, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                let Some(facts) = crate::place::records::job_facts(c, &id)? else {
                    return Ok(None);
                };
                let rows = crate::place::records::plan(c, &id)?;
                let (positions, whole) = crate::place::records::placeable(&facts, &rows);
                let asked = rows
                    .into_iter()
                    .filter(|r| positions.contains(&r.position))
                    .collect();
                Ok(Some((asked, whole)))
            })
            .await
    }

    /// Applies a person's 배치 확인 of the job
    /// ([`crate::place::records::confirm_placement`]); `removals` are the
    /// relocation's removals the person was shown, and `total` is the
    /// season's episode count when known.
    pub async fn confirm_placement(
        &self,
        job_id: &str,
        placings: Vec<crate::place::records::RowPlacing>,
        removals: Vec<String>,
        total: Option<u32>,
        now: Millis,
    ) -> Result<crate::place::records::Confirmed, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| {
                crate::place::records::confirm_placement(c, &id, &placings, &removals, total, now)
            })
            .await
    }

    /// The relocation's removals of the job, by the episode they take a copy
    /// off ([`crate::place::relocate::removals`]); none for another job.
    pub async fn removals(
        &self,
        job_id: &str,
    ) -> Result<Vec<crate::place::relocate::Removal>, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| Ok(crate::place::relocate::removals(c, &id)?))
            .await
    }

    /// What the relocation job came to, for its last note
    /// ([`crate::place::relocate::outcome_note`]).
    pub async fn relocation_note(&self, job_id: &str) -> Result<Option<String>, JobError> {
        let id = job_id.to_owned();
        self.db
            .run(move |c| Ok(crate::place::relocate::outcome_note(c, &id)?))
            .await
    }

    /// The library's videos and subtitles of the season, by episode
    /// ([`crate::place::records::season_files`]).
    pub async fn season_files(
        &self,
        work_id: &str,
        season: u32,
    ) -> Result<std::collections::BTreeMap<i64, crate::place::records::EpisodeFiles>, JobError>
    {
        let id = work_id.to_owned();
        self.db
            .run(move |c| Ok(crate::place::records::season_files(c, &id, season)?))
            .await
    }
}
