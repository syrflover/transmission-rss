-- A job the process stopped in the middle of a step is `interrupted`
-- (`중단됨`): it stays where it was cut (`stage`) until a person restarts it
-- (`trss-jobs`'s `JobRun::interrupt_running` and `JobRequests::restart`;
-- docs/adr/0020-interrupted-jobs-wait-for-a-restart.md, docs/specs/jobs.md,
-- 체크포인트와 중단 복구).
--
-- Run with foreign keys off (a remake, see `db.rs`): `subtitle_jobs` is made
-- anew for the CHECK of `state`, and the tables that refer to it keep
-- referring to it by name. Its columns keep the order earlier migrations left,
-- and the number `AUTOINCREMENT` has reached carries over, so no `seq` is
-- given twice.

CREATE TABLE subtitle_jobs_next (
    seq                    INTEGER PRIMARY KEY AUTOINCREMENT,
    id                     TEXT    NOT NULL UNIQUE CHECK (id <> ''),
    command_id             TEXT    NOT NULL UNIQUE CHECK (command_id <> ''),
    request                TEXT    NOT NULL,
    origin                 TEXT    NOT NULL CHECK (origin <> ''),
    work_id                TEXT,
    season                 INTEGER,
    anime_no               INTEGER,
    source_id              TEXT    REFERENCES subtitle_sources (id),
    creator                TEXT,
    state                  TEXT    NOT NULL CHECK (state IN
                               ('pending', 'running', 'waiting', 'held', 'failed', 'partial',
                                'done', 'interrupted')),
    note                   TEXT,
    attempts               INTEGER NOT NULL DEFAULT 0,
    created_at             INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL,
    state_at               INTEGER NOT NULL,
    finished_at            INTEGER,
    revision_of            INTEGER REFERENCES caption_observations (id),
    revises_attributed     INTEGER NOT NULL DEFAULT 0 CHECK (revises_attributed IN (0, 1)),
    finish_at              INTEGER,
    wait                   TEXT    CHECK (wait IN
                               ('auth', 'subtitle', 'placement', 'approval', 'video')),
    stage                  TEXT    CHECK (stage IN
                               ('found', 'open', 'auth', 'receive', 'placement', 'store', 'apply',
                                'approval')),
    placement_confirmed_at INTEGER,
    remapped_at            INTEGER,
    movie                  INTEGER CHECK (movie IS NULL OR movie >= 1)
);
INSERT INTO subtitle_jobs_next
    (seq, id, command_id, request, origin, work_id, season, anime_no, source_id, creator,
     state, note, attempts, created_at, updated_at, state_at, finished_at, revision_of,
     revises_attributed, finish_at, wait, stage, placement_confirmed_at, remapped_at, movie)
    SELECT seq, id, command_id, request, origin, work_id, season, anime_no, source_id, creator,
           state, note, attempts, created_at, updated_at, state_at, finished_at, revision_of,
           revises_attributed, finish_at, wait, stage, placement_confirmed_at, remapped_at,
           movie
      FROM subtitle_jobs;

-- The new table's own counter ends at its largest `seq`; the old table's may
-- be higher where its last jobs were deleted.
DELETE FROM sqlite_sequence WHERE name = 'subtitle_jobs_next';
INSERT INTO sqlite_sequence (name, seq)
    SELECT 'subtitle_jobs_next', seq FROM sqlite_sequence WHERE name = 'subtitle_jobs';

DROP TABLE subtitle_jobs;
ALTER TABLE subtitle_jobs_next RENAME TO subtitle_jobs;
CREATE INDEX subtitle_jobs_by_state ON subtitle_jobs (state, seq);
CREATE INDEX subtitle_jobs_done ON subtitle_jobs (finished_at DESC, seq DESC) WHERE state = 'done';
