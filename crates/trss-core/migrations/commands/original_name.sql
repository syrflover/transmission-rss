-- `original_name` is the name Transmission reported for the single file of
-- the torrent a command put in, recorded before the command first renamed
-- it. A start that comes after a worker died derives the file's name from it
-- again, never from the name the file has by then, so no episode conversion
-- is applied twice. NULL until a command records it.

ALTER TABLE commands ADD COLUMN original_name TEXT;
