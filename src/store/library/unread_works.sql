-- Work folders that a scan saw but could not read, and the library has no work
-- for yet.
--
-- A file's added time is the time of the scan that first saw it, or NULL when
-- the app cannot know. A work folder that could not be read when it first
-- showed up may have held its files for a long time, so when a later scan reads
-- it, its files have no added time, and the work is dated by the scan that
-- first saw the folder (`seen_at`, NULL when that was the folder's first scan,
-- which is not a moment the work appeared). The row goes when the work is read
-- or its folder is gone. This replaces holding the whole watch folder back
-- until every work folder is readable: one folder that never can be read must
-- not make every later file of the others unknown.

CREATE TABLE unread_works (
    watch_folder_id TEXT    NOT NULL REFERENCES watch_folders (id) ON DELETE CASCADE,
    dir_name        TEXT    NOT NULL CHECK (dir_name <> ''),
    seen_at         INTEGER,
    PRIMARY KEY (watch_folder_id, dir_name)
) WITHOUT ROWID;
