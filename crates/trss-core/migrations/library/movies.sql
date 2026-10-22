-- Movies (`docs/requirements/library.md`, 작품 연결과 제외 and 자막의 회차 대응;
-- `docs/adr/0018-movies-in-season-zero-with-their-own-link.md`): a movie is
-- an episode of season 0 that has an Anissia anime of its own.
--
-- `movie_anissia` is a movie's link to an Anissia anime (see
-- `trss_library::store::seasons::movie`): the anime whose subtitles the movie
-- gets candidates from. A movie is the work's season 0, episode `episode`, so
-- the season is not a column. The movie of a work that has only a movie is its
-- one video directly in the work folder, recorded as season 0, episode 1.
--
-- Like `season_anissia`, the link is the movie's, not a subscription's;
-- `anime_no` is Anissia's `animeNo` and is NULL for a link that was cut;
-- `version` goes up with every change of the link (a movie with no row has
-- version 0), so a change made from an older version changes nothing. The link
-- has no reference to `episodes`, so a scan that misses the video for a moment
-- does not lose what the user linked; it goes with the work. `episode` is a
-- whole number from 1, which is how a subtitle job places a file on an
-- episode.
--
-- A work whose only video is a movie (a media file directly in the work folder,
-- in season 0) gets its season info's automatic search the way a first season
-- does (`season_info_for_new_first_season` in the seasons migration): season 0
-- is that work's first season. Nothing is created for the works that exist
-- when this migration runs, and none of them has such a file.
--
-- `subtitle_jobs.movie` is the episode of season 0 that a job for a movie
-- places every subtitle on, whatever the candidates' episode texts say;
-- NULL for any other job.

CREATE TABLE movie_anissia (
    work_id  TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    episode  INTEGER NOT NULL CHECK (episode >= 1),
    anime_no INTEGER REFERENCES anissia_anime (anime_no),
    version  INTEGER NOT NULL CHECK (version >= 1),
    PRIMARY KEY (work_id, episode)
) WITHOUT ROWID;

CREATE TRIGGER season_info_for_new_movie AFTER INSERT ON media_files
WHEN NEW.season = 0 AND NEW.kind = 'video' AND instr(NEW.path, '/') = 0
BEGIN
    INSERT OR IGNORE INTO season_info (work_id, season, job, job_requested_at)
    VALUES (NEW.work_id, 0, 'search',
            CAST((julianday('now') - 2440587.5) * 86400000 AS INTEGER));
END;

ALTER TABLE subtitle_jobs ADD COLUMN movie INTEGER CHECK (movie IS NULL OR movie >= 1);
