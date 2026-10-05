-- Unpacking a received archive in a child process with limits
-- (`docs/specs/subtitles.md`, 압축 해제의 격리와 한도; `trss_jobs::place::unpack`).
--
-- A received archive (the first volume of a split one) is unpacked into the
-- receive area's `<job id>/.unpack/<file id>/`. `unpacked_at` is when its
-- members were recorded, `unpack_error` why it could not be unpacked
-- (풀지 못함: it stays in the receive area). Both are `NULL` until it was
-- tried. `volume_of` is the first volume of the split archive a later volume
-- belongs to: it is unpacked, kept and cleared with that one.

ALTER TABLE subtitle_job_files ADD COLUMN volume_of TEXT REFERENCES subtitle_job_files (id);
ALTER TABLE subtitle_job_files ADD COLUMN unpacked_at INTEGER;
ALTER TABLE subtitle_job_files ADD COLUMN unpack_error TEXT
    CHECK (unpack_error IS NULL OR unpack_error <> '');

-- The members an archive was unpacked to: its `path` inside the archive (a
-- member of an archive inside it under that archive's path), the file
-- `position` names in the unpack folder, the bytes' length and SHA-256, and
-- what their check found them to be (`format`, as `subtitle_job_files` has
-- it), or why they are not a file (`reason`: a web page, nothing).
CREATE TABLE subtitle_job_members (
    file_id  TEXT    NOT NULL REFERENCES subtitle_job_files (id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position >= 0),
    path     TEXT    NOT NULL CHECK (path <> ''),
    size     INTEGER NOT NULL CHECK (size >= 0),
    sha256   TEXT    NOT NULL CHECK (length(sha256) = 64),
    format   TEXT    CHECK (format IN ('zip', 'ass', 'srt', 'smi', 'other')),
    reason   TEXT,
    PRIMARY KEY (file_id, position),
    CHECK ((format IS NULL) <> (reason IS NULL))
) WITHOUT ROWID;

CREATE UNIQUE INDEX subtitle_job_members_by_path ON subtitle_job_members (file_id, path);
