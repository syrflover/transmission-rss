-- The common policy (`docs/specs/settings.md`, 공통 정책): the order of the
-- subtitle formats, and the server browser's idle time and how many jobs may
-- use it at once. The three share one version, as they share one save.
--
-- `policy_settings` holds at most one row. No row means the defaults
-- (`ass,srt,smi`, 300 seconds, 1 job) at version 0. `format_order` names the
-- three formats, each once, in order, separated by commas. The supported
-- ranges of the two numbers are the app's (`trss_core::settings::policy`), so
-- the table only keeps them positive. `saved_at` is when the row was last
-- written (Unix ms).
--
-- `work_subtitle_policy` is a work's own format order, which takes the place
-- of the global one for that work. The work's subtitles write it; the settings
-- list the works that have one.

CREATE TABLE policy_settings (
    id                   INTEGER PRIMARY KEY CHECK (id = 1),
    format_order         TEXT    NOT NULL CHECK (format_order IN (
                             'ass,srt,smi', 'ass,smi,srt', 'srt,ass,smi',
                             'srt,smi,ass', 'smi,ass,srt', 'smi,srt,ass')),
    idle_timeout_seconds INTEGER NOT NULL CHECK (idle_timeout_seconds > 0),
    max_concurrent_jobs  INTEGER NOT NULL CHECK (max_concurrent_jobs > 0),
    version              INTEGER NOT NULL CHECK (version >= 1),
    saved_at             INTEGER NOT NULL
);

CREATE TABLE work_subtitle_policy (
    work_id      TEXT    PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    format_order TEXT    NOT NULL CHECK (format_order IN (
                     'ass,srt,smi', 'ass,smi,srt', 'srt,ass,smi',
                     'srt,smi,ass', 'smi,ass,srt', 'smi,srt,ass')),
    updated_at   INTEGER NOT NULL
) WITHOUT ROWID;
