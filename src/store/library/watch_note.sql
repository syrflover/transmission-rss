-- What the worker's inotify watches could not cover in a watch folder, as a
-- sentence for the folder's row (see `worker::live`): how many directories have
-- no watch and why. NULL while every directory is watched, or while the worker
-- does not watch the folder. The worker writes it; the web only shows it.

ALTER TABLE watch_folders
    ADD COLUMN watch_note TEXT CHECK (watch_note IS NULL OR watch_note <> '');
