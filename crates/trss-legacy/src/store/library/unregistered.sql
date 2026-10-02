-- Watch folders that are no longer registered, kept with their works (see
-- `store::library`). Unregistering a watch folder, by hand or because the
-- collection settings stopped using an automatic one, takes its works out of
-- the library without forgetting them: `unregistered_at` is set (Unix
-- milliseconds) and the row stays with every work under it, so their IDs, and
-- what is linked to them by ID (covers, season links), stay. Such a folder is
-- not read, listed or watched, and its works are in no list, screen or queue.
-- Registering the same path again (`path` as stored) clears it: the folder
-- comes back under its own ID, and its works are found again by folder name.

ALTER TABLE watch_folders ADD COLUMN unregistered_at INTEGER;
