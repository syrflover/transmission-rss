-- The episode mapping decided by air time (`docs/specs/library.md`, 자막의
-- 회차 대응; `trss_jobs::mapping`).
--
-- `subtitle_episode_mappings.retired_offset` is the offset an `auto` mapping
-- had when the app took it back (the grounds came to a different offset, or to
-- none). It is set on an `undecided` row only. While it is set the app decides
-- the source again only to that same offset, so a different one is never
-- taken up by itself after one pass through `undecided`; only the user's
-- mapping ends that. It is `NULL` for every other row, and for the rows of
-- earlier builds.
--
-- `subtitle_mapping_conflicts` is the set of the source's episodes that do not
-- fit its mapping (a decimal or other text, a number outside the season's
-- episodes once offset, or, for an `auto` mapping, an episode whose own air
-- time points at another offset). `reason` says which. Such an episode is not
-- received by itself; the other episodes of the source still are. The rows of
-- one (work, season, source) are rewritten whenever the follower looks at the
-- source, in the same transaction as the mapping, so they equal the current
-- set; `found_at` stays what it was for an episode that was there already.
-- The rows go with the work.

ALTER TABLE subtitle_episode_mappings ADD COLUMN retired_offset INTEGER;

CREATE TABLE subtitle_mapping_conflicts (
    work_id   TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    season    INTEGER NOT NULL CHECK (season >= 0),
    source_id TEXT    NOT NULL REFERENCES subtitle_sources (id),
    -- The episode text as the newest observation of it wrote it.
    episode   TEXT    NOT NULL,
    reason    TEXT    NOT NULL CHECK (reason <> ''),
    found_at  INTEGER NOT NULL,
    PRIMARY KEY (work_id, season, source_id, episode)
) WITHOUT ROWID;
