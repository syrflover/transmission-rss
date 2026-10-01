-- A number that changes whenever what a video lookup by absolute path can
-- answer changes (`LibraryStore::generation`), so the worker can tell that a
-- question it asked of the library before would be answered the same way now
-- without asking it again.
--
-- The database keeps it, not the code that writes the library, so every
-- writer counts (the worker's scans and watches, the web's registering and
-- rescanning, an archive move) and none can forget to. A row is a video
-- lookup's answer, so the triggers watch the files, the place of a work
-- folder (its watch folder and its name) and whether a watch folder counts.
-- A scan that finds everything as it was writes none of these columns, so it
-- leaves the number alone. Rows that only change `missing`, times or notes do
-- not matter to a lookup and are not watched.

CREATE TABLE library_generation (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    generation INTEGER NOT NULL
);

INSERT INTO library_generation (id, generation) VALUES (1, 0);

CREATE TRIGGER library_generation_file_added AFTER INSERT ON media_files
BEGIN
    UPDATE library_generation SET generation = generation + 1;
END;

CREATE TRIGGER library_generation_file_removed AFTER DELETE ON media_files
BEGIN
    UPDATE library_generation SET generation = generation + 1;
END;

CREATE TRIGGER library_generation_file_changed
    AFTER UPDATE OF work_id, path, season, kind ON media_files
BEGIN
    UPDATE library_generation SET generation = generation + 1;
END;

CREATE TRIGGER library_generation_work_moved
    AFTER UPDATE OF watch_folder_id, dir_name ON works
BEGIN
    UPDATE library_generation SET generation = generation + 1;
END;

CREATE TRIGGER library_generation_work_removed AFTER DELETE ON works
BEGIN
    UPDATE library_generation SET generation = generation + 1;
END;

CREATE TRIGGER library_generation_folder_changed
    AFTER UPDATE OF path, unregistered_at ON watch_folders
BEGIN
    UPDATE library_generation SET generation = generation + 1;
END;

CREATE TRIGGER library_generation_folder_removed AFTER DELETE ON watch_folders
BEGIN
    UPDATE library_generation SET generation = generation + 1;
END;
