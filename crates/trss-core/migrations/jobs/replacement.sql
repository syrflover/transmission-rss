-- Replacing the subtitle an episode has, once a person approves it
-- (`trss-jobs`'s `place::replace`; docs/specs/subtitles.md, 교체 비교와 승인
-- and 승인 증거와 반영 직전 검사; docs/specs/jobs.md, 체크포인트와 중단 복구).
--
-- Run with foreign keys off (a remake, see `db.rs`): `subtitle_packages` and
-- `subtitle_file_effects` are made anew for their CHECKs.

-- A package may be a subtitle the app found beside a video and did not manage
-- (`existing`): its bytes are imported, as the creator nobody named's stored
-- subtitle, before a replacement overwrites or removes it. Such a package has
-- no source page and no receipt time.
CREATE TABLE subtitle_packages_next (
    id          TEXT    PRIMARY KEY CHECK (id <> ''),
    work_id     TEXT    NOT NULL,
    job_id      TEXT    REFERENCES subtitle_jobs (id) ON DELETE SET NULL,
    source_kind TEXT    NOT NULL CHECK (source_kind IN ('post', 'upload', 'find', 'existing')),
    source_page TEXT,
    received_at INTEGER,
    created_at  INTEGER NOT NULL
) WITHOUT ROWID;
INSERT INTO subtitle_packages_next
    (id, work_id, job_id, source_kind, source_page, received_at, created_at)
    SELECT id, work_id, job_id, source_kind, source_page, received_at, created_at
      FROM subtitle_packages;
DROP TABLE subtitle_packages;
ALTER TABLE subtitle_packages_next RENAME TO subtitle_packages;
CREATE INDEX subtitle_packages_by_job ON subtitle_packages (job_id) WHERE job_id IS NOT NULL;

-- One plan to replace what an episode has beside its video with a stored
-- subtitle (교체 계획), made for a row of a job's plan (`job_id`,
-- `position`) when its episode has a subtitle. Its evidence never changes
-- (a trigger refuses it): a change found on disk or in the records makes the
-- next `version` of the row's plan instead. It binds:
--
-- - the target and what put the row there (`work_id`, `season`, `episode`,
--   `assignment`, `basis`, as in `subtitle_job_plan`);
-- - the video (`video_path`, relative to `folder`, the work folder as it
--   was): a regular file, its object, length and change time
--   (`video_mtime`, nanoseconds), not a hash of its bytes;
-- - the new subtitle: the stored subtitle `stored_id`, its asset and the
--   asset's path, length and SHA-256 as recorded, and its dialogue line count
--   (`asset_lines`, `NULL` when the app cannot count them);
-- - `target`, where the new copy goes beside the video;
-- - what happens to each path (`subtitle_replacement_paths`).
--
-- `state` is `open` (waiting for the person), `kept` (현재 유지: nothing
-- changes), `approved` (새 자막으로 교체: to be carried out), `done`,
-- `stale` (a check found the evidence changed: `reason` says how, and the
-- row's next version is the one to decide), `held` (an effect whose result
-- the app could not confirm; its protective copies stay) or `failed` (it
-- stopped before anything beside the video changed). It only goes forward
-- (a trigger refuses the rest), so an approval is never used twice.
CREATE TABLE subtitle_replacements (
    id           TEXT    PRIMARY KEY CHECK (id <> ''),
    job_id       TEXT    NOT NULL,
    position     INTEGER NOT NULL,
    version      INTEGER NOT NULL CHECK (version >= 1),
    state        TEXT    NOT NULL CHECK (state IN
                     ('open', 'kept', 'approved', 'done', 'stale', 'held', 'failed')),
    reason       TEXT,
    work_id      TEXT    NOT NULL,
    season       INTEGER NOT NULL CHECK (season >= 0),
    episode      INTEGER NOT NULL CHECK (episode >= 1),
    assignment   TEXT    NOT NULL CHECK (assignment IN ('mapped', 'explicit')),
    basis        TEXT    CHECK (basis IN ('anissia', 'attachment')),
    folder       TEXT    NOT NULL CHECK (folder <> ''),
    video_path   TEXT    NOT NULL CHECK (video_path <> ''),
    video_object TEXT    NOT NULL CHECK (video_object <> ''),
    video_size   INTEGER NOT NULL CHECK (video_size >= 0),
    video_mtime  INTEGER NOT NULL,
    stored_id    TEXT    NOT NULL REFERENCES subtitle_stored (id),
    asset_id     TEXT    NOT NULL REFERENCES subtitle_assets (id),
    asset_path   TEXT    NOT NULL CHECK (asset_path <> ''),
    asset_size   INTEGER NOT NULL CHECK (asset_size >= 0),
    asset_sha256 TEXT    NOT NULL CHECK (length(asset_sha256) = 64),
    asset_lines  INTEGER CHECK (asset_lines >= 0),
    target       TEXT    NOT NULL CHECK (target <> ''),
    created_at   INTEGER NOT NULL,
    decided_at   INTEGER,
    updated_at   INTEGER NOT NULL,
    UNIQUE (job_id, position, version),
    FOREIGN KEY (job_id, position) REFERENCES subtitle_job_plan (job_id, position)
        ON DELETE CASCADE
);

-- A row has one plan to decide or carry out at a time.
CREATE UNIQUE INDEX subtitle_replacements_one_live ON subtitle_replacements (job_id, position)
    WHERE state IN ('open', 'approved');
CREATE INDEX subtitle_replacements_by_episode
    ON subtitle_replacements (work_id, season, episode);

CREATE TRIGGER subtitle_replacements_evidence_fixed
    BEFORE UPDATE OF id, job_id, position, version, work_id, season, episode, assignment,
                     basis, folder, video_path, video_object, video_size, video_mtime,
                     stored_id, asset_id, asset_path, asset_size, asset_sha256, asset_lines,
                     target, created_at
    ON subtitle_replacements
BEGIN
    SELECT RAISE(ABORT, 'a replacement plan''s evidence does not change');
END;

CREATE TRIGGER subtitle_replacements_go_forward
    BEFORE UPDATE OF state ON subtitle_replacements
    WHEN NOT (OLD.state = NEW.state
              OR (OLD.state = 'open' AND NEW.state IN ('kept', 'approved', 'stale'))
              OR (OLD.state = 'approved' AND NEW.state IN ('done', 'stale', 'held', 'failed')))
BEGIN
    SELECT RAISE(ABORT, 'a replacement plan only goes forward');
END;

-- What a plan does to each path beside the video it looked at (relative to
-- the plan's folder): `add` the new copy where nothing is, `replace` the file
-- at the new copy's path, `remove` an earlier copy the app applied at
-- another path (its record `applied_id`), `keep` a subtitle the app does not
-- manage at another path. A file there has its length, SHA-256, object,
-- change time (nanoseconds) and dialogue line count as the plan saw them;
-- `applied_id` is the app's applied copy it is, while its bytes are the ones
-- applied (a copy a person changed since is not managed any more).
CREATE TABLE subtitle_replacement_paths (
    plan_id    TEXT    NOT NULL REFERENCES subtitle_replacements (id) ON DELETE CASCADE,
    path       TEXT    NOT NULL CHECK (path <> ''),
    action     TEXT    NOT NULL CHECK (action IN ('add', 'replace', 'remove', 'keep')),
    byte_size  INTEGER CHECK (byte_size >= 0),
    sha256     TEXT    CHECK (sha256 IS NULL OR length(sha256) = 64),
    object     TEXT,
    mtime      INTEGER,
    lines      INTEGER CHECK (lines >= 0),
    applied_id TEXT    REFERENCES subtitle_applied (id),
    PRIMARY KEY (plan_id, path),
    CHECK ((action = 'add') = (sha256 IS NULL)),
    CHECK ((sha256 IS NULL) = (object IS NULL)),
    CHECK (action <> 'remove' OR applied_id IS NOT NULL)
) WITHOUT ROWID;

CREATE TRIGGER subtitle_replacement_paths_fixed
    BEFORE UPDATE ON subtitle_replacement_paths
BEGIN
    SELECT RAISE(ABORT, 'a replacement plan''s paths do not change');
END;

-- The effects gain two kinds for a replacement (`plan_id`):
--
-- - `remove` takes the file at `source` off its path: `intended`, then its
--   protective copy `temp` in `.trss/tmp/` is written, synced and read back
--   as the plan saw it (`prepared`, with the copy's object), then the file is
--   renamed, replacing nothing, to `target` (`.trss/tmp/<ID>.aside`) and found
--   to be the plan's object and bytes there (`set_aside`), and once the
--   replacement is recorded `done`, its two files removed;
-- - `import` keeps the bytes of a subtitle the app did not manage, from the
--   protective copy `source`, as a stored subtitle at `target` in
--   `.trss/subtitles/`, as `store` does.
--
-- Two effects under way never aim at one path, and two removals never take
-- one off, whatever its letters' case.
CREATE TABLE subtitle_file_effects_next (
    id         TEXT    PRIMARY KEY CHECK (id <> ''),
    job_id     TEXT    NOT NULL,
    position   INTEGER NOT NULL,
    kind       TEXT    NOT NULL CHECK (kind IN ('store', 'apply', 'remove', 'import')),
    state      TEXT    NOT NULL CHECK (state IN
                   ('intended', 'prepared', 'set_aside', 'done', 'held', 'failed', 'abandoned')),
    folder     TEXT    NOT NULL CHECK (folder <> ''),
    temp       TEXT    NOT NULL CHECK (temp <> ''),
    target     TEXT    NOT NULL CHECK (target <> ''),
    video      TEXT    CHECK ((kind = 'apply') = (video IS NOT NULL)),
    source     TEXT    CHECK ((kind IN ('remove', 'import')) = (source IS NOT NULL)),
    plan_id    TEXT    REFERENCES subtitle_replacements (id) ON DELETE CASCADE,
    size       INTEGER NOT NULL CHECK (size >= 0),
    sha256     TEXT    NOT NULL CHECK (length(sha256) = 64),
    object     TEXT,
    reason     TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (job_id, position) REFERENCES subtitle_job_plan (job_id, position)
        ON DELETE CASCADE,
    CHECK ((state IN ('prepared', 'set_aside')) <= (object IS NOT NULL)),
    CHECK (state <> 'set_aside' OR kind = 'remove'),
    CHECK (kind NOT IN ('remove', 'import') OR plan_id IS NOT NULL)
) WITHOUT ROWID;
INSERT INTO subtitle_file_effects_next
    (id, job_id, position, kind, state, folder, temp, target, video, size, sha256, object,
     reason, created_at, updated_at)
    SELECT id, job_id, position, kind, state, folder, temp, target, video, size, sha256, object,
           reason, created_at, updated_at
      FROM subtitle_file_effects;
DROP TABLE subtitle_file_effects;
ALTER TABLE subtitle_file_effects_next RENAME TO subtitle_file_effects;

CREATE INDEX subtitle_file_effects_by_row ON subtitle_file_effects (job_id, position);
CREATE INDEX subtitle_file_effects_by_plan ON subtitle_file_effects (plan_id)
    WHERE plan_id IS NOT NULL;
CREATE UNIQUE INDEX subtitle_file_effects_one_target
    ON subtitle_file_effects (folder, lower(target))
    WHERE state IN ('intended', 'prepared', 'set_aside');
CREATE UNIQUE INDEX subtitle_file_effects_one_removal
    ON subtitle_file_effects (folder, lower(source))
    WHERE kind = 'remove' AND state IN ('intended', 'prepared', 'set_aside');
