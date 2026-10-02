-- Subtitle jobs (see `trss-jobs` and `docs/specs/jobs.md`, 작업 결과 and
-- 체크포인트와 중단 복구): what the user asked to receive, how far the worker
-- got, and enough about every file it touched to tell, after a restart, what
-- it can reuse and what it cannot vouch for.
--
-- `subtitle_jobs` is one job. `id` is made by the app; `command_id` is the ID
-- the browser made for the action that asked for it, with `request` the
-- request's content in canonical JSON, so a repeated delivery finds the job it
-- made instead of making a second one. `origin` says how it was asked for
-- (`pick`: candidates the user picked). `work_id`, `season`, `anime_no`,
-- `source_id` and `creator` say what it is about as it was when it was made
-- (`work_id` has no reference: a job outlives its work's folder).
--
-- `state` is where the job is: `pending` (accepted, not started), `running`
-- (a worker started it; one still `running` when a worker claims it is one an
-- earlier start did not finish), `waiting` (it cannot go on until something
-- happens; `wait` says what: `auth`, a person's check on the site, or
-- `subtitle`, a source the app cannot read yet), `held` (a restart found a file
-- whose receipt it cannot confirm, so it stopped rather than receive it again
-- or claim it), `failed`, `partial` (some items failed, the rest were
-- received) and `done`. `stage` is the step a running job is at, `note` one
-- sentence about the state, `state_at` since when it is in it, `attempts` how
-- many times a worker started it since its last run ended (0 again when a run
-- ends, so only runs cut short add up). Times are Unix milliseconds.
--
-- `subtitle_job_items` is one episode of a job: one candidate (an observation
-- of Anissia's lines) as it was picked, its post and episode text copied so the
-- item stays readable as it was asked for. Its `state` and `wait` are the
-- job's words for one item (no `partial`); `reason` says why it failed, waits
-- or is held.
--
-- `subtitle_job_steps` are the steps the job reached (`found`, `open`, `auth`,
-- `receive`), each with its state and when it got there.
--
-- `subtitle_job_events` is the job's log as the screen shows it. Like
-- everything here it holds no cookie, token or signed address.
--
-- `subtitle_job_files` is one receipt of a file into the receive area, written
-- before each effect and after it is confirmed (`intended`, then `fetched` with
-- the bytes' size, SHA-256 and the file's object (`dev:ino`), then `done` with
-- the published path; `held`, `failed`, or `abandoned` when the attempt left no
-- bytes). `id` is the attempt's ID and names its own temporary folder
-- (`temp_dir`); `path` and `temp_dir` are relative to the receive area.
-- `file_key` is the source's name for the file within its post, stable across
-- readings and free of secret values; a file of the job received once for one
-- item is recorded for another as `same_as` that receipt. `expected_size` is
-- the length the source announced before the bytes came.

CREATE TABLE subtitle_jobs (
    seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    id          TEXT    NOT NULL UNIQUE CHECK (id <> ''),
    command_id  TEXT    NOT NULL UNIQUE CHECK (command_id <> ''),
    request     TEXT    NOT NULL,
    origin      TEXT    NOT NULL CHECK (origin <> ''),
    work_id     TEXT,
    season      INTEGER,
    anime_no    INTEGER,
    source_id   TEXT    REFERENCES subtitle_sources (id),
    creator     TEXT,
    state       TEXT    NOT NULL CHECK (state IN
                    ('pending', 'running', 'waiting', 'held', 'failed', 'partial', 'done')),
    wait        TEXT    CHECK (wait IN ('auth', 'subtitle')),
    stage       TEXT    CHECK (stage IN ('found', 'open', 'auth', 'receive')),
    note        TEXT,
    attempts    INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    state_at    INTEGER NOT NULL,
    finished_at INTEGER
);

-- What the worker claims and what the lists read.
CREATE INDEX subtitle_jobs_by_state ON subtitle_jobs (state, seq);
CREATE INDEX subtitle_jobs_done ON subtitle_jobs (finished_at DESC, seq DESC) WHERE state = 'done';

CREATE TABLE subtitle_job_items (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id         TEXT    NOT NULL REFERENCES subtitle_jobs (id) ON DELETE CASCADE,
    position       INTEGER NOT NULL,
    observation_id INTEGER REFERENCES caption_observations (id),
    episode        TEXT    NOT NULL,
    post_url       TEXT    NOT NULL CHECK (post_url <> ''),
    found_at       INTEGER NOT NULL,
    state          TEXT    NOT NULL CHECK (state IN
                       ('pending', 'running', 'waiting', 'held', 'failed', 'done')),
    wait           TEXT    CHECK (wait IN ('auth', 'subtitle')),
    reason         TEXT,
    updated_at     INTEGER NOT NULL,
    UNIQUE (job_id, position)
);

CREATE TABLE subtitle_job_steps (
    job_id TEXT    NOT NULL REFERENCES subtitle_jobs (id) ON DELETE CASCADE,
    step   TEXT    NOT NULL CHECK (step IN ('found', 'open', 'auth', 'receive')),
    state  TEXT    NOT NULL CHECK (state IN ('current', 'waiting', 'done', 'failed', 'partial')),
    at     INTEGER NOT NULL,
    note   TEXT,
    PRIMARY KEY (job_id, step)
) WITHOUT ROWID;

CREATE TABLE subtitle_job_events (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id  TEXT    NOT NULL REFERENCES subtitle_jobs (id) ON DELETE CASCADE,
    at      INTEGER NOT NULL,
    message TEXT    NOT NULL,
    detail  TEXT
);

CREATE INDEX subtitle_job_events_by_job ON subtitle_job_events (job_id, id);

CREATE TABLE subtitle_job_files (
    id            TEXT    PRIMARY KEY CHECK (id <> ''),
    job_id        TEXT    NOT NULL REFERENCES subtitle_jobs (id) ON DELETE CASCADE,
    item_id       INTEGER NOT NULL REFERENCES subtitle_job_items (id) ON DELETE CASCADE,
    file_key      TEXT    NOT NULL,
    name          TEXT    NOT NULL,
    state         TEXT    NOT NULL CHECK (state IN
                      ('intended', 'fetched', 'done', 'held', 'failed', 'abandoned')),
    same_as       TEXT    REFERENCES subtitle_job_files (id),
    temp_dir      TEXT,
    expected_size INTEGER,
    size          INTEGER,
    sha256        TEXT,
    object        TEXT,
    path          TEXT,
    reason        TEXT,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
) WITHOUT ROWID;

CREATE INDEX subtitle_job_files_by_item ON subtitle_job_files (item_id, created_at);
CREATE INDEX subtitle_job_files_by_key ON subtitle_job_files (job_id, file_key);
CREATE INDEX subtitle_job_files_same_as ON subtitle_job_files (same_as) WHERE same_as IS NOT NULL;

-- A file of a job is received once, and one path holds one receipt's file
-- (the receipts that name the first one share its path).
CREATE UNIQUE INDEX subtitle_job_files_one_receipt ON subtitle_job_files (job_id, file_key)
    WHERE state = 'done' AND same_as IS NULL;
CREATE UNIQUE INDEX subtitle_job_files_one_path ON subtitle_job_files (path)
    WHERE path IS NOT NULL AND same_as IS NULL AND state <> 'abandoned';
