-- A rule can be paused (`영상 받기` off): it keeps its place and its work
-- folder, and collects nothing until it is turned on again. SQLite cannot
-- change a CHECK constraint in place, so `rules` is rebuilt with the wider one.
--
-- `rule_subscriptions` has a foreign key to `rules` with ON DELETE CASCADE:
-- dropping `rules` would delete every subscription with it, so the
-- subscriptions are put aside and put back (the DELETE first, so the copy does
-- not meet the rows a connection without foreign keys would have kept).
--
-- `rule_subscriptions.season_blocked` is set when the season the rule's videos
-- are in is already connected to another Anissia anime: it holds that season's
-- ID (see `store::channels::SeasonRef`), and the rule's detail says so. It is
-- empty while the rule has no such season or has one connected.

ALTER TABLE rule_subscriptions
    ADD COLUMN season_blocked TEXT CHECK (season_blocked IS NULL OR season_blocked <> '');

CREATE TEMP TABLE rule_subscriptions_kept AS SELECT * FROM rule_subscriptions;
DELETE FROM rule_subscriptions;

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
    state            TEXT    NOT NULL CHECK (state IN ('active', 'paused', 'archived')),
    version          INTEGER NOT NULL CHECK (version >= 1)
);

INSERT INTO rules_rebuilt
    (id, channel_id, position, match_text, regex, case_insensitive, directory, episode,
     episode_auto, state, version)
SELECT id, channel_id, position, match_text, regex, case_insensitive, directory, episode,
       episode_auto, state, version
  FROM rules;

DROP TABLE rules;
ALTER TABLE rules_rebuilt RENAME TO rules;
CREATE INDEX rules_channel_position ON rules (channel_id, position);

INSERT INTO rule_subscriptions SELECT * FROM rule_subscriptions_kept;
DROP TABLE rule_subscriptions_kept;
