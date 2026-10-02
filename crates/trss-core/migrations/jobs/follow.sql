-- The subscribed creator's subtitles received without a pick
-- (`docs/specs/subtitles.md`, 구독 제작자 자동 수신; `trss_jobs::follow`), and
-- the episode mapping of a subtitle source (`docs/specs/library.md`, 자막의
-- 회차 대응; `trss_jobs::mapping`).
--
-- `subtitle_jobs`:
-- - A job the app made itself has `origin` `auto` and `command_id`
--   `auto:<observation id>`, so a second look at the same observation finds
--   the job it made instead of making another.
-- - `revision_of` is set on a job that receives a revision of the creator's
--   subtitle the app holds: the observation whose subtitle was received
--   before (`NULL` for every other job, and for the jobs of earlier builds).
--   The job only receives; the subtitle in place stays until a replacement is
--   approved.
--
-- `subtitle_episode_mappings` is the default mapping of one subtitle source's
-- episodes to a season of a work: Anissia's episode `n` is the season's
-- episode `n + episode_offset`. `kind` says who decided it: `auto` the app,
-- from grounds that agree (`evidence` says which), `undecided` the app found
-- no such grounds (`evidence` says why, and there is no offset), `user` the
-- user (the app never changes such a row). A source with no row has not been
-- looked at. The row goes with the work.

ALTER TABLE subtitle_jobs ADD COLUMN revision_of INTEGER REFERENCES caption_observations (id);

CREATE TABLE subtitle_episode_mappings (
    work_id        TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    season         INTEGER NOT NULL CHECK (season >= 0),
    source_id      TEXT    NOT NULL REFERENCES subtitle_sources (id),
    kind           TEXT    NOT NULL CHECK (kind IN ('auto', 'undecided', 'user')),
    episode_offset INTEGER,
    evidence       TEXT    NOT NULL CHECK (evidence <> ''),
    decided_at     INTEGER NOT NULL,
    PRIMARY KEY (work_id, season, source_id),
    CHECK ((kind = 'undecided') = (episode_offset IS NULL))
) WITHOUT ROWID;
