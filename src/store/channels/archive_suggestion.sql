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
