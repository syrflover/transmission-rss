-- The recheck of a received episode's files (`docs/specs/subtitles.md`, 구독
-- 제작자 자동 수신; `trss_jobs::recheck`): once a day for 14 days after an
-- episode of the subscribed creator was received, the worker reads again what
-- the source tells about the files without receiving them, and receives the
-- post again when a file's size (or a Drive file's modified time) differs.
--
-- `subtitle_item_rechecks` is where one received item (an episode of a job) is
-- in that check, one row per item:
-- - `checked_at` is when the item was last claimed for a reading, written
--   before the first request, so a restart or a second worker does not read it
--   again the same day. `checks` counts the readings.
-- - `result` is how the last reading came out: `same` (every file as it was
--   received), `changed` (a file differs: `job_id` is the job made to receive
--   the post again), `missing` (the post or a file is gone), `failed` (the
--   site could not be reached or answered something unexpected), `unreadable`
--   (no source of this build reads the post, or it tells nothing about the
--   file). The claim of a new reading sets `result`, `observed`, `job_id` and
--   `result_at` to `NULL`, so they are `NULL` while a reading is under way (or
--   when the worker was killed during it). A reading cut short by a shutdown
--   gives the previous reading back whole, with its `checked_at` and `checks`.
-- - `observed` is what the reading saw, as a JSON array of objects with `key`
--   (the file's key within its post, the same value `subtitle_job_files`
--   keeps), `size`, `last_modified` and, for a file that could not be read,
--   `problem` (the class of the failure) and, for a file the post offers in
--   place of one that is gone, `replacement`. It holds no address, cookie or
--   token. The next readings of an item received again with the same bytes
--   compare with these sizes.
-- The row goes with its item.
--
-- `subtitle_job_items.unchanged_from` is set on an item of a job that receives
-- a revision (`subtitle_jobs.revision_of`) when every file it received is the
-- same bytes (SHA-256) as the files of the earlier receipt of the episode:
-- the job that received them. There is nothing to replace, and no replacement
-- is to be approved for it.

ALTER TABLE subtitle_job_items ADD COLUMN unchanged_from TEXT REFERENCES subtitle_jobs (id);

CREATE TABLE subtitle_item_rechecks (
    item_id    INTEGER PRIMARY KEY REFERENCES subtitle_job_items (id) ON DELETE CASCADE,
    checked_at INTEGER NOT NULL,
    checks     INTEGER NOT NULL DEFAULT 1 CHECK (checks >= 1),
    result     TEXT    CHECK (result IN ('same', 'changed', 'missing', 'failed', 'unreadable')),
    observed   TEXT    CHECK (observed IS NULL OR json_valid(observed)),
    job_id     TEXT    REFERENCES subtitle_jobs (id),
    result_at  INTEGER
);
