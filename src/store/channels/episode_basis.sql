-- What the app keeps of a rule's episode offset when it sets it by itself
-- (`docs/specs/collection.md`, 영상 회차 변환).
--
-- `episode_basis` is the sentence the rule's detail shows beside `자동`. It is
-- kept with the value because the AniList counts and the library it came from
-- can change afterwards, and the sentence has to say why the value was set,
-- not why it would be set now. NULL for a rule whose offset the user typed
-- (`episode_auto` = 0), and for a rule whose automatic offset came in a file
-- that carries no sentence. A save that changes the offset, or turns
-- `episode_auto` off, clears it.
--
-- `episode_previous` is the offset the rule had just before the app set its
-- own (`되돌리기` puts it back). It goes with the automatic value: NULL once
-- the value is the user's (a save that changes it, `적용`, `되돌리기`), and
-- for an automatic value that came in a file. It is never exported: it only
-- tells how to rename the videos this database's worker named.
--
-- `episode_decided` is 1 once the app has set the rule's offset (or the rule
-- came in with an automatic one). The app decides a rule once: after the user
-- changed or undid its value, it does not decide again, even for a rule that
-- has not picked anything yet.
ALTER TABLE rules ADD COLUMN episode_basis TEXT CHECK (episode_basis IS NULL OR episode_basis <> '');
ALTER TABLE rules ADD COLUMN episode_previous INTEGER;
ALTER TABLE rules ADD COLUMN episode_decided INTEGER NOT NULL DEFAULT 0
    CHECK (episode_decided IN (0, 1));
UPDATE rules SET episode_decided = 1 WHERE episode_auto = 1;
