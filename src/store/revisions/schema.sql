-- The replacement of video revisions (`store::revisions`): one row per
-- history item that a rule selected as a higher revision (`14v2`) of an
-- episode whose folder already holds a video.
--
-- `item_id` is the new revision's history item and `old_item_id` the item of
-- the video it replaces, when the worker could tell which. `old_crc` is the
-- CRC32 of the episode's file as the worker read it when it decided (only
-- when it had to read it), and `old_torrent_hash` the torrent removed with
-- the old video, once the worker removes it. `folder` is the
-- rule's save folder and `episode_name` the episode's file name in it (the
-- name `trname` gives, which the old video holds). `expected_crc` is the
-- CRC32 the new release's name carries, eight upper-case hex digits; NULL
-- when the person received it with `다시 받기` and that request is the
-- confirmation. `received_name` and `file_crc` are the new file's name as
-- received and its CRC32 as read, once it has been checked, and
-- `file_identity` what told that file apart when it was read (device, inode,
-- size, modification and status-change times, `:`-joined): the old video is
-- removed only while the file under `received_name` is still that one.
-- `state` is where
-- the replacement is; the worker writes each step before it takes the next
-- (see `RevisionState`). `overtaken_by` is the row of the higher revision a
-- `skipped` row was skipped for while that one was on its way; when that row
-- fails, this one goes back to `receiving`. NULL for any other skip. Times
-- are Unix milliseconds; `replaced_at` is when the new video got the episode
-- name.

CREATE TABLE video_revisions (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    item_id       INTEGER NOT NULL UNIQUE REFERENCES history_items (id),
    old_item_id   INTEGER REFERENCES history_items (id),
    rule_id       TEXT    NOT NULL,
    folder        TEXT    NOT NULL CHECK (folder <> ''),
    episode_name  TEXT    NOT NULL CHECK (episode_name <> ''),
    old_version   INTEGER,
    old_crc       TEXT,
    old_torrent_hash TEXT,
    new_version   INTEGER NOT NULL,
    expected_crc  TEXT,
    torrent_hash  TEXT,
    received_name TEXT,
    file_crc      TEXT,
    file_identity TEXT,
    state         TEXT    NOT NULL CHECK (state IN ('unknown', 'skipped', 'receiving',
                      'verified', 'removing', 'removed', 'done', 'failed', 'cleared',
                      'abandoned')),
    reason        TEXT,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    replaced_at   INTEGER,
    overtaken_by  INTEGER REFERENCES video_revisions (id)
);

CREATE INDEX video_revisions_old_item ON video_revisions (old_item_id);
CREATE INDEX video_revisions_state ON video_revisions (state);
CREATE INDEX video_revisions_episode ON video_revisions (folder, episode_name);
CREATE INDEX video_revisions_hash ON video_revisions (torrent_hash);
CREATE INDEX video_revisions_overtaken_by ON video_revisions (overtaken_by);
