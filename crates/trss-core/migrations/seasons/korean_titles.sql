-- The Korean titles of an AniList entry (`anilist_entries.korean_titles`): the
-- entry's AniList synonyms that contain Hangul, as a JSON list in AniList's
-- order. Anissia's full-list search matches Korean titles, so the work detail
-- shows them next to the entry's other titles for the user to search with
-- (`docs/specs/library.md`, 작품 연결과 제외).
--
-- Entries stored before this migration have an empty list. They get their
-- titles when the entry is received from AniList again, which the worker does
-- daily for an entry that is not finished and the user does with `정보 다시
-- 받기`; nothing here asks AniList for them.

ALTER TABLE anilist_entries ADD COLUMN korean_titles TEXT NOT NULL DEFAULT '[]'
    CHECK (json_valid(korean_titles));
