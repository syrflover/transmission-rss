-- App-wide settings. Today that is the collection folders.
--
-- `collection_settings` holds at most one row. No row means the collect folder
-- has not been chosen (a fresh database); the worker then adds no torrents.
-- `collect_folder` is where rules save what they select (a rule's `directory`
-- is relative to it). `archive_folder` is the paired folder archived works move
-- to, or NULL when archiving leaves folders alone. `version` starts at 1 and
-- increases on each change; a writer presents the version it last saw (0 when
-- it saw no row).

CREATE TABLE collection_settings (
    id             INTEGER PRIMARY KEY CHECK (id = 1),
    collect_folder TEXT    NOT NULL CHECK (collect_folder <> ''),
    archive_folder TEXT    CHECK (archive_folder IS NULL OR archive_folder <> ''),
    version        INTEGER NOT NULL CHECK (version >= 1)
);
