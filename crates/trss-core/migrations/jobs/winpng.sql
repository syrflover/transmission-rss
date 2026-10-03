-- What a post's WinPNG images need (`docs/specs/jobs.md`, 공통 수신 결과와 실패
-- 분류; `trss_subtitles::winpng`).
--
-- `subtitle_job_items.failure` has two classes more: `no_subtitle` (the
-- post's images were read and none holds a subtitle) and `needs_input` (an
-- image needs a key only a person can give). SQLite cannot change a column's
-- `CHECK`, so the column is made again beside the old one, filled from it,
-- and the old one dropped; the new one takes its name. The rows keep their
-- classes. A file's `failure` does not get them: no file exists for an item
-- that failed for either.
--
-- `subtitle_job_files.folder` is the folders a file is in within what the post
-- offers (a WinPNG image's), a relative path of safe names joined by `/`. A
-- file with none is published at the top of the job's folder as before; one
-- with a folder is published under it, and a restart finds it there.

ALTER TABLE subtitle_job_items ADD COLUMN failure_class TEXT
    CHECK (failure_class IN ('missing', 'expired', 'not_a_file', 'changed', 'network',
                             'no_subtitle', 'needs_input'));
UPDATE subtitle_job_items SET failure_class = failure;
ALTER TABLE subtitle_job_items DROP COLUMN failure;
ALTER TABLE subtitle_job_items RENAME COLUMN failure_class TO failure;

ALTER TABLE subtitle_job_files ADD COLUMN folder TEXT CHECK (folder IS NULL OR folder <> '');
