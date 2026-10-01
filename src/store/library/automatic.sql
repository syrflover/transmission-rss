-- Watch folders the app registers itself: the collect folder and the archive
-- folder of the collection settings are always watch folders (see
-- `store::library`). `automatic` is 1 for those; they cannot be unregistered by
-- hand and go when the settings stop using their path. A folder the user
-- registered by hand becomes automatic (keeping its records) when the settings
-- name its path.

ALTER TABLE watch_folders
    ADD COLUMN automatic INTEGER NOT NULL DEFAULT 0 CHECK (automatic IN (0, 1));
