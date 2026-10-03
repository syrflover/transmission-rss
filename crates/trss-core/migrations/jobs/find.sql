-- A job a person makes to find a subtitle in the server browser
-- (`docs/specs/subtitles.md`, 직접 찾기와 자막 올리기; `trss_jobs::runner`).
--
-- A find job has `origin` `find`, the creator the person chose
-- (`source_id`, `creator`), and one item that stands for its package, with no
-- episode (`''`), no observation, and `post_url` the creator's newest post
-- the app observed for the anime, which the browser opens. While the person
-- browses on the job's remote screen the job waits as for a site's check
-- (`waiting`, `auth`) with its run bound (`subtitle_job_screens`). Each file
-- the run downloads is judged by its content as an upload's is: a kept one is
-- a receipt at `done` of the item with its `kind` (and `archive_type`), in the
-- job's folder of the receive area; a dropped one is a row of
-- `subtitle_job_dropped` with why, and its bytes are not kept.
--
-- `subtitle_jobs.finish_at` is when a person asked a find job to finish
-- receiving (`받기 끝내기`): the worker ends it once no download of its run is
-- on its way, `done` with the files it kept, or with none (받은 파일 없음).
-- Other jobs have none.

ALTER TABLE subtitle_jobs ADD COLUMN finish_at INTEGER;

-- A find job's screen follows a page the post opened (a popup) by moving
-- `subtitle_job_screens.target_id` to it. `first_target_id` is the page the
-- run was bound with (the post), written with every binding and cleared with
-- it: a screen whose `target_id` is another page shows a popup, which a
-- person may close. `close_target_id` is such a request of the web (the page
-- shown when the person asked): the worker takes it, closes that page in the
-- browser unless it is the first, and the screen goes back to the page before
-- it. Both are `NULL` for other jobs' screens and with no run bound.

ALTER TABLE subtitle_job_screens ADD COLUMN first_target_id TEXT;
ALTER TABLE subtitle_job_screens ADD COLUMN close_target_id TEXT;
