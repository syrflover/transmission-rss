-- A subtitle job no longer records its file effects before it makes them, and
-- a restart no longer compares such records with the disk: a step cut short
-- is done again from its start (ticket lighter-structure/0153; docs/adr/0020-interrupted-jobs-
-- wait-for-a-restart.md; docs/specs/jobs.md, 체크포인트와 중단 복구). What
-- only those records held goes, and the rows an earlier release left under
-- way end as the current code would leave them. No file is touched: what
-- earlier releases moved aside stays in the work folders' `.trss/tmp/`.
--
-- Run with foreign keys off (a remake, see `db.rs`): `subtitle_relocations`
-- is made anew without `folder` and `aside`, which a CHECK names, so the
-- columns cannot be dropped in place. No table refers to it. The commit
-- checks that nothing here broke a reference, and with foreign keys off the
-- deletes below cascade to nothing, so they remove the rows that would
-- refer to what they delete themselves. The states' CHECKs stay as they
-- are: no row is left in a state nothing writes any more.

-- Whether a row's store made its file (`made`: a new file, 0 for one kept
-- before with the same bytes, which the job's detail tells `새로 받음` from
-- `받아서 같음` by) is what the records of a store's effect told: a `done`
-- `store` effect of the row. The store records it from now on in the
-- transaction that records its asset. The effects' rows are read here, before
-- they go.
ALTER TABLE subtitle_job_plan ADD COLUMN made INTEGER NOT NULL DEFAULT 0
    CHECK (made IN (0, 1));
UPDATE subtitle_job_plan
   SET made = 1
 WHERE EXISTS (SELECT 1 FROM subtitle_file_effects e
                WHERE e.job_id = subtitle_job_plan.job_id
                  AND e.position = subtitle_job_plan.position
                  AND e.kind = 'store' AND e.state = 'done');

-- The effects of storing, applying and replacing (`store_apply.sql`,
-- `replacement.sql`), with their indexes. Their temporary files stay where
-- they are.
DROP TABLE subtitle_file_effects;

-- A receipt still being received (`intended`), or received and not yet
-- published (`fetched`, written by releases before lighter-structure/0153), is received anew
-- by its job's next run, which removes its attempt's bytes. Such a receipt
-- has no members, and no plan row, receipt sharing it (`same_as`) or later
-- volume (`volume_of`) names it: those are made from `done` receipts only.
-- One named all the same is kept as `abandoned` (an attempt replaced by the
-- next), with its path, so no reference breaks and no bytes it names go.
UPDATE subtitle_job_files
   SET state = 'abandoned'
 WHERE state IN ('intended', 'fetched')
   AND (EXISTS (SELECT 1 FROM subtitle_job_plan p WHERE p.file_id = subtitle_job_files.id)
        OR EXISTS (SELECT 1 FROM subtitle_job_files f
                    WHERE f.same_as = subtitle_job_files.id
                       OR f.volume_of = subtitle_job_files.id));
DELETE FROM subtitle_job_members
 WHERE file_id IN (SELECT id FROM subtitle_job_files WHERE state IN ('intended', 'fetched'));
DELETE FROM subtitle_job_files WHERE state IN ('intended', 'fetched');

-- A failed receipt no longer names its bytes: the job's next run removes
-- every file in its folder of the receive area that no receipt names.
UPDATE subtitle_job_files SET path = NULL WHERE state = 'failed' AND path IS NOT NULL;

-- What a restart compared a receipt with (its attempt's folder, the length
-- its source gave and the file's object), and an applied copy's object,
-- which nothing read.
ALTER TABLE subtitle_job_files DROP COLUMN temp_dir;
ALTER TABLE subtitle_job_files DROP COLUMN expected_size;
ALTER TABLE subtitle_job_files DROP COLUMN object;
ALTER TABLE subtitle_applied DROP COLUMN object;

-- A relocation's removal an earlier release left under way (`intended`, the
-- copy to be moved into `.trss/tmp/`, or `set_aside`, moved there) or held
-- ends `kept`: it is not taken off again, and its file stays where that
-- release left it. A held one keeps why it was held; the others say they
-- were cut. A later relocation may plan its copy again.
UPDATE subtitle_relocations
   SET state = 'kept', reason = '이전 버전에서 지우다 끊겨 그대로 뒀어요'
 WHERE state IN ('intended', 'set_aside');
UPDATE subtitle_relocations SET state = 'kept' WHERE state = 'held';

CREATE TABLE subtitle_relocations_next (
    id         TEXT    PRIMARY KEY CHECK (id <> ''),
    job_id     TEXT    NOT NULL REFERENCES subtitle_jobs (id) ON DELETE CASCADE,
    position   INTEGER,
    applied_id TEXT    NOT NULL REFERENCES subtitle_applied (id),
    episode    INTEGER NOT NULL CHECK (episode >= 1),
    path       TEXT    NOT NULL CHECK (path <> ''),
    state      TEXT    NOT NULL CHECK (state IN
                   ('planned', 'intended', 'set_aside', 'done', 'kept', 'held')),
    reason     TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (job_id, position) REFERENCES subtitle_job_plan (job_id, position)
        ON DELETE CASCADE,
    CHECK ((state IN ('kept', 'held')) = (reason IS NOT NULL))
) WITHOUT ROWID;
INSERT INTO subtitle_relocations_next
    (id, job_id, position, applied_id, episode, path, state, reason, created_at, updated_at)
    SELECT id, job_id, position, applied_id, episode, path, state, reason, created_at,
           updated_at
      FROM subtitle_relocations;
DROP TABLE subtitle_relocations;
ALTER TABLE subtitle_relocations_next RENAME TO subtitle_relocations;
CREATE UNIQUE INDEX subtitle_relocations_one_live ON subtitle_relocations (applied_id)
    WHERE state IN ('planned', 'intended', 'set_aside');
CREATE INDEX subtitle_relocations_by_job ON subtitle_relocations (job_id);
