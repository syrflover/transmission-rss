-- The grounds the app wrote when it set a rule's episode offset by itself
-- (`docs/specs/collection.md`, 영상 회차 변환): a sentence the rule's detail
-- shows beside `자동`. It is kept with the value because the AniList counts and
-- the library it came from can change afterwards, and the sentence has to say
-- why the value was set, not why it would be set now.
--
-- NULL for a rule whose offset the user typed (`episode_auto` = 0), and for a
-- rule whose automatic offset came in a file that carries no sentence. A save
-- that changes the offset, or turns `episode_auto` off, clears it.
ALTER TABLE rules ADD COLUMN episode_basis TEXT CHECK (episode_basis IS NULL OR episode_basis <> '');
