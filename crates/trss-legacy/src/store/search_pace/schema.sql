-- The pace of the app's search requests to a feed host (`store::search_pace`,
-- ticket 0026): the past episode search reads a tracker's search RSS once, and
-- the extra searches of a long series follow each other. The web and the
-- worker are separate processes sharing this table, so the time the next
-- request may start is kept here, per host, and every request takes its slot
-- from it. `next_at` is the earliest start (Unix ms) of the next request;
-- `blocked_until` is set by a `429` answer's `Retry-After` and no request
-- starts before it.

CREATE TABLE search_pace (
    host          TEXT    PRIMARY KEY CHECK (host <> ''),
    next_at       INTEGER NOT NULL,
    blocked_until INTEGER
) WITHOUT ROWID;
