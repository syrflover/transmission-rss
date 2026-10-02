-- Title-waiting subscriptions (docs/specs/collection.md, 방영작 구독).
--
-- `rule_subscriptions.titled_at` (Unix ms) is when the match phrase of a
-- subscription was last given or cleared: a subscription that waited for its
-- title (a rule whose match phrase was NULL) was given one, or one that had a
-- phrase was blanked to wait again. What history first saw before then, without any rule taking it, is past for the
-- rule, as it is for what came before `subscribed_at` or while the rule was
-- paused: the cycle leaves it alone and only the user receives it
-- (`ChannelPlan::is_past`). NULL for a subscription that had its phrase from the
-- start.
--
-- `rejected_titles` holds the title candidates the user turned down: a work
-- (`title_key` is `subscriptions::work_key` of it, `work` the way it was
-- written) that history records in a channel and that must not be offered again
-- as a candidate. A rejection leaves the title-waiting subscriptions as they
-- are, and goes with its channel.
ALTER TABLE rule_subscriptions ADD COLUMN titled_at INTEGER;

CREATE TABLE rejected_titles (
    channel_id  TEXT    NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    title_key   TEXT    NOT NULL CHECK (title_key <> ''),
    work        TEXT    NOT NULL,
    rejected_at INTEGER NOT NULL,
    PRIMARY KEY (channel_id, title_key)
) WITHOUT ROWID;
