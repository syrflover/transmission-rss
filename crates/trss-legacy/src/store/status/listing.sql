-- Every torrent Transmission held when the worker last looked, for the web's
-- past episode search (ticket 0026): it cannot ask Transmission, yet must tell
-- an item whose torrent is still there from one whose torrent was removed.
-- `transmission_downloading` holds only the torrents being downloaded.
--
-- `transmission_listing` is the single row that says when the list was taken;
-- no row means the worker has not written one, and nothing is known about the
-- torrents that are gone. `transmission_torrents` holds the hashes, lowercase,
-- replaced as a whole each time. A failed look leaves the previous list.

CREATE TABLE transmission_listing (
    id       INTEGER PRIMARY KEY CHECK (id = 1),
    taken_at INTEGER NOT NULL
);

CREATE TABLE transmission_torrents (
    hash TEXT PRIMARY KEY CHECK (hash <> '')
) WITHOUT ROWID;
