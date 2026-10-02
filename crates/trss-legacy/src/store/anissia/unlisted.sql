-- When Anissia was last found not to list an anime (`docs/specs/web-app.md`,
-- 이번 주 편성): the worker's daily refresh asked every week of Anissia's
-- schedule, all of them answered, and none listed the anime. NULL while the
-- anime is listed or nobody has looked. A refresh that failed, was refused
-- (`429`) or was cut short never sets it, and a snapshot received again clears
-- it, so it only ever says "Anissia answered and the anime was not there". The
-- weekly schedule drops the card of an anime that has it; the snapshot stays,
-- and the anime is looked for again a day later (`refresh_not_before`).

ALTER TABLE anissia_anime ADD COLUMN unlisted_at INTEGER;
