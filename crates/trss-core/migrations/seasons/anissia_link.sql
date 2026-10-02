-- A season's link to an Anissia anime (see `store::seasons::anissia` in
-- `trss-library` and `docs/specs/library.md`, 작품 연결과 제외): the anime whose
-- subtitles the season gets candidates from. The link is the season's, not a
-- subscription's: a season nobody subscribes to is linked from the work detail.
--
-- `anime_no` is Anissia's `animeNo` (a provider ID, never an app ID); the
-- anime's snapshot in `anissia_anime` gives its name, so a link always has one.
-- It is NULL for a season whose link was cut. `version` goes up with every
-- change of the link (a row exists from the first one on; a season with no row
-- has version 0), so a change made from an older version changes nothing.
--
-- A season that a subscription is connected to (`rule_subscriptions.season_id`,
-- `<work id>:<season number>`) holds that subscription's anime: the
-- connection of a subscription writes the link in its own transaction, and the
-- work detail refuses to change a link a subscription holds (the subscription
-- is deleted first; the link stays after that).
--
-- As with `season_info`, a season's link outlives the season's row in `seasons`
-- (a season folder that is moved away for a moment must not lose what the user
-- linked) and goes with the work.
--
-- The migration gives each season a subscription is connected to the
-- subscription's anime, as version 1. A subscription with no season yet, and
-- one whose work is gone, add nothing. If two subscriptions held one season
-- with different anime (the connection refuses that), the first rule by ID
-- gives its anime.

CREATE TABLE season_anissia (
    work_id  TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    season   INTEGER NOT NULL CHECK (season >= 0),
    anime_no INTEGER REFERENCES anissia_anime (anime_no),
    version  INTEGER NOT NULL CHECK (version >= 1),
    PRIMARY KEY (work_id, season)
) WITHOUT ROWID;

INSERT OR IGNORE INTO season_anissia (work_id, season, anime_no, version)
SELECT w.id,
       CAST(substr(s.season_id, length(w.id) + 2) AS INTEGER),
       s.anissia_anime_no,
       1
  FROM rule_subscriptions s
  JOIN works w ON substr(s.season_id, 1, length(w.id) + 1) = w.id || ':'
 WHERE substr(s.season_id, length(w.id) + 2) <> ''
   AND substr(s.season_id, length(w.id) + 2) NOT GLOB '*[^0-9]*'
 ORDER BY s.rule_id;
