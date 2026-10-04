-- The browser controls of a remote screen (`docs/specs/jobs.md`, 작업 화면 안의
-- 인증과 브라우저 수명; `trss_jobs::screen`).
--
-- `subtitle_job_screens.pages` is the pages of the bound run a person may see
-- as tabs: a JSON array of DevTools target IDs in the order the pages came,
-- written by the worker (`NULL` with no run bound). The web names a page in a
-- request only when it is in this list, and reads the title and the host of
-- each from the browser itself. Only target IDs are stored here, never an
-- address or a title.
--
-- `switch_target_id` is a person's request to show another page of the run:
-- the web writes it, the worker takes it and moves the screen there. It is
-- `NULL` when there is none, and with no run bound.
--
-- `close_target_id` (migration 46) may now name several pages, separated by
-- one space: a person may close more than one tab before the worker looks.
-- Any page of the run but its first may be closed, shown or not, for a find
-- job's screen and for a check's.

ALTER TABLE subtitle_job_screens ADD COLUMN pages TEXT;
ALTER TABLE subtitle_job_screens ADD COLUMN switch_target_id TEXT;
