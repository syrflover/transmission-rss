-- Whether the worker is alive and busy, for the board's `RSS 확인이 멈췄어요`
-- (ticket 0021). The web cannot ask the worker, and must not touch its cycle
-- lock (taking even a try-lock could make the worker skip a cycle), so the
-- worker leaves a pulse in the database while it holds the lock.
--
-- `worker_heartbeat` is a single row: `beat_at` is the last time the worker wrote
-- it, every few seconds while it holds the cycle lock (a cycle with the watch
-- folder reading and the season link after it, the web's commands, or a reading
-- a watch folder's alert asked for), and once more when it lets go;
-- `held_since` is when the current hold began, or NULL once it let go. A worker
-- killed while holding leaves `held_since` set and a `beat_at` that ages. No row
-- means no worker of this version has held the lock.

CREATE TABLE worker_heartbeat (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    beat_at    INTEGER NOT NULL,
    held_since INTEGER
);
