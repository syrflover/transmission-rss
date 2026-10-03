-- The episode mapping the user sets (`docs/specs/library.md`, 자막의 회차
-- 대응; `trss_jobs::mapping`).
--
-- `subtitle_episode_mappings.version` is what a save from the work detail
-- carries, so a save made from an older look at the row is refused. Every
-- write that changes the row (the app's, the user's save) gives it the next
-- value of `subtitle_mapping_clock`, a counter shared by all rows, so a value
-- is never given twice: a source whose row was deleted (`자동으로 되돌리기`)
-- and decided again by the app does not come back to the version a screen
-- still holds. A source with no row is version 0. The rows of earlier builds
-- are version 1, which the clock starts after.
--
-- `subtitle_episode_exceptions` are the user's per-episode exceptions of a
-- `user` mapping: Anissia's episode text `episode`, compared by `episode_key`
-- (`n:13` is `013`, `13` and `13.0`; `t:SP` any other text as written), is the
-- season's episode `target`, or `NULL` for 받지 않음 (not received). They are
-- applied before the mapping's offset. `episode` keeps the text as the user
-- wrote it. The rows go with their mapping, so `자동으로 되돌리기`, the work's
-- deletion and a merge that replaces the mapping take them along.

ALTER TABLE subtitle_episode_mappings ADD COLUMN version INTEGER NOT NULL DEFAULT 1;

CREATE TABLE subtitle_mapping_clock (
    id      INTEGER PRIMARY KEY CHECK (id = 1),
    version INTEGER NOT NULL
);
INSERT INTO subtitle_mapping_clock (id, version) VALUES (1, 1);

CREATE TABLE subtitle_episode_exceptions (
    work_id     TEXT    NOT NULL,
    season      INTEGER NOT NULL,
    source_id   TEXT    NOT NULL,
    episode_key TEXT    NOT NULL CHECK (episode_key <> ''),
    episode     TEXT    NOT NULL CHECK (episode <> ''),
    target      INTEGER CHECK (target IS NULL OR target >= 1),
    PRIMARY KEY (work_id, season, source_id, episode_key),
    FOREIGN KEY (work_id, season, source_id)
        REFERENCES subtitle_episode_mappings (work_id, season, source_id) ON DELETE CASCADE
) WITHOUT ROWID;
