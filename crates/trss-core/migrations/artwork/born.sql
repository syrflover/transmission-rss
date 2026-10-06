-- The birth time of an image file the app made (`docs/specs/library.md`,
-- 이미지 파일의 수명; `trss_library::artwork::files`).
--
-- A file system mounted again may give the same file another device number
-- (btrfs numbers its devices at each mount), so a recorded file is known by
-- its inode and its birth time, not by `dev`. `born_ns` is the staged file's
-- birth time in nanoseconds since the Unix epoch, which the rename to the
-- published path keeps; NULL when the file system keeps none, and on the rows
-- recorded before this migration, which are then known by their inode alone.
-- `dev` is still recorded and no longer compared.

ALTER TABLE artwork_files ADD COLUMN born_ns INTEGER;
