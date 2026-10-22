-- The cover of a subscription that is not connected to a season yet (see
-- `trss_library::store::artwork::subscription` and
-- `docs/requirements/library.md`, 작품 표지): until its rule receives a video,
-- the subscription has no work whose cover the schedule's card and the rule's
-- poster could show.
--
-- - The row is made when the subscribe screen or a rule's `Anissia 연결`
--   makes the subscription, with a `search` to do (an import makes none). It
--   goes with the subscription (`ON DELETE CASCADE`), and when the
--   subscription is connected to a season, which hands its image to the work
--   whose cover has none yet.
-- - `search` looks the Anissia anime's original title up on AniList and keeps
--   the one entry whose native title and start date agree; `fetch` receives
--   that entry's image (`job_image_url`, the cover URL the verified search
--   answer gave). `anilist_media_id` is set from the search on.
-- - The `image_*` columns are all set or all NULL, as `work_artwork`'s: the
--   image is a file of `artwork_files`, and the cleanup keeps it while this
--   table or `work_artwork` refers to it.
-- - `job_requested_at` names the request a running job serves: a result is
--   kept only while the row still waits for that same job.
--
-- Times are Unix milliseconds.

CREATE TABLE subscription_artwork (
    rule_id          TEXT    PRIMARY KEY REFERENCES rule_subscriptions (rule_id) ON DELETE CASCADE,
    anilist_media_id INTEGER CHECK (anilist_media_id IS NULL OR anilist_media_id > 0),
    image_id         TEXT    CHECK (image_id IS NULL OR image_id <> ''),
    image_path       TEXT    CHECK (image_path IS NULL OR image_path <> ''),
    image_size       INTEGER CHECK (image_size IS NULL OR image_size >= 0),
    image_sha256     TEXT    CHECK (image_sha256 IS NULL OR (length(image_sha256) = 64
                                    AND image_sha256 NOT GLOB '*[^0-9a-f]*')),
    image_format     TEXT    CHECK (image_format IS NULL OR image_format IN ('jpeg', 'png', 'webp')),
    job              TEXT    CHECK (job IS NULL OR job IN ('search', 'fetch')),
    job_requested_at INTEGER,
    job_attempts     INTEGER NOT NULL DEFAULT 0,
    job_not_before   INTEGER,
    job_image_url    TEXT,
    note             TEXT,
    note_at          INTEGER,
    CHECK ((image_id IS NULL) = (image_path IS NULL)
       AND (image_id IS NULL) = (image_size IS NULL)
       AND (image_id IS NULL) = (image_sha256 IS NULL)
       AND (image_id IS NULL) = (image_format IS NULL)),
    CHECK (image_id IS NULL OR anilist_media_id IS NOT NULL),
    CHECK (job IS NULL OR job_requested_at IS NOT NULL),
    CHECK (job IS NOT 'fetch' OR anilist_media_id IS NOT NULL)
) WITHOUT ROWID;

CREATE INDEX subscription_artwork_jobs ON subscription_artwork (job_requested_at, rule_id)
    WHERE job IS NOT NULL;
