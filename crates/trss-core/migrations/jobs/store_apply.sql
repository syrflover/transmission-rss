-- Storing and applying what a job received (`trss-jobs`'s `place`, and
-- docs/specs/subtitles.md, 보관본과 적용본; docs/specs/jobs.md, 체크포인트와
-- 중단 복구).
--
-- A job goes on after receiving: it waits for a person to say which episode
-- a file is (`placement`, 배치 확인, also asked for a file whose episode the
-- analysis could not settle), to approve a replacement (`approval`, 교체
-- 승인) or for a video (`video`, 영상 대기), and its steps go on with
-- `placement`, `store` (보관), `apply` (적용) and `approval`. SQLite cannot
-- change a CHECK, so each such column is made anew (`steps.step`, part of its
-- key, with its table).

ALTER TABLE subtitle_jobs ADD COLUMN wait_next TEXT
    CHECK (wait_next IN ('auth', 'subtitle', 'placement', 'approval', 'video'));
UPDATE subtitle_jobs SET wait_next = wait;
ALTER TABLE subtitle_jobs DROP COLUMN wait;
ALTER TABLE subtitle_jobs RENAME COLUMN wait_next TO wait;

ALTER TABLE subtitle_jobs ADD COLUMN stage_next TEXT
    CHECK (stage_next IN ('found', 'open', 'auth', 'receive', 'placement', 'store', 'apply',
                          'approval'));
UPDATE subtitle_jobs SET stage_next = stage;
ALTER TABLE subtitle_jobs DROP COLUMN stage;
ALTER TABLE subtitle_jobs RENAME COLUMN stage_next TO stage;

ALTER TABLE subtitle_job_items ADD COLUMN wait_next TEXT
    CHECK (wait_next IN ('auth', 'subtitle', 'placement', 'approval', 'video'));
UPDATE subtitle_job_items SET wait_next = wait;
ALTER TABLE subtitle_job_items DROP COLUMN wait;
ALTER TABLE subtitle_job_items RENAME COLUMN wait_next TO wait;

CREATE TABLE subtitle_job_steps_next (
    job_id TEXT    NOT NULL REFERENCES subtitle_jobs (id) ON DELETE CASCADE,
    step   TEXT    NOT NULL CHECK (step IN
               ('found', 'open', 'auth', 'receive', 'placement', 'store', 'apply', 'approval')),
    state  TEXT    NOT NULL CHECK (state IN ('current', 'waiting', 'done', 'failed', 'partial')),
    at     INTEGER NOT NULL,
    note   TEXT,
    PRIMARY KEY (job_id, step)
) WITHOUT ROWID;
INSERT INTO subtitle_job_steps_next (job_id, step, state, at, note)
    SELECT job_id, step, state, at, note FROM subtitle_job_steps;
DROP TABLE subtitle_job_steps;
ALTER TABLE subtitle_job_steps_next RENAME TO subtitle_job_steps;

-- A receipt whose bytes were removed from the receive area once what it held
-- was stored: set before the removal, so a removal cut short is done again.
ALTER TABLE subtitle_job_files ADD COLUMN cleared_at INTEGER;

-- What the library keeps (docs/specs/settings.md, 보관 관계 필드). None of it
-- names its work by reference: the records outlive a work's folder, as jobs
-- do, and the files in `.trss/` stay with the folder.
--
-- `subtitle_packages` is what one post (`post`), one upload (`upload`) or one
-- find job (`find`) brought, as it was received (`received_at`: when the
-- job's receipt of it was done). `subtitle_package_entries` are its members
-- as their source named them (the folders inside an archive included), each
-- with the asset that keeps its bytes.
CREATE TABLE subtitle_packages (
    id          TEXT    PRIMARY KEY CHECK (id <> ''),
    work_id     TEXT    NOT NULL,
    job_id      TEXT    REFERENCES subtitle_jobs (id) ON DELETE SET NULL,
    source_kind TEXT    NOT NULL CHECK (source_kind IN ('post', 'upload', 'find')),
    source_page TEXT,
    received_at INTEGER,
    created_at  INTEGER NOT NULL
) WITHOUT ROWID;

CREATE INDEX subtitle_packages_by_job ON subtitle_packages (job_id) WHERE job_id IS NOT NULL;

-- One kept file: a subtitle or a font in the work folder's
-- `.trss/subtitles/` (`base = 'work'`, its path relative to the work
-- folder), anything else in the app data folder (`base = 'app_data'`). One
-- path holds one asset, whatever the case of its letters.
CREATE TABLE subtitle_assets (
    id            TEXT    PRIMARY KEY CHECK (id <> ''),
    work_id       TEXT    NOT NULL,
    kind          TEXT    NOT NULL CHECK (kind IN
                      ('subtitle', 'font', 'attachment', 'companion', 'other')),
    base          TEXT    NOT NULL CHECK (base IN ('work', 'app_data')),
    relative_path TEXT    NOT NULL CHECK (relative_path <> ''),
    byte_size     INTEGER NOT NULL CHECK (byte_size >= 0),
    sha256        TEXT    NOT NULL CHECK (length(sha256) = 64),
    created_at    INTEGER NOT NULL,
    CHECK ((base = 'work') = (kind IN ('subtitle', 'font')))
) WITHOUT ROWID;

CREATE UNIQUE INDEX subtitle_assets_one_path
    ON subtitle_assets (work_id, base, lower(relative_path));

CREATE TABLE subtitle_package_entries (
    package_id       TEXT    NOT NULL REFERENCES subtitle_packages (id) ON DELETE CASCADE,
    position         INTEGER NOT NULL,
    asset_id         TEXT    NOT NULL REFERENCES subtitle_assets (id),
    original_name    TEXT    NOT NULL CHECK (original_name <> ''),
    source_file_date TEXT    CHECK (source_file_date IS NULL OR json_valid(source_file_date)),
    PRIMARY KEY (package_id, position)
) WITHOUT ROWID;

CREATE INDEX subtitle_package_entries_by_asset ON subtitle_package_entries (asset_id);

-- One stored revision of a subtitle (보관본) of a season of the work. The
-- episodes it was observed as stay as they were written: Anissia's
-- candidate's (`anissia_episode`) and the one the file's name says
-- (`attachment_episode`). `assignment` says what puts it on an episode:
-- `mapped`, the source's mapping from the observed episode `basis` (the
-- episode is recomputed when the mapping changes); `explicit`, a person's
-- choice or the same number where no mapping was; `NULL`, no episode.
-- `episode` is the episode it is on now. `links_known` says whether the
-- analysis looked for its fonts, attachments and companions
-- (`subtitle_stored_assets`), so none linked is no font rather than unknown.
-- `anissia_observation` is the candidate's line as observed (`anime_no`,
-- `episode`, `creator`, `website`, `updDt_raw`).
CREATE TABLE subtitle_stored (
    id                  TEXT    PRIMARY KEY CHECK (id <> ''),
    work_id             TEXT    NOT NULL,
    season              INTEGER NOT NULL CHECK (season >= 0),
    package_id          TEXT    NOT NULL REFERENCES subtitle_packages (id),
    subtitle_asset_id   TEXT    NOT NULL REFERENCES subtitle_assets (id),
    source_id           TEXT    REFERENCES subtitle_sources (id),
    anissia_episode     TEXT,
    attachment_episode  TEXT,
    assignment          TEXT    CHECK (assignment IN ('mapped', 'explicit')),
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
    CHECK ((assignment IS NULL) = (episode IS NULL)),
    CHECK (assignment IS NOT 'mapped' OR (basis IS NOT NULL AND source_id IS NOT NULL))
) WITHOUT ROWID;

CREATE INDEX subtitle_stored_by_episode ON subtitle_stored (work_id, season, episode);
CREATE INDEX subtitle_stored_by_asset ON subtitle_stored (subtitle_asset_id);

CREATE TABLE subtitle_stored_assets (
    stored_id TEXT NOT NULL REFERENCES subtitle_stored (id) ON DELETE CASCADE,
    asset_id  TEXT NOT NULL REFERENCES subtitle_assets (id),
    role      TEXT NOT NULL CHECK (role IN ('font', 'attachment', 'companion')),
    PRIMARY KEY (stored_id, asset_id)
) WITHOUT ROWID;

CREATE INDEX subtitle_stored_assets_by_asset ON subtitle_stored_assets (asset_id);

-- A copy of a stored subtitle the app put beside a video (적용본), with the
-- facts of the file it published: paths relative to the work folder, the
-- bytes' length and SHA-256 and the file's object. `removed_at` is set when
-- the app took it away again.
CREATE TABLE subtitle_applied (
    id         TEXT    PRIMARY KEY CHECK (id <> ''),
    work_id    TEXT    NOT NULL,
    stored_id  TEXT    NOT NULL REFERENCES subtitle_stored (id),
    season     INTEGER NOT NULL CHECK (season >= 0),
    episode    INTEGER NOT NULL CHECK (episode >= 1),
    video_path TEXT    NOT NULL CHECK (video_path <> ''),
    path       TEXT    NOT NULL CHECK (path <> ''),
    byte_size  INTEGER NOT NULL CHECK (byte_size >= 0),
    sha256     TEXT    NOT NULL CHECK (length(sha256) = 64),
    object     TEXT    NOT NULL CHECK (object <> ''),
    job_id     TEXT    REFERENCES subtitle_jobs (id) ON DELETE SET NULL,
    applied_at INTEGER NOT NULL,
    removed_at INTEGER
) WITHOUT ROWID;

CREATE UNIQUE INDEX subtitle_applied_one_path ON subtitle_applied (work_id, path)
    WHERE removed_at IS NULL;
CREATE INDEX subtitle_applied_by_episode ON subtitle_applied (work_id, season, episode);

-- A job's placement plan (배치 계획): one row for each file it received (and,
-- once archives are opened, each member), what the analysis found it to be
-- and where it goes. `episode` is the season's episode it is put on, with
-- what puts it there (`assignment`, `basis`, as in `subtitle_stored`), and
-- `item_id` the candidate it was picked for. `action` is `apply` (store,
-- then put beside the video), `store` (store only) or `drop` (not kept).
-- `question` says why a person has to say which episode it is (회차 확인
-- 필요); `NULL` once settled. `outcome` is what came of it (`NULL`: not yet):
-- `applied`, `stored` (stored only), `existing` (the episode has a subtitle
-- already, which stays), `no_video`, `held` (a file effect the app could not
-- confirm, or a name another file took first), `failed` or `dropped`.
CREATE TABLE subtitle_job_plan (
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
    assignment         TEXT    CHECK (assignment IN ('mapped', 'explicit')),
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
    PRIMARY KEY (job_id, position),
    CHECK ((assignment IS NULL) = (episode IS NULL))
) WITHOUT ROWID;

-- One effect on a file of a work folder: storing (`store`, the received bytes
-- copied into `.trss/subtitles/`) or applying (`apply`, a stored subtitle
-- copied beside its video). Each is written before it happens and after it is
-- confirmed: `intended` (its own temporary file `temp` in the work folder's
-- `.trss/tmp/`, the path `target` it is to be published at, the bytes'
-- length and SHA-256 as their source has them), `prepared` (the temporary
-- file written, synced and read back, with its `object`), then `done` once a
-- rename that replaces nothing published it and its folder was synced;
-- `held`, `failed` or `abandoned` (nothing of it was left) otherwise.
-- `folder` is the work folder as it was when the effect was intended, and
-- the paths are relative to it; `video` is the video an applied copy is
-- put beside.
CREATE TABLE subtitle_file_effects (
    id         TEXT    PRIMARY KEY CHECK (id <> ''),
    job_id     TEXT    NOT NULL,
    position   INTEGER NOT NULL,
    kind       TEXT    NOT NULL CHECK (kind IN ('store', 'apply')),
    state      TEXT    NOT NULL CHECK (state IN
                   ('intended', 'prepared', 'done', 'held', 'failed', 'abandoned')),
    folder     TEXT    NOT NULL CHECK (folder <> ''),
    temp       TEXT    NOT NULL CHECK (temp <> ''),
    target     TEXT    NOT NULL CHECK (target <> ''),
    video      TEXT    CHECK ((kind = 'apply') = (video IS NOT NULL)),
    size       INTEGER NOT NULL CHECK (size >= 0),
    sha256     TEXT    NOT NULL CHECK (length(sha256) = 64),
    object     TEXT,
    reason     TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (job_id, position) REFERENCES subtitle_job_plan (job_id, position)
        ON DELETE CASCADE,
    CHECK ((state = 'prepared') <= (object IS NOT NULL))
) WITHOUT ROWID;

CREATE INDEX subtitle_file_effects_by_row ON subtitle_file_effects (job_id, position);
-- Two effects under way never aim at one path, whatever its letters' case.
CREATE UNIQUE INDEX subtitle_file_effects_one_target
    ON subtitle_file_effects (folder, lower(target))
    WHERE state IN ('intended', 'prepared');

-- The candidates' jobs that received before storing and applying were made go
-- on from their receipts: the worker takes them up again and does not receive
-- them anew. Uploads and find jobs wait for a person's placement first and
-- are left as they are.
UPDATE subtitle_jobs
   SET state = 'pending', wait = NULL, stage = NULL, finished_at = NULL,
       note = '받아 둔 파일의 보관과 적용을 기다려요',
       state_at = CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER),
       updated_at = CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
 WHERE state IN ('done', 'partial') AND origin NOT IN ('upload', 'find');
