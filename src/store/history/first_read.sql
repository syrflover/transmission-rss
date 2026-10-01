-- When each channel was first read: the boundary between the items its feed
-- already held (set aside as past for a subscription) and the ones that came
-- later.
--
-- It is written once, by the first record of an item of the channel, with the
-- time of that sighting, and never changed afterwards. It used to be read as
-- the channel's earliest `first_seen_at`, which moves earlier when the clock
-- goes back and a new item is recorded, and then lets a subscription take
-- items set aside on the first read.
--
-- Like the rest of the history it has no foreign key: it is a fact about
-- records that outlive their channel. Timestamps are Unix milliseconds. The
-- channels that already have records get their earliest `first_seen_at`, which
-- is what they were read by before.

CREATE TABLE history_first_reads (
    channel_id    TEXT    PRIMARY KEY CHECK (channel_id <> ''),
    first_read_at INTEGER NOT NULL
) WITHOUT ROWID;

INSERT INTO history_first_reads (channel_id, first_read_at)
SELECT channel_id, MIN(first_seen_at) FROM history_items GROUP BY channel_id;
