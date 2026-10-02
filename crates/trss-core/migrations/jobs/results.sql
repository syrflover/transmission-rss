-- The common receive result (`docs/specs/jobs.md`, 공통 수신 결과와 실패
-- 분류): what a receipt of a file came to, the same for every source.
--
-- `subtitle_job_files`:
-- - `format` is what the received bytes are (`zip`, `ass`, `srt`, `smi`,
--   `other`), found when they were checked before the file was published.
-- - `failure` is the class of a failed receipt (`missing`, `expired`,
--   `not_a_file`, `changed`, `network`); an attempt abandoned to be tried
--   again keeps the class of what stopped it.
-- - `http_status`, `content_type` (the media type alone) and `response_size`
--   are the facts of the answer: the status and type of the answer the bytes
--   came in, and for a failure the size of the answer that showed it.
-- - `snapshot` is what the source read about the file and its post, as a
--   JSON array of `[name, value]` pairs (Tistory's `article:modified_time`
--   and the size the post shows), to compare with the next snapshot of the
--   same path.
--
-- `subtitle_job_items.failure` is the class of a failed item.
--
-- Like the rest of the job records, none of these holds an address with its
-- query, a cookie or a token. The rows of earlier builds have none of them.

ALTER TABLE subtitle_job_files ADD COLUMN format TEXT
    CHECK (format IN ('zip', 'ass', 'srt', 'smi', 'other'));
ALTER TABLE subtitle_job_files ADD COLUMN failure TEXT
    CHECK (failure IN ('missing', 'expired', 'not_a_file', 'changed', 'network'));
ALTER TABLE subtitle_job_files ADD COLUMN http_status INTEGER
    CHECK (http_status BETWEEN 100 AND 599);
ALTER TABLE subtitle_job_files ADD COLUMN content_type TEXT;
ALTER TABLE subtitle_job_files ADD COLUMN response_size INTEGER CHECK (response_size >= 0);
ALTER TABLE subtitle_job_files ADD COLUMN snapshot TEXT
    CHECK (snapshot IS NULL OR json_valid(snapshot));

ALTER TABLE subtitle_job_items ADD COLUMN failure TEXT
    CHECK (failure IN ('missing', 'expired', 'not_a_file', 'changed', 'network'));
