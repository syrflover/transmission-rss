-- The observation of Anissia's subtitle lines (see `store::anissia::captions`
-- in `trss-collect` and `docs/specs/subtitles.md`, 자막 후보 조회).
--
-- Anissia keeps one line per anime and creator (the last episode, post and
-- update the creator entered) and overwrites it. The worker reads the lines
-- every 30 minutes (and an anime's own lines when its season is linked or the
-- user refreshes) and keeps every state it sees of a line as an observation, so
-- the episodes that passed before a season was linked stay candidates.
--
-- `subtitle_sources` is the 자막 출처 연결: the app's own ID (`id`, made by the
-- app) for a creator's lines of one Anissia anime, so that what is decided
-- about a creator's episodes later (the episode mapping) attaches to an ID of
-- the app. `creator_name` is the display name Anissia gives. It finds the
-- source of a line within `anime_no`; it is not an ID of the app, and neither
-- is a post's address. A source is made when its first line is observed and is
-- never removed.
--
-- `caption_observations` is one observation of a line, taken when the line
-- differed from the creator's previous observation (a different episode text,
-- post address or update time) or was seen for the first time. Nothing deletes
-- an observation, so a line that vanishes from Anissia's list leaves its
-- observations as they were. `episode` and `updated` are Anissia's texts as
-- written: the episode is never read as a number, and `updated` stays even
-- when it is not a date. `updated_at` (Unix ms) is the moment `updated` names,
-- an `updDt` without a zone being read as Asia/Seoul, and NULL when `updated`
-- is not a date and time (a failed reading, which the candidates say).
-- `first_seen_at` (Unix ms) is when the app first saw this state. `id` grows
-- with the time of observation, so the greatest `id` of a source is its latest.
--
-- `anissia_caption_poll` is the schedule of the 30-minute reading of the
-- recent list: `next_at` is when it is due next, `last_read_at` when it last
-- read every page to the empty one (NULL before the first time).

CREATE TABLE subtitle_sources (
    id           TEXT    PRIMARY KEY CHECK (id <> ''),
    anime_no     INTEGER NOT NULL CHECK (anime_no > 0),
    creator_name TEXT    NOT NULL CHECK (creator_name <> ''),
    created_at   INTEGER NOT NULL,
    UNIQUE (anime_no, creator_name)
) WITHOUT ROWID;

CREATE TABLE caption_observations (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id     TEXT    NOT NULL REFERENCES subtitle_sources (id),
    post_url      TEXT    NOT NULL CHECK (post_url <> ''),
    episode       TEXT    NOT NULL,
    updated       TEXT    NOT NULL,
    updated_at    INTEGER,
    first_seen_at INTEGER NOT NULL
);

CREATE INDEX caption_observations_by_source ON caption_observations (source_id, id);

CREATE TABLE anissia_caption_poll (
    id           INTEGER PRIMARY KEY CHECK (id = 1),
    next_at      INTEGER NOT NULL,
    last_read_at INTEGER
);
