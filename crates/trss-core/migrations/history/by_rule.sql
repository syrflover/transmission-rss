-- The items a rule received, found by the rule: the worker asks which torrents
-- each subscription received on every cycle, and the history is kept
-- indefinitely, so without this index that is a read of every received item.

CREATE INDEX history_items_by_rule ON history_items (rule_id, result);
