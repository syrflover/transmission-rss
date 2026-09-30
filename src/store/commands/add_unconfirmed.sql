-- `add_unconfirmed` is 1 on a command whose request to add a torrent to
-- Transmission got no answer (the connection failed or timed out), so
-- Transmission may hold a torrent whose hash history never learned. The next
-- collection cycle removes no departed torrents after such a command.

ALTER TABLE commands ADD COLUMN add_unconfirmed INTEGER NOT NULL DEFAULT 0
    CHECK (add_unconfirmed IN (0, 1));
