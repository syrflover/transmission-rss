-- A person's request to start the browser run of a remote screen anew
-- (`docs/specs/jobs.md`, 작업 화면 안의 인증과 브라우저 수명;
-- `trss_jobs::screen`).
--
-- `subtitle_job_screens.restart_run_id` is the run the person asked to start
-- anew: the web writes it only while that run and the binding the person saw
-- are still the job's, and the worker answers it with the prepare requests,
-- by ending the run and bringing the job back to the check in a new one. It is
-- `NULL` when there is none, and whenever no run is bound. Only a run ID is
-- stored here, never an address.

ALTER TABLE subtitle_job_screens ADD COLUMN restart_run_id TEXT;
