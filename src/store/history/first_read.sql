-- The first read of a channel: which of its items the feed already held then
-- (set aside as past for a subscription) and when it was.
--
-- `history_items.first_read` marks the items recorded by the channel's first
-- read: the ones its first record and the records made in the same cycle
-- wrote. Whether an item is past because of the first read used to be told by
-- comparing its `first_seen_at` with the channel's earliest one, which a clock
-- that goes back or forward breaks either way: a later item looked older than
-- the first read, or the first read moved earlier and let a subscription take
-- items set aside on it. Membership does not depend on any clock.
--
-- `history_first_reads` has the time of that first record, written once and
-- never changed. It tells whether a channel has been read at all, which is
-- what the first read is, and when. Like the rest of the history it has no
-- foreign key: it is a fact about records that outlive their channel.
--
-- The records from before are taken by what they were read by before: a
-- channel's first read is its earliest `first_seen_at`, and the items first
-- seen then are the ones its first read recorded.

CREATE TABLE history_first_reads (
    channel_id    TEXT    PRIMARY KEY CHECK (channel_id <> ''),
    first_read_at INTEGER NOT NULL
) WITHOUT ROWID;

INSERT INTO history_first_reads (channel_id, first_read_at)
SELECT channel_id, MIN(first_seen_at) FROM history_items GROUP BY channel_id;

ALTER TABLE history_items
    ADD COLUMN first_read INTEGER NOT NULL DEFAULT 0 CHECK (first_read IN (0, 1));

UPDATE history_items SET first_read = 1
WHERE first_seen_at = (
    SELECT first_read_at FROM history_first_reads
    WHERE history_first_reads.channel_id = history_items.channel_id);
