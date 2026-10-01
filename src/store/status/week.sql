-- What the weekly schedule (`docs/specs/web-app.md`, 이번 주 편성) needs the
-- worker to leave besides the counts of `transmission_snapshot`.
--
-- `transmission_downloading` holds the hashes of the torrents Transmission was
-- downloading (or had queued to download) when the worker last looked, replaced
-- as a whole with the counts. A subscription's episode whose torrent is in it
-- is `영상 받는 중`. A failed look leaves the previous list, like the counts.
--
-- `worker_info` is the single row of the worker's own settings the web cannot
-- read from its environment: the time between two collection cycles in
-- milliseconds, written when the worker starts. The next RSS check is the last
-- cycle's start plus it.

CREATE TABLE transmission_downloading (
    hash TEXT PRIMARY KEY CHECK (hash <> '')
) WITHOUT ROWID;

CREATE TABLE worker_info (
    id                INTEGER PRIMARY KEY CHECK (id = 1),
    cycle_interval_ms INTEGER NOT NULL CHECK (cycle_interval_ms > 0)
);
