-- The remote screen of a job that waits for a person's check on the site
-- (`docs/specs/jobs.md`, 작업 화면 안의 인증과 브라우저 수명; `trss_jobs::screen`).
-- The worker owns the server browser's runs; the web shows a run's page and
-- relays a person's input to it. They share what they need to know of each
-- other here and wake each other through the wake socket.
--
-- `subtitle_job_screens` is one row per job whose item the server browser
-- brought to the site's check:
-- - `item_id` is that item: one item of a job has its check on screen at a
--   time.
-- - `run_id` and `target_id` name the live browser run the worker bound to
--   the job and the page in it that shows the check; `bound_at` is when. They
--   are `NULL` when the run ended (idle, the worker restarted, the file
--   arrived) or the preparation failed (`note` says why, in a sentence). A run
--   ID is a new name for every run, so a run that ended is never confused
--   with the next one.
-- - `prepare_at` is when the web last asked the worker to prepare the screen
--   (a person opened the job's page): the worker answers it, and writes
--   `prepared_at`, by counting it as use of the live run, or by starting the
--   job again so that the browser reaches the check anew. A request is
--   unanswered while `prepare_at` is later than `prepared_at`.
-- - `input_at` is when the web last relayed a person's input to the bound run
--   (written at most every few seconds). The worker's idle end counts it as
--   activity; the screen itself is not.
--
-- No token, cookie or address is kept: the web reaches the run through the
-- browser container's launcher with a token of its own environment.

CREATE TABLE subtitle_job_screens (
    job_id      TEXT    PRIMARY KEY REFERENCES subtitle_jobs (id) ON DELETE CASCADE,
    item_id     INTEGER NOT NULL REFERENCES subtitle_job_items (id) ON DELETE CASCADE,
    run_id      TEXT    CHECK (run_id IS NULL OR run_id <> ''),
    target_id   TEXT    CHECK ((run_id IS NULL) = (target_id IS NULL)),
    bound_at    INTEGER CHECK ((run_id IS NULL) = (bound_at IS NULL)),
    note        TEXT,
    prepare_at  INTEGER,
    prepared_at INTEGER,
    input_at    INTEGER,
    updated_at  INTEGER NOT NULL
) WITHOUT ROWID;

-- What the worker's idle end and its answer to the web read.
CREATE INDEX subtitle_job_screens_by_run ON subtitle_job_screens (run_id) WHERE run_id IS NOT NULL;
