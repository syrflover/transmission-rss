-- Work artwork (see `store::artwork` and `docs/specs/library.md`, 작품 표지).
--
-- `work_artwork` is one row per work: the selection (`settings.works[].artwork`
-- of the app YAML) and the current image (`library.works[].artwork_image`) in
-- the same row, so one version guards both.
--
-- - `mode` is `auto`, `manual` or `disabled`. `source` is `anilist` or
--   `upload` while something is selected; `anilist_media_id` is AniList's
--   positive `Media` ID (a provider ID, never an app ID) and only with
--   `source = 'anilist'`.
-- - The `image_*` columns are all set or all NULL. `image_id` is issued by the
--   app; `image_path` is relative to the app data folder (the folder of the
--   database), `image_sha256` lowercase hex. A selection keeps its image
--   reference when the file is gone or differs; serving checks the file each
--   time.
-- - `version` grows with every change of what is selected (not when the
--   image of an unchanged selection arrives, nor for jobs). A user change carries the
--   version it was made from, and an automatic result the version its job was
--   taken at; a write from an older version changes nothing.
-- - `job` is the automatic work still to do for the work: `search` (look the
--   work's folder name up on AniList and select a clear match) or `fetch`
--   (receive the image of the selected AniList ID). Only a new work, a user's
--   request and a season's link being saved while the cover is `auto` (it
--   follows the earliest season's first entry) set it; `job_image_url` is the
--   cover URL a verified search answer gave, NULL when the fetch must ask
--   AniList for it. `job_attempts`
--   and `job_not_before` space retries out. `note` is the code of the last
--   automatic outcome the screen explains (no match, ambiguous, ...).
--
-- Times are Unix milliseconds.

CREATE TABLE work_artwork (
    work_id          TEXT    PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    mode             TEXT    NOT NULL CHECK (mode IN ('auto', 'manual', 'disabled')),
    source           TEXT    CHECK (source IS NULL OR source IN ('anilist', 'upload')),
    anilist_media_id INTEGER CHECK (anilist_media_id IS NULL OR anilist_media_id > 0),
    image_id         TEXT    CHECK (image_id IS NULL OR image_id <> ''),
    image_origin     TEXT    CHECK (image_origin IS NULL OR image_origin IN ('anilist', 'upload')),
    image_path       TEXT    CHECK (image_path IS NULL OR image_path <> ''),
    image_size       INTEGER CHECK (image_size IS NULL OR image_size >= 0),
    image_sha256     TEXT    CHECK (image_sha256 IS NULL OR (length(image_sha256) = 64
                                    AND image_sha256 NOT GLOB '*[^0-9a-f]*')),
    image_format     TEXT    CHECK (image_format IS NULL OR image_format IN ('jpeg', 'png', 'webp')),
    version          INTEGER NOT NULL DEFAULT 1,
    job              TEXT    CHECK (job IS NULL OR job IN ('search', 'fetch')),
    job_requested_at INTEGER,
    job_attempts     INTEGER NOT NULL DEFAULT 0,
    job_not_before   INTEGER,
    job_image_url    TEXT,
    note             TEXT,
    note_at          INTEGER,
    -- The image columns go together.
    CHECK ((image_id IS NULL) = (image_origin IS NULL)
       AND (image_id IS NULL) = (image_path IS NULL)
       AND (image_id IS NULL) = (image_size IS NULL)
       AND (image_id IS NULL) = (image_sha256 IS NULL)
       AND (image_id IS NULL) = (image_format IS NULL)),
    -- An AniList ID only with an AniList selection, and always with one.
    CHECK ((source IS 'anilist') = (anilist_media_id IS NOT NULL)),
    -- An image's origin is the selection's source.
    CHECK (image_id IS NULL OR image_origin IS source),
    -- `disabled` selects nothing; `manual` always has a source and an image;
    -- `auto` selects only from AniList, and has an image only with a selection.
    CHECK (mode <> 'disabled' OR (source IS NULL AND image_id IS NULL)),
    CHECK (mode <> 'manual' OR (source IS NOT NULL AND image_id IS NOT NULL)),
    CHECK (mode <> 'auto' OR source IS NULL OR source = 'anilist'),
    CHECK (source IS NOT NULL OR image_id IS NULL),
    CHECK (job IS NULL OR job_requested_at IS NOT NULL),
    CHECK (job IS NOT 'fetch' OR source = 'anilist')
);

CREATE INDEX work_artwork_jobs ON work_artwork (job_requested_at, work_id)
    WHERE job IS NOT NULL;

-- A work the app records for the first time starts in `auto` with a search to
-- do. A trigger, so every place that records works (a scan of the worker or
-- the web, a folder's first reading) gives a new work its search, and nothing
-- else (a rescan, a restart, opening a screen) does.
CREATE TRIGGER work_artwork_for_new_work AFTER INSERT ON works
BEGIN
    INSERT INTO work_artwork (work_id, mode, job, job_requested_at)
    VALUES (NEW.id, 'auto', 'search',
            CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER));
END;

-- The works recorded before artwork existed get no row and no search here:
-- searches come only from a new registration or a user's request. Their
-- unselected `auto` row is made when something first reads or changes it.

-- Image files the app made in the app data folder. A row is written before the
-- file exists (`staging`) and becomes `published` in the transaction that
-- makes a selection refer to it (or that gives up on it), so a file is only
-- ever removed when this table says the app made it, its identity (`dev`,
-- `ino`, taken from the staged file, which the rename keeps) still matches, and
-- nothing refers to it. `relative_path` and `staging_path` are relative to the
-- app data folder.
CREATE TABLE artwork_files (
    relative_path TEXT    PRIMARY KEY CHECK (relative_path <> ''),
    staging_path  TEXT    NOT NULL UNIQUE CHECK (staging_path <> ''),
    state         TEXT    NOT NULL CHECK (state IN ('staging', 'published')),
    dev           INTEGER,
    ino           INTEGER,
    created_at    INTEGER NOT NULL
) WITHOUT ROWID;

-- The pace of AniList API requests, shared by the web and the worker: the next
-- request may start at `next_at`, and none before `blocked_until` (set from a
-- `429` answer's `Retry-After`).
CREATE TABLE anilist_pace (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    next_at       INTEGER NOT NULL,
    blocked_until INTEGER
);
