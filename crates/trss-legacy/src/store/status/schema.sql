-- Snapshots the worker records at each collection cycle for the collection
-- screen's status board. The web never reads RSS feeds or asks Transmission
-- itself; it shows what the worker last saw and when.
--
-- `channel_read_status` has one row per channel: whether the worker's last
-- attempt to read the feed worked, when it tried, and when a read last worked.
-- There is no foreign key to `channels`; the worker drops the rows of channels
-- that no longer exist each time it writes. Timestamps are Unix milliseconds.
--
-- `transmission_snapshot` is the single row of torrent counts from the
-- worker's last successful look at Transmission.

CREATE TABLE channel_read_status (
    channel_id TEXT    PRIMARY KEY,
    ok         INTEGER NOT NULL CHECK (ok IN (0, 1)),
    read_at    INTEGER NOT NULL,
    ok_at      INTEGER
);

CREATE TABLE transmission_snapshot (
    id          INTEGER PRIMARY KEY CHECK (id = 1),
    downloading INTEGER NOT NULL CHECK (downloading >= 0),
    seeding     INTEGER NOT NULL CHECK (seeding >= 0),
    taken_at    INTEGER NOT NULL
);
