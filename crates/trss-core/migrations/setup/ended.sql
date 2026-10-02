-- The first run's checklist, ended for good (`docs/specs/settings.md`, 처음
-- 실행): a checklist that has ended never comes back, even if the folders or
-- channels that finished it are removed later.
--
-- Each step is latched where it happens instead of being read from the data
-- each time:
--
-- - `folder_added_at` is when a watch folder was registered. Triggers on
--   `watch_folders` write it, so every way a folder gets registered counts (the
--   settings, the import's collect folder, a folder that comes back).
-- - `import_applied_at` is when an import was applied (written by the web
--   when the import succeeds). Channels that exist for another reason do not
--   end the step.
-- - `ended_at` is when both steps were done or skipped. It is what keeps the
--   checklist away; taking back the skip of a step that is not done clears it,
--   which is how the notice shown right after the checklist disappears brings
--   the checklist back.
--
-- A database that is past the checklist by the old reading (a folder or a
-- channel exists) keeps it past: the steps those make count as done, and a
-- checklist whose steps are all done or skipped is ended.

ALTER TABLE first_run ADD COLUMN folder_added_at   INTEGER;
ALTER TABLE first_run ADD COLUMN import_applied_at INTEGER;
ALTER TABLE first_run ADD COLUMN ended_at          INTEGER;

UPDATE first_run
   SET folder_added_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000
 WHERE EXISTS (SELECT 1 FROM watch_folders WHERE unregistered_at IS NULL);

UPDATE first_run
   SET import_applied_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000
 WHERE EXISTS (SELECT 1 FROM channels);

UPDATE first_run
   SET ended_at = CAST(strftime('%s', 'now') AS INTEGER) * 1000
 WHERE (folder_added_at IS NOT NULL OR folder_skipped_at IS NOT NULL)
   AND (import_applied_at IS NOT NULL OR import_skipped_at IS NOT NULL);

CREATE TRIGGER first_run_folder_registered AFTER INSERT ON watch_folders
WHEN NEW.unregistered_at IS NULL
BEGIN
    UPDATE first_run SET folder_added_at = coalesce(folder_added_at, NEW.created_at)
     WHERE id = 1;
END;

CREATE TRIGGER first_run_folder_registered_again
    AFTER UPDATE OF unregistered_at ON watch_folders
WHEN OLD.unregistered_at IS NOT NULL AND NEW.unregistered_at IS NULL
BEGIN
    UPDATE first_run SET folder_added_at = coalesce(folder_added_at, NEW.created_at)
     WHERE id = 1;
END;
