-- The first run (`docs/specs/settings.md`, 처음 실행): the `처음 설정`
-- checklist that takes the place of the weekly schedule on a new install.
--
-- `first_run` has a row only while the install is one that began empty: the
-- migration that adds the table writes it when the database has no channel and
-- no registered watch folder, and nothing writes it later. An install that had
-- either is not a first run, so the checklist never shows for it. The row keeps
-- the steps the user skipped (Unix milliseconds; `folder` is `감시 폴더 등록`
-- and `import` is `기존 설정 가져오기`; NULL when not skipped), on the server
-- so every device sees the same checklist. (`ended.sql` later keeps the steps
-- that were done and the end of the checklist as well.)
CREATE TABLE first_run (
    id                INTEGER PRIMARY KEY CHECK (id = 1),
    folder_skipped_at INTEGER,
    import_skipped_at INTEGER
);

INSERT INTO first_run (id)
SELECT 1
 WHERE NOT EXISTS (SELECT 1 FROM channels)
   AND NOT EXISTS (SELECT 1 FROM watch_folders WHERE unregistered_at IS NULL);
