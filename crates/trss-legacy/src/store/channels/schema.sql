-- Channels and their download rules.
--
-- `position` orders channels among themselves and rules within a channel. It is
-- not UNIQUE: reordering rewrites several rows inside one transaction and
-- SQLite checks uniqueness per row. `version` starts at 1 and increases on each
-- change; callers must present the version they last saw.
-- `excludes` and `secret_query` are JSON arrays of strings. `url` keeps the
-- original query values, including secret ones.

CREATE TABLE channels (
    id           TEXT    PRIMARY KEY CHECK (id <> ''),
    position     INTEGER NOT NULL,
    url          TEXT    NOT NULL,
    base_dir     TEXT    NOT NULL,
    excludes     TEXT    NOT NULL CHECK (json_valid(excludes)),
    secret_query TEXT    NOT NULL CHECK (json_valid(secret_query)),
    past_search  TEXT,
    version      INTEGER NOT NULL CHECK (version >= 1)
);

CREATE INDEX channels_position ON channels (position);

CREATE TABLE rules (
    id               TEXT    PRIMARY KEY CHECK (id <> ''),
    channel_id       TEXT    NOT NULL REFERENCES channels (id),
    position         INTEGER NOT NULL,
    match_text       TEXT    CHECK (match_text IS NULL OR match_text <> ''),
    regex            INTEGER NOT NULL CHECK (regex IN (0, 1)),
    case_insensitive INTEGER NOT NULL CHECK (case_insensitive IN (0, 1)),
    directory        TEXT    NOT NULL,
    episode          INTEGER NOT NULL,
    episode_auto     INTEGER NOT NULL CHECK (episode_auto IN (0, 1)),
    state            TEXT    NOT NULL CHECK (state IN ('active', 'archived')),
    version          INTEGER NOT NULL CHECK (version >= 1)
);

CREATE INDEX rules_channel_position ON rules (channel_id, position);
