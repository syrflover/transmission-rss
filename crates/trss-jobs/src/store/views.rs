//! What the lists of jobs and a job's detail show.

use rusqlite::{params, Connection};

use super::{
    rows::{dropped_rows, items, rows, steps, WAITS_FOR_CHECK, WAITS_FOR_PLACEMENT},
    DonePage, EventRow, ItemRow, JobDetail, JobError, JobRow, JobViews, OpenGroups, Pick,
};

impl JobViews {
    /// The jobs that are not done, oldest first.
    pub async fn open_jobs(&self) -> Result<Vec<JobRow>, JobError> {
        self.db
            .run(|c| rows(c, "WHERE j.state <> 'done' ORDER BY j.seq", []))
            .await
    }

    /// The jobs that are not done, in the groups the job list shows them in.
    pub async fn open_groups(&self) -> Result<OpenGroups, JobError> {
        Ok(OpenGroups::of(self.open_jobs().await?))
    }

    /// The jobs waiting for a person's check ([`JobRow::waits_for_check`]),
    /// oldest first.
    pub async fn auth_waits(&self) -> Result<Vec<JobRow>, JobError> {
        self.db
            .run(|c| rows(c, &format!("WHERE {WAITS_FOR_CHECK} ORDER BY j.seq"), []))
            .await
    }

    /// The jobs waiting for a person to say which episode a file is
    /// ([`JobRow::waits_for_placement`]), oldest first.
    pub async fn placement_waits(&self) -> Result<Vec<JobRow>, JobError> {
        self.db
            .run(|c| {
                rows(
                    c,
                    &format!("WHERE {WAITS_FOR_PLACEMENT} ORDER BY j.seq"),
                    [],
                )
            })
            .await
    }

    /// The jobs waiting for a person to approve or refuse a replacement
    /// (`교체 승인`), oldest first.
    pub async fn approval_waits(&self) -> Result<Vec<JobRow>, JobError> {
        self.db
            .run(|c| {
                rows(
                    c,
                    "WHERE j.state = 'waiting' AND j.wait = 'approval' ORDER BY j.seq",
                    [],
                )
            })
            .await
    }

    /// Up to `limit` done jobs after `after` (a page's `next`), newest first.
    /// A cursor that cannot be read starts from the newest.
    pub async fn done_page(
        &self,
        after: Option<String>,
        limit: usize,
    ) -> Result<DonePage, JobError> {
        self.db
            .run(move |c| done_page(c, after.as_deref(), limit))
            .await
    }

    pub async fn detail(&self, id: &str) -> Result<Option<JobDetail>, JobError> {
        let id = id.to_owned();
        self.db.run(move |c| detail(c, &id)).await
    }

    pub async fn items(&self, job_id: &str) -> Result<Vec<ItemRow>, JobError> {
        let id = job_id.to_owned();
        self.db.run(move |c| items(c, &id)).await
    }

    /// The candidates the jobs of Anissia anime `anime_no` took, in the order
    /// they were taken.
    pub async fn picks_of_anime(&self, anime_no: i64) -> Result<Vec<Pick>, JobError> {
        self.db
            .run(move |c| {
                let mut stmt = c.prepare_cached(
                    "SELECT i.observation_id, j.source_id, i.episode, i.post_url, i.state,
                            i.wait, j.id, j.state, i.updated_at
                       FROM subtitle_job_items i
                       JOIN subtitle_jobs j ON j.id = i.job_id
                      WHERE j.anime_no = ?1 AND i.observation_id IS NOT NULL
                      ORDER BY i.id",
                )?;
                let rows = stmt
                    .query_map([anime_no], |r| {
                        Ok(Pick {
                            observation_id: r.get(0)?,
                            source_id: r.get(1)?,
                            episode: r.get(2)?,
                            post_url: r.get(3)?,
                            item_state: r.get(4)?,
                            item_wait: r.get(5)?,
                            job_id: r.get(6)?,
                            job_state: r.get(7)?,
                            updated_at: r.get(8)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
    }
}

fn done_page(c: &Connection, after: Option<&str>, limit: usize) -> Result<DonePage, JobError> {
    let cursor = after.and_then(|a| {
        let (at, seq) = a.split_once('.')?;
        Some((at.parse::<i64>().ok()?, seq.parse::<i64>().ok()?))
    });
    let (at, seq) = cursor.unwrap_or((i64::MAX, i64::MAX));
    let limit = limit.max(1);
    let mut items = rows(
        c,
        "WHERE j.state = 'done' AND (j.finished_at, j.seq) < (?1, ?2)
         ORDER BY j.finished_at DESC, j.seq DESC LIMIT ?3",
        params![at, seq, (limit + 1) as i64],
    )?;
    let next = match items.len() > limit {
        true => {
            items.truncate(limit);
            items
                .last()
                .map(|j| format!("{}.{}", j.finished_at.unwrap_or(0), j.seq))
        }
        false => None,
    };
    let total: i64 = c
        .prepare_cached("SELECT count(*) FROM subtitle_jobs WHERE state = 'done'")?
        .query_row([], |r| r.get(0))?;
    Ok(DonePage {
        items,
        next,
        total: total as usize,
    })
}

fn detail(c: &mut Connection, id: &str) -> Result<Option<JobDetail>, JobError> {
    // One snapshot, so the steps, items and log agree with the row.
    let tx = c.transaction()?;
    let c = &*tx;
    let Some(row) = rows(c, "WHERE j.id = ?1", [id])?.pop() else {
        return Ok(None);
    };
    let mut stmt = c.prepare_cached(
        "SELECT at, message, detail FROM subtitle_job_events
         WHERE job_id = ?1 ORDER BY id DESC",
    )?;
    let events = stmt
        .query_map([id], |r| {
            Ok(EventRow {
                at: r.get(0)?,
                message: r.get(1)?,
                detail: r.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let detail = JobDetail {
        steps: steps(c, id)?,
        items: items(c, id)?,
        dropped: dropped_rows(c, id)?,
        events,
        row,
    };
    drop(stmt);
    tx.commit()?;
    Ok(Some(detail))
}
