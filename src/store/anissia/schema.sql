-- Subscriptions to Anissia anime (see `store::anissia`, `store::channels` and
-- `docs/specs/collection.md`, 방영작 구독).
--
-- `anissia_anime` is the schedule snapshot of an anime a rule subscribes to, as
-- Anissia last listed it, so a rule's detail and the weekly schedule read
-- without asking Anissia. `anime_no` is Anissia's `animeNo` (a provider ID,
-- never an app ID). `week` is Anissia's group: 0 (Sunday) to 6 (Saturday), 7
-- (`기타`, no weekday) or 8 (`신작`, not started yet). `air_time` is `HH:MM` in
-- Asia/Seoul, NULL when Anissia gives none. `start_date` and `end_date` are
-- `YYYY-MM-DD`, or `YYYY-MM` when Anissia knows the month only; NULL when
-- unknown. `status` is Anissia's `ON`/`OFF`. `fetched_at` is when the row was
-- last received (Unix ms); `refresh_not_before` delays the next daily refresh
-- after a failed one or when Anissia no longer lists the anime.
--
-- `rule_subscriptions` is the subscription of a rule: a rule without a row here
-- is not a subscription. `subtitles` is how the subscription gets subtitles:
-- `follow` (the creator in `creator` is followed), `undecided` (`제작자 미정`,
-- no creator chosen yet) or `none` (`받지 않음`, video only); `creator` has a
-- value exactly for `follow`. `season_id` is the season the rule's videos belong to; it stays
-- NULL until the season is connected. `subscribed_at` (Unix ms) is when the
-- rule became a subscription: what the feed held and history had recorded
-- before it is past, and only the user receives that. Deleting the rule deletes
-- its row. A change of the row also changes the rule's `version`, in code.
--
-- `anissia_pace` is the pace of Anissia API requests, shared by the web and the
-- worker: the next request may start at `next_at`, and none before
-- `blocked_until` (set from a `429` answer's `Retry-After`).

CREATE TABLE anissia_anime (
    anime_no           INTEGER PRIMARY KEY CHECK (anime_no > 0),
    subject            TEXT    NOT NULL,
    original_subject   TEXT,
    week               INTEGER NOT NULL CHECK (week BETWEEN 0 AND 8),
    air_time           TEXT,
    start_date         TEXT,
    end_date           TEXT,
    status             TEXT    NOT NULL,
    fetched_at         INTEGER NOT NULL,
    refresh_not_before INTEGER
);

CREATE TABLE rule_subscriptions (
    rule_id          TEXT    PRIMARY KEY REFERENCES rules (id) ON DELETE CASCADE,
    anissia_anime_no INTEGER NOT NULL REFERENCES anissia_anime (anime_no),
    subtitles        TEXT    NOT NULL CHECK (subtitles IN ('follow', 'undecided', 'none')),
    creator          TEXT    CHECK (creator IS NULL OR creator <> ''),
    season_id        TEXT    CHECK (season_id IS NULL OR season_id <> ''),
    subscribed_at    INTEGER NOT NULL,
    CHECK ((subtitles = 'follow') = (creator IS NOT NULL))
) WITHOUT ROWID;

CREATE INDEX rule_subscriptions_by_anime ON rule_subscriptions (anissia_anime_no);

CREATE TABLE anissia_pace (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    next_at       INTEGER NOT NULL,
    blocked_until INTEGER
);
