-- The creator of a subtitle file the library found (`docs/specs/library.md`,
-- 머리와 시즌의 `제작자 지정`; `store::library::creators` in `trss-library`).
--
-- A subtitle file the scan found in a watch folder, or the user put there,
-- has no creator the app knows (`제작자 알 수 없음`). The user can name the
-- creator of one, or of all of a season's at once, from the creators of the
-- Anissia anime the season is linked to.
--
-- `creator_source_id` is the subtitle source (`subtitle_sources`: one creator's
-- lines of one anime) the user named; NULL is `제작자 알 수 없음`, and a video
-- never has one. `creator_version` goes up with every change of it, so a
-- change made from an older version changes nothing (two screens changing one
-- file's creator: the later save is refused). A scan that clears the creator
-- (the file is read as a video) raises it too, and a file the scan records as
-- new starts at the scan's time in milliseconds, so it is past every version
-- an earlier file of that path had (rows that exist before this migration are
-- at 0).
--
-- `creator_set_at` is when the creator was named (NULL while there is none,
-- and for a file whose creator is unknown): the subscribed creator's automatic
-- receipt of a revision for a file with that creator takes only the lines
-- the app first observed after it (`trss_jobs::follow`).
--
-- The attribution is kept with the file's row, so it is the file's identity
-- that decides what keeps it: the path below the work folder, which an archive
-- move of the work folder and a rescan that finds the file as before leave
-- alone, and which a rescan that reads the file's episode differently carries
-- to the row it records again. A file that is renamed or leaves the folder is
-- another file, or none, and loses it. Nothing here moves, renames or receives
-- a file, and a source is never removed, so the reference always holds.

ALTER TABLE media_files ADD COLUMN creator_source_id TEXT REFERENCES subtitle_sources (id);
ALTER TABLE media_files ADD COLUMN creator_version INTEGER NOT NULL DEFAULT 0
    CHECK (creator_version >= 0);
ALTER TABLE media_files ADD COLUMN creator_set_at INTEGER;

-- `subtitle_jobs.revises_attributed` is 1 on a job the app made on its own for
-- a line of the subscribed creator that revises a subtitle file whose creator
-- the user named (`revision_of` stays NULL: no earlier receipt of it exists).
-- The job only receives, like any other revision job.
ALTER TABLE subtitle_jobs ADD COLUMN revises_attributed INTEGER NOT NULL DEFAULT 0
    CHECK (revises_attributed IN (0, 1));
