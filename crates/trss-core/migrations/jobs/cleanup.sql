-- Cleaning a work's stored files, one stored subtitle at a time
-- (`docs/specs/library.md`, 보관 파일의 정리; `docs/specs/subtitles.md`,
-- 보관본과 적용본 and 폰트; `docs/specs/jobs.md`, 체크포인트와 중단 복구;
-- `trss_jobs::place::cleanup`).
--
-- `subtitle_stored.cleaned_at` is when a person's cleanup of the stored
-- subtitle was taken: from then it is no stored copy any more, so no list
-- shows it and no job picks, reuses or applies it. Its row stays, as the
-- plans and packages that name it do.
--
-- `subtitle_assets.removed_at` is set by the worker once the asset's file is
-- confirmed gone. The row stays (package entries and plan rows name it), and
-- its path is free again: a later store may publish another file there, so
-- one path holds one asset that is not removed.

ALTER TABLE subtitle_stored ADD COLUMN cleaned_at INTEGER;
ALTER TABLE subtitle_assets ADD COLUMN removed_at INTEGER;

DROP INDEX subtitle_assets_one_path;
CREATE UNIQUE INDEX subtitle_assets_one_path
    ON subtitle_assets (work_id, base, lower(relative_path))
    WHERE removed_at IS NULL;

-- One person's cleanup of a stored subtitle of the work: `asked` until the
-- worker has looked at each file it named, then `done`, or `held` with the
-- first file's `reason` that the worker did not remove (a work folder not
-- there, bytes other than recorded). A stored subtitle has one asked at a
-- time.
CREATE TABLE subtitle_cleanups (
    id         TEXT    PRIMARY KEY CHECK (id <> ''),
    work_id    TEXT    NOT NULL,
    stored_id  TEXT    NOT NULL REFERENCES subtitle_stored (id),
    state      TEXT    NOT NULL CHECK (state IN ('asked', 'done', 'held')),
    reason     TEXT,
    asked_at   INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK ((state = 'held') = (reason IS NOT NULL))
) WITHOUT ROWID;

CREATE UNIQUE INDEX subtitle_cleanups_one_asked ON subtitle_cleanups (stored_id)
    WHERE state = 'asked';
CREATE INDEX subtitle_cleanups_by_work ON subtitle_cleanups (work_id);

-- The files a cleanup removes: `named`, the ones the person was shown would
-- go with it; then what the worker found when it came to them, in one
-- transaction with the other writers of the records: `intended` (nothing uses
-- it any more: to be removed), `kept` (something took it up since: `reason`
-- says what), `done` (its file is confirmed gone, and the asset is
-- `removed_at`) or `held` (`reason`: it was not removed). The worker never
-- removes a file the person was not shown.
CREATE TABLE subtitle_asset_removals (
    cleanup_id TEXT NOT NULL REFERENCES subtitle_cleanups (id) ON DELETE CASCADE,
    asset_id   TEXT NOT NULL REFERENCES subtitle_assets (id),
    state      TEXT NOT NULL CHECK (state IN ('named', 'intended', 'done', 'kept', 'held')),
    reason     TEXT,
    PRIMARY KEY (cleanup_id, asset_id),
    CHECK ((state IN ('kept', 'held')) = (reason IS NOT NULL))
) WITHOUT ROWID;

CREATE INDEX subtitle_asset_removals_by_asset ON subtitle_asset_removals (asset_id);
