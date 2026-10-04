-- A received subtitle whose episode has no video waits for the video
-- (`docs/specs/subtitles.md`, 영상 대기; `trss_jobs::place`).
--
-- A plan row to apply that found no video (`action = 'apply'`,
-- `outcome = 'no_video'`) is no longer settled as stored only: it waits for
-- the video, and its job applies it once the library records a video for the
-- episode. Such a job waits too (`state = 'waiting'`, `wait = 'video'`) unless
-- something else comes first: a job that received only part of its posts, or
-- some of whose rows failed, stays `partial` while its row waits. The rows
-- earlier builds settled so take the new note, and their finished jobs go
-- back in line: the worker takes them up again from their receipts, applies
-- what has a video now, and settles the rest.

UPDATE subtitle_job_plan
   SET note = '영상이 아직 없어 영상이 들어오면 적용해요'
 WHERE action = 'apply' AND outcome = 'no_video';

UPDATE subtitle_jobs
   SET state = 'pending', wait = NULL, stage = NULL, finished_at = NULL,
       note = '영상이 없는 회차의 자막을 영상 대기로 옮겨요',
       state_at = CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
       updated_at = CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
 WHERE state IN ('done', 'partial')
   AND EXISTS (SELECT 1 FROM subtitle_job_plan p
                WHERE p.job_id = subtitle_jobs.id
                  AND p.action = 'apply' AND p.outcome = 'no_video');
