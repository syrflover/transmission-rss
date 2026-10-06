-- A video a person is asked about (`회차 확인 필요`, see `trss_library::discovery`):
-- one directly in a season folder other than `Season 00` whose name gives no
-- episode or another season's. The scan records, with its unrecognized row,
-- the file as it saw it: `size` (bytes) and `mtime_ns` (modification time,
-- nanoseconds since the Unix epoch). Both are NULL on every other row, and on
-- such a video whose size and time could not be read; rows recorded before
-- this migration get them at the next scan of their work.
--
-- `unrecognized_checks` is a person's `확인함` on such a video: it is no longer
-- asked about while a video of the same size and time is at the path. A scan
-- that finds no file at the path any more, or another one, drops the mark, so
-- a different video put there later is asked about again.

ALTER TABLE unrecognized_files ADD COLUMN size INTEGER CHECK (size >= 0);
ALTER TABLE unrecognized_files ADD COLUMN mtime_ns INTEGER;

CREATE TABLE unrecognized_checks (
    work_id    TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    path       TEXT NOT NULL CHECK (path <> ''),
    size       INTEGER NOT NULL CHECK (size >= 0),
    mtime_ns   INTEGER NOT NULL,
    checked_at INTEGER NOT NULL,
    PRIMARY KEY (work_id, path)
) WITHOUT ROWID;
