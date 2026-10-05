-- What differs between the current subtitle of a replacement plan and the new
-- one (`trss_jobs::place::replace`; docs/specs/subtitles.md, 교체 비교와
-- 승인; ticket 0069): one row for each plan the app made after it compared
-- contents, written with the plan in one transaction. A plan made before has
-- no row, and nothing makes one for it: its screens say it was not compared.
--
-- `path` is the current file compared, as in the plan's paths
-- (`subtitle_replacement_paths`). A comparison is one of two:
--
-- - made: `diff` is the engine's summary as JSON (`trss_subtitles::compare::Diff`
--   with no dialogue or timing lines: the counts, both sides' format,
--   encoding and cue count, styles, fonts, and what was not compared), and
--   `lines` is `{"dialogue": [...], "timing": [...]}`, the lines the screen
--   opens on demand. They are apart so that reading a job's detail, which
--   is polled, never reads a file's whole change (hundreds of KB for a
--   rewritten file);
-- - not made: `unreadable` is why, in Korean for the screen (a file whose
--   content cannot be read, one too large, or a current file that changed
--   after the plan saw it). It is never shown as no difference.
--
-- A plan's evidence never changes, and neither does what it was compared to:
-- a trigger refuses any UPDATE.
CREATE TABLE subtitle_replacement_diffs (
    plan_id    TEXT PRIMARY KEY REFERENCES subtitle_replacements (id) ON DELETE CASCADE,
    path       TEXT NOT NULL CHECK (path <> ''),
    diff       TEXT CHECK (diff IS NULL OR json_valid(diff)),
    lines      TEXT CHECK (lines IS NULL OR json_valid(lines)),
    unreadable TEXT CHECK (unreadable IS NULL OR unreadable <> ''),
    CHECK ((diff IS NULL) <> (unreadable IS NULL)),
    CHECK ((diff IS NULL) = (lines IS NULL))
) WITHOUT ROWID;

CREATE TRIGGER subtitle_replacement_diffs_fixed
    BEFORE UPDATE ON subtitle_replacement_diffs
BEGIN
    SELECT RAISE(ABORT, 'a replacement plan''s comparison does not change');
END;
