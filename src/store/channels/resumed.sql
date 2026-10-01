-- When a rule was last turned back on (`영상 받기` on, or restored from the
-- archive), in Unix milliseconds. The items that history first saw before
-- then, while the rule was paused or archived, are not received on their own:
-- the user picks them, like the items a subscription finds from before it began
-- (`ChannelPlan::is_past`). NULL for a rule that was never switched back on,
-- which then holds nothing back.
ALTER TABLE rules ADD COLUMN resumed_at INTEGER;
