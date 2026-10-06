-- Unpacking a received archive again after this machine failed it
-- (`docs/specs/subtitles.md`, 압축 해제의 격리와 한도; `trss_jobs::place::unpack`).
--
-- A failure that is not the archive's (a full disk, the memory or time
-- limit, a child that died or did not start) is tried again, three tries in
-- all. `unpack_tries` is how many tries failed so, `unpack_failure` why the
-- last one did, and `unpack_retry_at` when the next one may go: an hour
-- after the failure, or at once once a worker started after it (`NULL`).
-- While a try is left, `unpack_error` stays `NULL`; the third failure sets it.
-- A try after such failures that the archive's own reason ends sets it too,
-- counts in `unpack_tries` and leaves no next try.

ALTER TABLE subtitle_job_files ADD COLUMN unpack_tries INTEGER NOT NULL DEFAULT 0
    CHECK (unpack_tries >= 0);
ALTER TABLE subtitle_job_files ADD COLUMN unpack_failure TEXT
    CHECK (unpack_failure IS NULL OR unpack_failure <> '');
ALTER TABLE subtitle_job_files ADD COLUMN unpack_retry_at INTEGER;

CREATE INDEX subtitle_job_files_unpack_retry ON subtitle_job_files (unpack_retry_at)
    WHERE unpack_retry_at IS NOT NULL;
