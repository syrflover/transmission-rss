-- The library: what the worker and the web found in the watch folders (see
-- `store::library`).
--
-- `watch_folders.path` is the folder as the user typed it (trailing slashes
-- dropped). `baselined` is 0 until one scan has read the folder without any
-- error: files and works found by scans up to then are of unknown age
-- (`added_at` / `first_seen_at` NULL), because they may have been there
-- before the app looked. `checked_at` is the last attempt, successful or not;
-- `error` is a sentence while the last attempt could not read everything.
--
-- A work is a folder directly under a watch folder (`dir_name`); its `id` is
-- issued by the app and outlives the folder's path, so an archive move can
-- change `watch_folder_id` / `dir_name` and keep the work. `missing` marks a
-- work whose folder is not there any more; its rows are kept.
--
-- Files are kept by their path relative to the work folder, which a move of
-- the whole work folder does not change. `added_at` is the time of the scan
-- that first saw the file, or NULL (unknown); it is never read from the file.
-- An episode is kept as written (`01` and `1` are two episodes), and exists
-- only while a file of it does. Unrecognized files are the ones that could not
-- be attached to an episode, with the reason's code. Times are Unix
-- milliseconds.

CREATE TABLE watch_folders (
    id         TEXT    PRIMARY KEY CHECK (id <> ''),
    path       TEXT    NOT NULL UNIQUE CHECK (path <> ''),
    created_at INTEGER NOT NULL,
    baselined  INTEGER NOT NULL DEFAULT 0 CHECK (baselined IN (0, 1)),
    checked_at INTEGER,
    error      TEXT    CHECK (error IS NULL OR error <> '')
);

CREATE TABLE works (
    id              TEXT    PRIMARY KEY CHECK (id <> ''),
    watch_folder_id TEXT    NOT NULL REFERENCES watch_folders (id) ON DELETE CASCADE,
    dir_name        TEXT    NOT NULL CHECK (dir_name <> ''),
    first_seen_at   INTEGER,
    missing         INTEGER NOT NULL DEFAULT 0 CHECK (missing IN (0, 1)),
    UNIQUE (watch_folder_id, dir_name)
);

CREATE TABLE seasons (
    work_id TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    number  INTEGER NOT NULL CHECK (number >= 0),
    PRIMARY KEY (work_id, number)
) WITHOUT ROWID;

CREATE TABLE episodes (
    work_id TEXT    NOT NULL,
    season  INTEGER NOT NULL,
    episode TEXT    NOT NULL CHECK (episode <> ''),
    PRIMARY KEY (work_id, season, episode),
    FOREIGN KEY (work_id, season) REFERENCES seasons (work_id, number) ON DELETE CASCADE
) WITHOUT ROWID;

CREATE TABLE media_files (
    work_id  TEXT    NOT NULL,
    path     TEXT    NOT NULL CHECK (path <> ''),
    season   INTEGER NOT NULL,
    episode  TEXT    NOT NULL,
    kind     TEXT    NOT NULL CHECK (kind IN ('video', 'subtitle')),
    added_at INTEGER,
    PRIMARY KEY (work_id, path),
    FOREIGN KEY (work_id, season, episode)
        REFERENCES episodes (work_id, season, episode) ON DELETE CASCADE
) WITHOUT ROWID;

CREATE TABLE unrecognized_files (
    work_id TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    path    TEXT NOT NULL CHECK (path <> ''),
    reason  TEXT NOT NULL CHECK (reason <> ''),
    PRIMARY KEY (work_id, path)
) WITHOUT ROWID;
