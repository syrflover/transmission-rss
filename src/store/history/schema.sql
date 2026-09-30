-- Collection history: every RSS item the worker has seen, kept indefinitely.
--
-- `channel_id` and `rule_id` are plain IDs with no foreign keys on purpose:
-- history must survive the deletion of the channel or rule it refers to.
-- `channel_label` is the channel's masked URL at the time of the last sighting,
-- so a deleted channel can still be told apart. Timestamps are Unix
-- milliseconds. `result` holds a stable code (see `HistoryResult`); it has no
-- CHECK constraint so later migrations can add codes without rebuilding the
-- table, and unknown codes are rejected on read instead.
-- `identity_key` is built by `identity_key()`: `guid:`, `link:` or `title:`
-- (the first the item has) followed by the SHA-256 of that value in hex, so
-- it holds nothing of the value.

CREATE TABLE history_items (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_id    TEXT    NOT NULL,
    channel_label TEXT    NOT NULL,
    identity_key  TEXT    NOT NULL CHECK (identity_key <> ''),
    title         TEXT    NOT NULL,
    link          TEXT    NOT NULL,
    first_seen_at INTEGER NOT NULL,
    last_seen_at  INTEGER NOT NULL,
    result        TEXT    NOT NULL,
    result_at     INTEGER NOT NULL,
    rule_id       TEXT,
    reason        TEXT,
    torrent_hash  TEXT,
    UNIQUE (channel_id, identity_key)
);

CREATE INDEX history_items_recent ON history_items (first_seen_at DESC, id DESC);
CREATE INDEX history_items_by_result ON history_items (result, first_seen_at DESC, id DESC);
CREATE INDEX history_items_by_channel ON history_items (channel_id, first_seen_at DESC, id DESC);

-- One row per change of an item's result after its first record. The
-- current state lives in `history_items`; this is the trail of how it got there.
CREATE TABLE history_changes (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id      INTEGER NOT NULL REFERENCES history_items (id),
    changed_at   INTEGER NOT NULL,
    from_result  TEXT    NOT NULL,
    to_result    TEXT    NOT NULL,
    rule_id      TEXT,
    reason       TEXT,
    torrent_hash TEXT
);

CREATE INDEX history_changes_item ON history_changes (item_id, id);

-- The single row remembering when the worker last started and finished a
-- collection cycle, so two workers do not both run the same period's cycle.
CREATE TABLE collection_cycle (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    started_at  INTEGER NOT NULL,
    finished_at INTEGER
);
