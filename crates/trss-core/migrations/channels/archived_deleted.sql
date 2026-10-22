-- `보관` deletes a rule now, and an archived work is one whose work folder is
-- in the archive folder (docs/adr/0022). The rules archived before go, as a
-- person's delete takes a rule: its subscription and its `수집 유지` with it
-- (their foreign keys cascade), its start (`rule_started`, by its trigger).
-- What is not the rule's stays: the collection history, which keeps the
-- rule's ID as a plain value, the received files, the seasons' and movies'
-- Anissia links and the episode mappings. Nothing links a deleted
-- subscription's anime to a season: a subscription with no season leaves none.

DELETE FROM rules WHERE state = 'archived';
