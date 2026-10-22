-- A rule is `active` or `paused`: `보관` deletes it (docs/adr/0022), and the
-- archived rules were deleted by the migration before this one. SQLite cannot
-- change a CHECK constraint in place, so `rules` is made anew with the
-- narrower one, as it was for `paused`. This runs with foreign keys off, so
-- dropping `rules` takes none of the rows that refer to it; its index and
-- triggers go with it and are made again.

CREATE TABLE rules_rebuilt (
    id               TEXT    PRIMARY KEY CHECK (id <> ''),
    channel_id       TEXT    NOT NULL REFERENCES channels (id),
    position         INTEGER NOT NULL,
    match_text       TEXT    CHECK (match_text IS NULL OR match_text <> ''),
    regex            INTEGER NOT NULL CHECK (regex IN (0, 1)),
    case_insensitive INTEGER NOT NULL CHECK (case_insensitive IN (0, 1)),
    directory        TEXT    NOT NULL,
    episode          INTEGER NOT NULL,
    episode_auto     INTEGER NOT NULL CHECK (episode_auto IN (0, 1)),
    state            TEXT    NOT NULL CHECK (state IN ('active', 'paused')),
    version          INTEGER NOT NULL CHECK (version >= 1),
    resumed_at       INTEGER,
    episode_basis    TEXT    CHECK (episode_basis IS NULL OR episode_basis <> ''),
    episode_previous INTEGER,
    episode_decided  INTEGER NOT NULL DEFAULT 0 CHECK (episode_decided IN (0, 1))
);

INSERT INTO rules_rebuilt
    (id, channel_id, position, match_text, regex, case_insensitive, directory, episode,
     episode_auto, state, version, resumed_at, episode_basis, episode_previous, episode_decided)
SELECT id, channel_id, position, match_text, regex, case_insensitive, directory, episode,
       episode_auto, state, version, resumed_at, episode_basis, episode_previous, episode_decided
  FROM rules;

DROP TABLE rules;
ALTER TABLE rules_rebuilt RENAME TO rules;
CREATE INDEX rules_channel_position ON rules (channel_id, position);

CREATE TRIGGER rule_started_on_insert AFTER INSERT ON rules
BEGIN
    INSERT OR REPLACE INTO rule_started (rule_id, started_at)
    VALUES (NEW.id, CAST(strftime('%s', 'now') AS INTEGER) * 1000);
END;

CREATE TRIGGER rule_started_on_delete AFTER DELETE ON rules
BEGIN
    DELETE FROM rule_started WHERE rule_id = OLD.id;
END;
