-- Moving the app's applied copies when a source's episode mapping changes
-- (재배치; `trss_jobs::place::relocate`; docs/specs/library.md, 자막의 회차
-- 대응; docs/specs/subtitles.md, 배치 확인; docs/specs/jobs.md, 체크포인트와
-- 중단 복구; ticket 0071).
--
-- A mapping change gives a mapped stored subtitle its new episode at once
-- (`subtitle_stored.episode`); an applied copy of it stays beside the video
-- it was put beside until a person confirms the relocation job
-- (`subtitle_jobs.origin = 'relocate'`) of the source. The job's plan rows
-- apply the stored subtitles on their new episodes, and its removals take
-- the copies off the old ones: one table, confirmed as a whole (배치 확인).
--
-- One removal takes off the applied copy `applied_id`, which is on `episode`
-- at `path` (as the copy's record has them), once the job's placement is
-- confirmed and before any of its rows is applied; `position` is the row
-- that applies the same stored subtitle on its new episode, if the job has
-- one. `planned` until then; `intended` (the copy is to be renamed, replacing
-- nothing, to `aside`, in the `.trss/tmp/` of the work folder `folder`);
-- `set_aside` (it was, and is the applied bytes and object there); then
-- `done` once that file is gone and the applied copy is recorded removed.
-- `kept` (`reason`: the copy stays where it is: a person changed it, its
-- stored subtitle is back on its episode, another effect uses its path, the
-- stored subtitle's file is not there as recorded, or the job was held
-- before it started) and `held` (`reason`: the app could not confirm what
-- became of it; its files stay) end it otherwise. An applied copy has one
-- removal under way at a time.
CREATE TABLE subtitle_relocations (
    id         TEXT    PRIMARY KEY CHECK (id <> ''),
    job_id     TEXT    NOT NULL REFERENCES subtitle_jobs (id) ON DELETE CASCADE,
    position   INTEGER,
    applied_id TEXT    NOT NULL REFERENCES subtitle_applied (id),
    episode    INTEGER NOT NULL CHECK (episode >= 1),
    path       TEXT    NOT NULL CHECK (path <> ''),
    state      TEXT    NOT NULL CHECK (state IN
                   ('planned', 'intended', 'set_aside', 'done', 'kept', 'held')),
    folder     TEXT,
    aside      TEXT,
    reason     TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (job_id, position) REFERENCES subtitle_job_plan (job_id, position)
        ON DELETE CASCADE,
    CHECK (state NOT IN ('intended', 'set_aside') OR (folder IS NOT NULL AND aside IS NOT NULL)),
    CHECK ((state IN ('kept', 'held')) = (reason IS NOT NULL))
) WITHOUT ROWID;

CREATE UNIQUE INDEX subtitle_relocations_one_live ON subtitle_relocations (applied_id)
    WHERE state IN ('planned', 'intended', 'set_aside');
CREATE INDEX subtitle_relocations_by_job ON subtitle_relocations (job_id);

-- A mapping change that rewrote the plan of a job a worker was running
-- (`remapped_at`): that run ends with the job back in line, so the rows are
-- looked at again with their new episodes.
ALTER TABLE subtitle_jobs ADD COLUMN remapped_at INTEGER;

-- A person's 배치 확인 of a row already stored did not move its stored
-- subtitle before this: the row is `explicit` on the person's episode, the
-- stored subtitle still `mapped` on the mapping's. The person's episode is
-- the stored subtitle's, and no mapping change moves it (the latest such
-- row's, if several name it).
UPDATE subtitle_stored
   SET episode = (SELECT p.episode FROM subtitle_job_plan p
                   WHERE p.stored_id = subtitle_stored.id AND p.assignment = 'explicit'
                     AND p.episode IS NOT NULL
                   ORDER BY p.updated_at DESC, p.job_id, p.position LIMIT 1),
       assignment = 'explicit', basis = NULL
 WHERE assignment = 'mapped' AND cleaned_at IS NULL
   AND EXISTS (SELECT 1 FROM subtitle_job_plan p
                WHERE p.stored_id = subtitle_stored.id AND p.assignment = 'explicit'
                  AND p.episode IS NOT NULL);
