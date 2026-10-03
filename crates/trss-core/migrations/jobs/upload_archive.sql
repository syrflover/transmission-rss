-- Which archive format an uploaded `archive` file is (`docs/specs/subtitles.md`,
-- 직접 찾기와 자막 올리기). An upload keeps an archive whole for the package
-- analysis whatever its format; the format is told by the first bytes alone
-- (`trss_subtitles::upload::Archive`): `zip` (also checked to its end), `rar`,
-- `7z`, `gz`, `bz2`, `xz` or `tar`. Files that are no archive, and the files
-- other jobs received, have none.

ALTER TABLE subtitle_job_files ADD COLUMN archive_type TEXT
    CHECK (archive_type IN ('zip', 'rar', '7z', 'gz', 'bz2', 'xz', 'tar'));
