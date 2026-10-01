-- What the app keeps of a rule's episode offset when it sets it by itself
-- (`docs/specs/collection.md`, 영상 회차 변환).
--
-- `episode_basis` is the sentence the rule's detail shows beside `자동`. It is
-- kept with the value because the AniList counts and the library it came from
-- can change afterwards, and the sentence has to say why the value was set,
-- not why it would be set now. NULL for a rule whose offset the user typed
-- (`episode_auto` = 0), and for a rule whose automatic offset came in a file
-- that carries no sentence. A save that changes the offset, or turns
-- `episode_auto` off, clears it.
--
-- `episode_previous` is the offset the rule had just before the app set its
-- own (`되돌리기` puts it back). It goes with the automatic value: NULL once
-- the value is the user's (a save that changes it, `적용`, `되돌리기`), and
-- for an automatic value that came in a file. It is never exported: it only
-- tells how to rename the videos this database's worker named.
--
-- `episode_decided` is 1 once the app has set the rule's offset (or the rule
-- came in with an automatic one). The app decides a rule once: after the user
-- changed or undid its value, it does not decide again, even for a rule that
-- has not picked anything yet.
ALTER TABLE rules ADD COLUMN episode_basis TEXT CHECK (episode_basis IS NULL OR episode_basis <> '');
ALTER TABLE rules ADD COLUMN episode_previous INTEGER;
ALTER TABLE rules ADD COLUMN episode_decided INTEGER NOT NULL DEFAULT 0
    CHECK (episode_decided IN (0, 1));
UPDATE rules SET episode_decided = 1 WHERE episode_auto = 1;

-- `되돌리기` of an automatic offset (`episode_undo` commands): the worker puts
-- the previous value back and renames the videos the rule received under the
-- automatic one. `episode_undos` is written together with that value, in one
-- transaction, so a command that stopped half-way knows it already began and
-- carries on with its files instead of finding the rule changed. `from_offset`
-- is the automatic value and `to_offset` the value put back. Times are Unix ms.
--
-- `episode_undo_files` is one row per video to rename, planned at that moment:
-- the history item it was received for, its folder, the name it has under the
-- automatic value and the one it takes, the torrent that held it then (renamed
-- through Transmission while that torrent is there) and the file's identity
-- (`revision::FileIdentity`, checked before a rename on disk). `state` is
-- `pending` until the worker renamed it (`renamed`) or left it as it is
-- (`kept`, with the `reason` the screen shows). A rename's row and the
-- `video_revisions` rows of the episode, which move to the new name with it,
-- are written in one transaction.
CREATE TABLE episode_undos (
    command_id  TEXT    PRIMARY KEY CHECK (command_id <> ''),
    rule_id     TEXT    NOT NULL,
    from_offset INTEGER NOT NULL,
    to_offset   INTEGER NOT NULL,
    started_at  INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE episode_undo_files (
    command_id   TEXT    NOT NULL REFERENCES episode_undos (command_id),
    item_id      INTEGER NOT NULL,
    folder       TEXT    NOT NULL CHECK (folder <> ''),
    from_name    TEXT    NOT NULL CHECK (from_name <> ''),
    to_name      TEXT    NOT NULL CHECK (to_name <> ''),
    torrent_hash TEXT,
    identity     TEXT,
    state        TEXT    NOT NULL CHECK (state IN ('pending', 'renamed', 'kept')),
    reason       TEXT,
    PRIMARY KEY (command_id, item_id)
);
