-- A person confirms the placement plan of an upload or a find job before any
-- of it is kept (`docs/specs/subtitles.md`, 배치 확인; `trss_jobs::place`).
--
-- `placement_confirmed_at` is when the person applied the job's 배치 확인
-- table; `NULL` until then. Until it is set, such a job is analysed and
-- waits (`state = 'waiting'`, `wait = 'placement'`) with nothing written to
-- the work folder.

ALTER TABLE subtitle_jobs ADD COLUMN placement_confirmed_at INTEGER;

-- The uploads and find jobs earlier builds finished without placing what
-- they received go back in line: the worker analyses their receipts, which
-- are still in the receive area, and they wait for the person's 배치 확인.
-- A job that kept nothing (a find job with no file, an upload whose files
-- are gone) stays as it is.
UPDATE subtitle_jobs
   SET state = 'pending', wait = NULL, stage = NULL, finished_at = NULL,
       note = '받아 둔 파일의 배치 확인을 준비해요',
       state_at = CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
       updated_at = CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
 WHERE origin IN ('upload', 'find')
   AND state IN ('done', 'partial')
   AND NOT EXISTS (SELECT 1 FROM subtitle_job_plan p WHERE p.job_id = subtitle_jobs.id)
   AND EXISTS (SELECT 1 FROM subtitle_job_files f
                WHERE f.job_id = subtitle_jobs.id AND f.state = 'done'
                  AND f.cleared_at IS NULL);
