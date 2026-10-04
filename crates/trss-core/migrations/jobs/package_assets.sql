-- A plan row that keeps a font, an attachment or a companion file of a
-- package (`kind` other than `subtitle`) keeps no stored subtitle: `asset_id`
-- is the asset it was stored as, set with its outcome `stored`, as
-- `stored_id` is for a subtitle (docs/specs/subtitles.md, 보관본과 적용본,
-- 폰트).
ALTER TABLE subtitle_job_plan ADD COLUMN asset_id TEXT REFERENCES subtitle_assets (id);

-- From here a store effect (`subtitle_file_effects`) of such a file is in the
-- app data folder: its `folder` is the app data folder, and its `temp` and
-- `target` are relative to it (`subtitle-files/.tmp/<id>`,
-- `subtitle-files/<work>/<creator>/<name>`). The others' `folder` is the
-- work folder, as before.
