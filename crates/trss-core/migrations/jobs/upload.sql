-- Subtitles and fonts a person uploads (`docs/specs/subtitles.md`, 직접 찾기와
-- 자막 올리기; `trss_jobs::upload`).
--
-- An upload makes one job with `origin` `upload`, already `done` (there is
-- nothing to fetch): its files are one subtitle package. The job has one item
-- that stands for the package, with no episode (`''`) and `post_url` `upload:`
-- (there is no post), and its files are receipts at `done` whose `path` is in
-- the job's folder of the receive area. `file_key` is the file's place in what
-- was uploaded (a folder's relative path), `name` the same as shown.
--
-- `subtitle_job_files.kind` says what the upload's content check judged the
-- file to be: `subtitle` (ASS, SRT, SMI or another format that is only
-- stored), `font`, or `archive` (a ZIP, kept as it is for the analysis). The
-- files other jobs received have none.
--
-- `subtitle_job_dropped` is the files an upload did not keep, with why:
-- their names only, as the person sees them in the job. The files themselves
-- are not stored.

ALTER TABLE subtitle_job_files ADD COLUMN kind TEXT
    CHECK (kind IN ('subtitle', 'font', 'archive'));

CREATE TABLE subtitle_job_dropped (
    job_id   TEXT    NOT NULL REFERENCES subtitle_jobs (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    name     TEXT    NOT NULL CHECK (name <> ''),
    reason   TEXT    NOT NULL CHECK (reason <> ''),
    PRIMARY KEY (job_id, position)
) WITHOUT ROWID;
