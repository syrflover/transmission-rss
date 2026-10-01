-- Archive suggestions (docs/specs/collection.md, 규칙 보관).
--
-- A suggestion is not stored: it is read off the rule, the Anissia snapshot and
-- the history each time (`archive_suggestions`). What is stored is what the
-- suggestion needs and cannot read from anywhere else.
--
-- `rule_started` is when the app first had the rule (Unix ms). The 4 weeks
-- without a new item count from the last receive, and a rule that never
-- received anything counts from when it started collecting, which a plain
-- rule (no subscription, never turned back on) has no column for. A trigger
-- stamps every inserted rule, so no code that inserts rules has to know about
-- it. The rules from before this migration are stamped with the moment it
-- runs, so a rule that never received starts its 4 weeks at the upgrade
-- rather than at its channel's first read, which may be long past. (A rule
-- that has no row still counts from its channel's first read.) A migration that rebuilds
-- `rules` drops the triggers with it and must create them again.
--
-- `archive_suggestion_kept` is `수집 유지`: the user looked at a suggestion and
-- chose to keep collecting. `ground` identifies what the suggestion rested on
-- (`archive_suggestions::Ground::key`), so the same ground does not suggest
-- again while a new one (the anime ended, a new quiet stretch after a receive)
-- does. It goes with its rule.
--
-- `channel_read_days` is the time the `새 항목 없음` ground can count: a quiet
-- stretch is 4 weeks in which the channel was *read*, not 4 weeks on the
-- calendar, so a dead address or a stopped worker does not make every rule of
-- the channel look abandoned. The worker leaves one row per channel per day
-- (Unix ms divided by a day) on which a read of its feed worked, and keeps the
-- newest 28 of them: the 28th newest is all the ground asks for
-- (`archive_suggestions::Facts`). The days before this migration are not
-- known; each channel is taken as read on the 28 days up to its last
-- successful read (`channel_read_status.ok_at`, which the worker has kept since
-- migration 4), which is how the ground read the past before: by the clock. A
-- channel that never read successfully has none, and earns its days from the
-- upgrade on. There is no foreign key: like `channel_read_status`, the rows of
-- channels that no longer exist are dropped when the worker writes.

CREATE TABLE channel_read_days (
    channel_id TEXT    NOT NULL CHECK (channel_id <> ''),
    day        INTEGER NOT NULL,
    PRIMARY KEY (channel_id, day)
) WITHOUT ROWID;

INSERT INTO channel_read_days (channel_id, day)
WITH RECURSIVE back (n) AS (
    SELECT 0 UNION ALL SELECT n + 1 FROM back WHERE n < 27
)
SELECT s.channel_id, s.ok_at / 86400000 - back.n
FROM channel_read_status s, back
WHERE s.ok_at IS NOT NULL;

CREATE TABLE rule_started (
    rule_id    TEXT    PRIMARY KEY CHECK (rule_id <> ''),
    started_at INTEGER NOT NULL
) WITHOUT ROWID;

INSERT INTO rule_started (rule_id, started_at)
SELECT id, CAST(strftime('%s', 'now') AS INTEGER) * 1000 FROM rules;

CREATE TRIGGER rule_started_on_insert AFTER INSERT ON rules
BEGIN
    INSERT OR REPLACE INTO rule_started (rule_id, started_at)
    VALUES (NEW.id, CAST(strftime('%s', 'now') AS INTEGER) * 1000);
END;

CREATE TRIGGER rule_started_on_delete AFTER DELETE ON rules
BEGIN
    DELETE FROM rule_started WHERE rule_id = OLD.id;
END;

CREATE TABLE archive_suggestion_kept (
    rule_id TEXT    NOT NULL REFERENCES rules (id) ON DELETE CASCADE,
    ground  TEXT    NOT NULL CHECK (ground <> ''),
    kept_at INTEGER NOT NULL,
    PRIMARY KEY (rule_id, ground)
) WITHOUT ROWID;
