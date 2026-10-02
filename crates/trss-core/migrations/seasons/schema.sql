-- Season info (see `store::seasons` and `docs/specs/library.md`, 시즌 정보): the
-- AniList entries linked to each local season, and the entries as AniList last
-- described them.
--
-- `anilist_entries` is a cache of what AniList answered for one anime entry
-- (a provider ID, never an app ID). Dates are AniList's fuzzy dates: any part
-- may be NULL. `studios` is a JSON array of the names of the main studios
-- AniList flags as animation studios, `genres` a JSON array of names, `airing`
-- a JSON array of `{episode, at}`, `at` in Unix seconds (AniList's per-episode
-- schedule, as far as it knows one), `sequels` a JSON array of the entries
-- AniList relates as `SEQUEL`. `description` is AniList's text as it came; it
-- is turned into plain text when it is shown. `fetched_at` is when the row was
-- last received; `refresh_not_before` delays the next daily refresh after a
-- failed one.
--
-- `season_info` is one row per local season that has had anything done to its
-- link: `version` grows with every change of the links (a user's or the
-- app's automatic one), so a change made from an older version changes
-- nothing. `origin` says who made the current links: `auto` (the app found the
-- entry itself, or nothing was linked yet) or `user`. `job` is the automatic
-- search still to do (`search`: look the work's folder name up and link a
-- clear match); only a newly recorded first season and a user's request set
-- it. `note` is the code of the last automatic outcome the screen explains.
--
-- A season's link outlives the season's row in `seasons` (a season folder
-- that is moved away for a moment must not lose what the user linked), and
-- goes with the work.
--
-- `season_entries` are the entries of a season in order (`position` from 0).
-- Times are Unix milliseconds.

CREATE TABLE anilist_entries (
    id                 INTEGER PRIMARY KEY CHECK (id > 0),
    romaji             TEXT,
    english            TEXT,
    native             TEXT,
    format             TEXT,
    status             TEXT,
    episodes           INTEGER CHECK (episodes IS NULL OR episodes >= 0),
    start_year         INTEGER,
    start_month        INTEGER,
    start_day          INTEGER,
    end_year           INTEGER,
    end_month          INTEGER,
    end_day            INTEGER,
    studios            TEXT    NOT NULL DEFAULT '[]',
    genres             TEXT    NOT NULL DEFAULT '[]',
    description        TEXT,
    airing             TEXT    NOT NULL DEFAULT '[]',
    sequels            TEXT    NOT NULL DEFAULT '[]',
    fetched_at         INTEGER NOT NULL,
    refresh_not_before INTEGER
);

CREATE TABLE season_info (
    work_id          TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    season           INTEGER NOT NULL CHECK (season >= 0),
    version          INTEGER NOT NULL DEFAULT 1,
    origin           TEXT    NOT NULL DEFAULT 'auto' CHECK (origin IN ('auto', 'user')),
    job              TEXT    CHECK (job IS NULL OR job = 'search'),
    job_requested_at INTEGER,
    job_attempts     INTEGER NOT NULL DEFAULT 0,
    job_not_before   INTEGER,
    note             TEXT,
    PRIMARY KEY (work_id, season),
    CHECK (job IS NULL OR job_requested_at IS NOT NULL)
) WITHOUT ROWID;

CREATE INDEX season_info_jobs ON season_info (job_requested_at, work_id, season)
    WHERE job IS NOT NULL;

CREATE TABLE season_entries (
    work_id    TEXT    NOT NULL,
    season     INTEGER NOT NULL,
    position   INTEGER NOT NULL CHECK (position >= 0),
    anilist_id INTEGER NOT NULL REFERENCES anilist_entries (id),
    PRIMARY KEY (work_id, season, position),
    UNIQUE (work_id, season, anilist_id),
    FOREIGN KEY (work_id, season) REFERENCES season_info (work_id, season) ON DELETE CASCADE
) WITHOUT ROWID;

CREATE INDEX season_entries_by_entry ON season_entries (anilist_id);

-- A season the app records for the first time, when it is the work's first
-- (the lowest-numbered season that is not season 0, the specials), gets its
-- search to do. A trigger, so every place that records seasons (a scan of the
-- worker or the web, a folder's first reading) gives a new first season its
-- search, and nothing else (a rescan of known seasons, a restart, opening a
-- screen) does. A row that exists already (the season was recorded before and
-- its folder came back) is left as it is. Nothing is created for the works and
-- seasons that exist when this migration runs.
CREATE TRIGGER season_info_for_new_first_season AFTER INSERT ON seasons
WHEN NEW.number >= 1
 AND NOT EXISTS (SELECT 1 FROM seasons s
                  WHERE s.work_id = NEW.work_id AND s.number >= 1 AND s.number < NEW.number)
BEGIN
    INSERT OR IGNORE INTO season_info (work_id, season, job, job_requested_at)
    VALUES (NEW.work_id, NEW.number, 'search',
            CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER));
END;
