-- A file put on its observed episode's own number because its source has no
-- episode mapping yet (`same_number`) follows the mapping decided later
-- (`trss-jobs`'s `place::relocate`; docs/specs/library.md, 자막의 회차 대응;
-- docs/specs/subtitles.md, 파일의 회차). Before, it was `explicit`, as a
-- person's choice is, and no mapping moved it.
--
-- Run with foreign keys off (a remake, see `db.rs`): `subtitle_stored`,
-- `subtitle_job_plan` and `subtitle_replacements` are made anew for their
-- CHECKs. A same-number link, like a mapped one, has the observed episode it
-- was taken from (`basis`) and a stored one its source.

CREATE TABLE subtitle_stored_next (
    id                  TEXT    PRIMARY KEY CHECK (id <> ''),
    work_id             TEXT    NOT NULL,
    season              INTEGER NOT NULL CHECK (season >= 0),
    package_id          TEXT    NOT NULL REFERENCES subtitle_packages (id),
    subtitle_asset_id   TEXT    NOT NULL REFERENCES subtitle_assets (id),
    source_id           TEXT    REFERENCES subtitle_sources (id),
    anissia_episode     TEXT,
    attachment_episode  TEXT,
    assignment          TEXT    CHECK (assignment IN ('mapped', 'same_number', 'explicit')),
    basis               TEXT    CHECK (basis IN ('anissia', 'attachment')),
    episode             INTEGER CHECK (episode >= 1),
    format              TEXT    NOT NULL CHECK (format IN ('ass', 'srt', 'smi', 'other')),
    encoding            TEXT,
    creator             TEXT,
    language            TEXT,
    purpose             TEXT,
    release             TEXT,
    revision_label      TEXT,
    links_known         INTEGER NOT NULL DEFAULT 0 CHECK (links_known IN (0, 1)),
    anissia_observation TEXT    CHECK (anissia_observation IS NULL
                                       OR json_valid(anissia_observation)),
    job_id              TEXT    REFERENCES subtitle_jobs (id) ON DELETE SET NULL,
    stored_at           INTEGER NOT NULL,
    cleaned_at          INTEGER,
    CHECK ((assignment IS NULL) = (episode IS NULL)),
    CHECK (assignment IS NULL OR assignment = 'explicit'
           OR (basis IS NOT NULL AND source_id IS NOT NULL))
) WITHOUT ROWID;
INSERT INTO subtitle_stored_next
    (id, work_id, season, package_id, subtitle_asset_id, source_id, anissia_episode,
     attachment_episode, assignment, basis, episode, format, encoding, creator, language,
     purpose, release, revision_label, links_known, anissia_observation, job_id, stored_at,
     cleaned_at)
    SELECT id, work_id, season, package_id, subtitle_asset_id, source_id, anissia_episode,
           attachment_episode, assignment, basis, episode, format, encoding, creator, language,
           purpose, release, revision_label, links_known, anissia_observation, job_id,
           stored_at, cleaned_at
      FROM subtitle_stored;
DROP TABLE subtitle_stored;
ALTER TABLE subtitle_stored_next RENAME TO subtitle_stored;
CREATE INDEX subtitle_stored_by_episode ON subtitle_stored (work_id, season, episode);
CREATE INDEX subtitle_stored_by_asset ON subtitle_stored (subtitle_asset_id);

CREATE TABLE subtitle_job_plan_next (
    job_id             TEXT    NOT NULL REFERENCES subtitle_jobs (id) ON DELETE CASCADE,
    position           INTEGER NOT NULL,
    file_id            TEXT    NOT NULL REFERENCES subtitle_job_files (id),
    member             TEXT,
    name               TEXT    NOT NULL CHECK (name <> ''),
    kind               TEXT    NOT NULL CHECK (kind IN
                           ('subtitle', 'font', 'attachment', 'companion', 'other')),
    format             TEXT    CHECK (format IN ('ass', 'srt', 'smi', 'other')),
    size               INTEGER NOT NULL CHECK (size >= 0),
    sha256             TEXT    NOT NULL CHECK (length(sha256) = 64),
    item_id            INTEGER REFERENCES subtitle_job_items (id) ON DELETE CASCADE,
    anissia_episode    TEXT,
    attachment_episode TEXT,
    assignment         TEXT    CHECK (assignment IN ('mapped', 'same_number', 'explicit')),
    basis              TEXT    CHECK (basis IN ('anissia', 'attachment')),
    episode            INTEGER CHECK (episode >= 1),
    action             TEXT    NOT NULL CHECK (action IN ('apply', 'store', 'drop')),
    question           TEXT    CHECK (question IS NULL OR question <> ''),
    stored_id          TEXT    REFERENCES subtitle_stored (id),
    outcome            TEXT    CHECK (outcome IN
                           ('applied', 'stored', 'existing', 'no_video', 'held', 'failed',
                            'dropped')),
    note               TEXT,
    applied_id         TEXT    REFERENCES subtitle_applied (id),
    updated_at         INTEGER NOT NULL,
    asset_id           TEXT    REFERENCES subtitle_assets (id),
    chosen             TEXT    CHECK (chosen IN ('apply', 'add')),
    PRIMARY KEY (job_id, position),
    CHECK ((assignment IS NULL) = (episode IS NULL))
) WITHOUT ROWID;
INSERT INTO subtitle_job_plan_next
    (job_id, position, file_id, member, name, kind, format, size, sha256, item_id,
     anissia_episode, attachment_episode, assignment, basis, episode, action, question,
     stored_id, outcome, note, applied_id, updated_at, asset_id, chosen)
    SELECT job_id, position, file_id, member, name, kind, format, size, sha256, item_id,
           anissia_episode, attachment_episode, assignment, basis, episode, action, question,
           stored_id, outcome, note, applied_id, updated_at, asset_id, chosen
      FROM subtitle_job_plan;
DROP TABLE subtitle_job_plan;
ALTER TABLE subtitle_job_plan_next RENAME TO subtitle_job_plan;
CREATE INDEX subtitle_job_plan_by_file ON subtitle_job_plan (file_id);

-- Earlier builds wrote a candidate's file put on the candidate's own number
-- for want of a mapping as `explicit`. Such a row is told from a person's
-- choice only where no person had a say: a candidate's job (a picked
-- candidate or a subscribed creator's) with a source, which no person
-- confirmed (it has no `placement` step) and whose source has no decided
-- mapping now. One whose mapping was decided since stays `explicit`.
UPDATE subtitle_job_plan
   SET assignment = 'same_number', basis = 'anissia'
 WHERE assignment = 'explicit' AND basis IS NULL
   AND trim(anissia_episode) <> '' AND trim(anissia_episode) NOT GLOB '*[^0-9]*'
   AND CAST(trim(anissia_episode) AS INTEGER) = episode
   AND EXISTS (
       SELECT 1 FROM subtitle_jobs j
        WHERE j.id = subtitle_job_plan.job_id AND j.origin IN ('pick', 'auto')
          AND j.source_id IS NOT NULL
          AND NOT EXISTS (SELECT 1 FROM subtitle_job_steps t
                           WHERE t.job_id = j.id AND t.step = 'placement')
          AND NOT EXISTS (SELECT 1 FROM subtitle_episode_mappings m
                           WHERE m.work_id = j.work_id AND m.season = j.season
                             AND m.source_id = j.source_id AND m.kind <> 'undecided'));

-- A stored subtitle follows when every row that names it does.
UPDATE subtitle_stored
   SET assignment = 'same_number', basis = 'anissia'
 WHERE assignment = 'explicit' AND basis IS NULL AND source_id IS NOT NULL
   AND trim(anissia_episode) <> '' AND trim(anissia_episode) NOT GLOB '*[^0-9]*'
   AND CAST(trim(anissia_episode) AS INTEGER) = episode
   AND EXISTS (SELECT 1 FROM subtitle_job_plan p
                WHERE p.stored_id = subtitle_stored.id AND p.assignment = 'same_number')
   AND NOT EXISTS (SELECT 1 FROM subtitle_job_plan p
                    WHERE p.stored_id = subtitle_stored.id
                      AND p.assignment IS NOT 'same_number')
   AND NOT EXISTS (SELECT 1 FROM subtitle_episode_mappings m
                    WHERE m.work_id = subtitle_stored.work_id
                      AND m.season = subtitle_stored.season
                      AND m.source_id = subtitle_stored.source_id AND m.kind <> 'undecided');

-- A replacement plan binds what put its row there: one made for a row the
-- update above marked binds the same.
CREATE TABLE subtitle_replacements_next (
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
    assignment   TEXT    NOT NULL CHECK (assignment IN ('mapped', 'same_number', 'explicit')),
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
INSERT INTO subtitle_replacements_next
    (id, job_id, position, version, state, reason, work_id, season, episode, assignment, basis,
     folder, video_path, video_object, video_size, video_mtime, stored_id, asset_id,
     asset_path, asset_size, asset_sha256, asset_lines, target, created_at, decided_at,
     updated_at)
    SELECT r.id, r.job_id, r.position, r.version, r.state, r.reason, r.work_id, r.season,
           r.episode,
           CASE WHEN r.assignment = 'explicit' AND r.basis IS NULL
                     AND p.assignment = 'same_number' AND p.episode = r.episode
                THEN 'same_number' ELSE r.assignment END,
           CASE WHEN r.assignment = 'explicit' AND r.basis IS NULL
                     AND p.assignment = 'same_number' AND p.episode = r.episode
                THEN p.basis ELSE r.basis END,
           r.folder, r.video_path, r.video_object, r.video_size, r.video_mtime, r.stored_id,
           r.asset_id, r.asset_path, r.asset_size, r.asset_sha256, r.asset_lines, r.target,
           r.created_at, r.decided_at, r.updated_at
      FROM subtitle_replacements r
      LEFT JOIN subtitle_job_plan p ON p.job_id = r.job_id AND p.position = r.position;
DROP TABLE subtitle_replacements;
ALTER TABLE subtitle_replacements_next RENAME TO subtitle_replacements;

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
