-- A Google Drive font that is not received again because it did not change
-- (`docs/specs/subtitles.md`, 폰트; `trss_jobs::place::unchanged`).
--
-- Before a job receives a Drive file (`file_key` `drive:<id>`) that the work
-- keeps already as a font in the job's creator folder, a `HEAD` reads its
-- size and `Last-Modified`. When they are those of the latest receipt of the
-- same key that the font was kept from (its size, and the `last_modified` of
-- its snapshot, tied to the font through its plan row's `asset_id`), the file
-- is not received: its receipt is `done` with no path and no temporary
-- folder, and `unchanged_asset` names the font it uses instead (its size and
-- SHA-256 are the font's, its snapshot holds what the `HEAD` said). The
-- package then uses that font as it uses one received again with the same
-- bytes. A receipt whose font went away or changed before it was used is
-- `abandoned`, and the file is received anew.
--
-- No other column records the values: the receipts of the key are where they
-- are, so a later receipt of the same bytes becomes the record by itself.

ALTER TABLE subtitle_job_files ADD COLUMN unchanged_asset TEXT
    REFERENCES subtitle_assets (id)
    CHECK (unchanged_asset IS NULL OR path IS NULL);

-- The receipts of a key in every job, for the font a key was kept as.
CREATE INDEX subtitle_job_files_by_file_key ON subtitle_job_files (file_key);

-- The plan rows of a receipt: the font a key was kept as, and the rows a
-- receipt whose font went away takes back.
CREATE INDEX subtitle_job_plan_by_file ON subtitle_job_plan (file_id);

-- The receipts that use a font instead of receiving it, which a cleanup
-- keeps the font for while their job may still store it.
CREATE INDEX subtitle_job_files_unchanged ON subtitle_job_files (unchanged_asset)
    WHERE unchanged_asset IS NOT NULL;
