//! Where a subscribed anime falls in a week, and which episode airs there.
//!
//! The weekday and time are the stored Anissia snapshot's ([`Anime`]); the
//! episode is a reading of it, since Anissia's schedule carries no episode
//! numbers (see [`episode_on`]).

use std::collections::BTreeMap;

use super::calendar::{day_start, weekday, weekday_of_anissia, PartialDate};
use crate::store::{
    anissia::{Anime, WEEK_UPCOMING},
    history::Millis,
};

/// The most an AniList air time may differ from the schedule's for the two to
/// be the same broadcast: a broadcast a day late is still that week's episode,
/// and the next one is a week away.
const SAME_BROADCAST_MS: i64 = 36 * 60 * 60 * 1000;

/// When an anime airs in the week: the day, the time Anissia gives (`HH:MM`,
/// the hour goes past 24 for late-night programmes) and the moment it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    pub day: i64,
    pub time: Option<String>,
    /// The Unix ms of the broadcast; the start of the day without a time.
    pub instant: Millis,
}

fn minutes_of(time: &str) -> Option<i64> {
    let (hour, minute) = time.split_once(':')?;
    Some(hour.parse::<i64>().ok()? * 60 + minute.parse::<i64>().ok()?)
}

/// The slot of `anime` in the week that begins on the Monday `week_start`, or
/// `None` when it does not air then:
///
/// - an anime on a weekday airs on that weekday;
/// - a `신작` anime (not started when Anissia listed it) airs on its start day,
///   if that is in the week, and has no time;
/// - `기타` has no weekday, so it is in no week;
/// - nothing airs before its start date or after its end date (a date that
///   knows the month only reaches as far as the month).
pub fn slot_in_week(anime: &Anime, week_start: i64) -> Option<Slot> {
    let start = anime.start_date.as_deref().and_then(PartialDate::parse);
    let day = match anime.week {
        0..=6 => week_start + i64::from(weekday_of_anissia(anime.week)),
        WEEK_UPCOMING => match start {
            Some(PartialDate::Day(day)) if (week_start..week_start + 7).contains(&day) => day,
            _ => return None,
        },
        _ => return None,
    };
    if start.is_some_and(|start| start.is_after(day)) {
        return None;
    }
    let end = anime.end_date.as_deref().and_then(PartialDate::parse);
    if end.is_some_and(|end| end.is_before(day)) {
        return None;
    }
    let time = anime.air_time.clone().filter(|t| minutes_of(t).is_some());
    let minutes = time.as_deref().and_then(minutes_of).unwrap_or(0);
    Some(Slot {
        day,
        time,
        instant: day_start(day) + minutes * 60_000,
    })
}

/// Why an anime's run is over ([`run_over`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Over {
    /// Its end date, as Anissia gave it, has passed.
    EndDate(String),
    /// It has no end date and Anissia no longer lists it.
    Unlisted,
}

/// Whether the anime's run is over on the Seoul day `today`, by the rule the
/// weekly schedule drops its card by ([`slot_in_week`], and `unlisted` from the
/// worker's daily refresh): an anime with an end date is over once the date has
/// passed (a date that knows the month only reaches as far as the month), and
/// only an anime without one is over when Anissia no longer lists it. An end
/// date that cannot be read says nothing, as it does there. While Anissia
/// cannot be reached `unlisted` is never found out, so nothing is over by it.
pub fn run_over(anime: &Anime, unlisted: bool, today: i64) -> Option<Over> {
    match &anime.end_date {
        Some(text) => PartialDate::parse(text)
            .is_some_and(|end| end.is_before(today))
            .then(|| Over::EndDate(text.clone())),
        None => unlisted.then_some(Over::Unlisted),
    }
}

/// The episode of the season that airs in `slot`, if it can be told.
///
/// - If the season's AniList entries know when each episode airs
///   (`air_times`: the season's episode number to its Unix ms, see
///   [`crate::seasons::combine::air_times`]), the episode that airs at the slot
///   (within [`SAME_BROADCAST_MS`], the nearest) is it. This follows breaks and
///   a second cour that goes on counting.
/// - Otherwise it is counted from Anissia's start date: the first airing is the
///   first day on the anime's weekday on or after it, and each week after it is
///   one episode more. A break week or a double episode makes this count wrong
///   from there on, which only AniList's schedule can tell.
/// - Without either, the episode is not known.
pub fn episode_on(anime: &Anime, slot: &Slot, air_times: &BTreeMap<u32, i64>) -> Option<u32> {
    let nearest = air_times
        .iter()
        .map(|(episode, at)| (*episode, (*at - slot.instant).abs()))
        .filter(|(_, apart)| *apart <= SAME_BROADCAST_MS)
        .min_by_key(|(_, apart)| *apart);
    if let Some((episode, _)) = nearest {
        return Some(episode);
    }
    let Some(PartialDate::Day(start)) = anime.start_date.as_deref().and_then(PartialDate::parse)
    else {
        return None;
    };
    if anime.week == WEEK_UPCOMING {
        return Some(1);
    }
    let first = start + i64::from((weekday_of_anissia(anime.week) + 7 - weekday(start)) % 7);
    let weeks = (slot.day - first).div_euclid(7);
    u32::try_from(weeks + 1).ok().filter(|n| *n >= 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::calendar::days_from_civil;
    use crate::store::anissia::WEEK_OTHER;

    fn anime(week: u8, time: Option<&str>, start: Option<&str>, end: Option<&str>) -> Anime {
        Anime {
            anime_no: 1,
            subject: "작품".into(),
            original_subject: None,
            week,
            air_time: time.map(str::to_owned),
            start_date: start.map(str::to_owned),
            end_date: end.map(str::to_owned),
            status: "ON".into(),
            fetched_at: 0,
        }
    }

    /// Monday 2026-09-28.
    fn monday() -> i64 {
        days_from_civil(2026, 9, 28)
    }

    #[test]
    fn a_weekday_anime_airs_on_its_day_at_its_time() {
        // Anissia's week 3 is Wednesday.
        let slot = slot_in_week(&anime(3, Some("23:30"), Some("2026-07-01"), None), monday())
            .expect("airs this week");
        assert_eq!(slot.day, monday() + 2);
        assert_eq!(slot.time.as_deref(), Some("23:30"));
        assert_eq!(
            slot.instant,
            day_start(monday() + 2) + (23 * 60 + 30) * 60_000
        );

        // Sunday is the last day of the week, Monday the first.
        assert_eq!(
            slot_in_week(&anime(0, None, None, None), monday())
                .unwrap()
                .day,
            monday() + 6
        );
        assert_eq!(
            slot_in_week(&anime(1, None, None, None), monday())
                .unwrap()
                .day,
            monday()
        );
    }

    #[test]
    fn a_late_night_time_goes_past_the_end_of_its_day() {
        let slot = slot_in_week(&anime(3, Some("25:30"), None, None), monday()).unwrap();
        assert_eq!(slot.day, monday() + 2);
        assert_eq!(slot.instant, day_start(monday() + 3) + 90 * 60_000);
    }

    #[test]
    fn an_anime_without_a_time_airs_from_the_start_of_its_day() {
        let slot = slot_in_week(&anime(3, None, None, None), monday()).unwrap();
        assert_eq!(slot.time, None);
        assert_eq!(slot.instant, day_start(monday() + 2));
    }

    #[test]
    fn nothing_airs_before_its_start_or_after_its_end() {
        // Wednesday 2026-09-30 of the week.
        let on = |start: Option<&str>, end: Option<&str>| {
            slot_in_week(&anime(3, Some("22:00"), start, end), monday()).is_some()
        };
        assert!(on(Some("2026-09-30"), None), "starts that day");
        assert!(!on(Some("2026-10-07"), None), "starts next week");
        assert!(on(Some("2026-09"), None), "starts in the month");
        assert!(!on(Some("2026-10"), None), "starts next month");
        assert!(on(None, Some("2026-09-30")), "ends that day");
        assert!(!on(None, Some("2026-09-23")), "ended last week");
        assert!(on(None, Some("2026-09")), "ends in the month");
        assert!(!on(None, Some("2026-08")), "ended last month");
        // A date that does not parse says nothing.
        assert!(on(Some("soon"), Some("later")));
    }

    #[test]
    fn a_not_started_anime_airs_on_its_start_day_when_that_is_this_week() {
        let new = |start: &str| anime(WEEK_UPCOMING, None, Some(start), None);
        let slot = slot_in_week(&new("2026-10-02"), monday()).unwrap();
        assert_eq!(slot.day, monday() + 4);
        assert_eq!(slot.time, None);
        assert_eq!(slot_in_week(&new("2026-10-05"), monday()), None);
        assert_eq!(slot_in_week(&new("2026-09-27"), monday()), None);
        // The month alone does not say which day.
        assert_eq!(slot_in_week(&new("2026-10"), monday()), None);
        assert_eq!(
            slot_in_week(&anime(WEEK_UPCOMING, None, None, None), monday()),
            None
        );
    }

    #[test]
    fn an_anime_with_no_weekday_airs_in_no_week() {
        assert_eq!(
            slot_in_week(
                &anime(WEEK_OTHER, Some("22:00"), Some("2026-07-01"), None),
                monday()
            ),
            None
        );
    }

    fn slot_on(week: u8, start: &str, offset: i64) -> (Anime, Slot) {
        let a = anime(week, Some("22:00"), Some(start), None);
        let slot = slot_in_week(&a, monday() + offset).expect("airs");
        (a, slot)
    }

    #[test]
    fn the_episode_counts_the_weeks_from_the_first_airing() {
        let none = BTreeMap::new();
        // Wednesday 2026-07-01 is the first airing: this Wednesday is the 14th.
        let (a, slot) = slot_on(3, "2026-07-01", 0);
        assert_eq!(slot.day, days_from_civil(2026, 9, 30));
        assert_eq!(episode_on(&a, &slot, &none), Some(14));
        // The week it starts is the first episode.
        let (a, slot) = slot_on(3, "2026-09-30", 0);
        assert_eq!(episode_on(&a, &slot, &none), Some(1));
        // A start date on another weekday: the first airing is the next Wednesday.
        let (a, slot) = slot_on(3, "2026-09-28", 0);
        assert_eq!(episode_on(&a, &slot, &none), Some(1));
        let (a, slot) = slot_on(3, "2026-09-28", 7);
        assert_eq!(episode_on(&a, &slot, &none), Some(2));
    }

    #[test]
    fn the_episode_is_unknown_without_a_start_day() {
        let none = BTreeMap::new();
        for start in [None, Some("2026-07")] {
            let a = anime(3, Some("22:00"), start, None);
            let slot = slot_in_week(&a, monday()).unwrap();
            assert_eq!(episode_on(&a, &slot, &none), None, "{start:?}");
        }
    }

    #[test]
    fn a_not_started_anime_airs_its_first_episode() {
        let a = anime(WEEK_UPCOMING, None, Some("2026-10-02"), None);
        let slot = slot_in_week(&a, monday()).unwrap();
        assert_eq!(episode_on(&a, &slot, &BTreeMap::new()), Some(1));
    }

    #[test]
    fn anilists_air_times_say_the_episode_when_they_know_it() {
        // A second cour that goes on counting at 13, with a break week before 15.
        let (a, slot) = slot_on(3, "2026-07-01", 0);
        let at = |day: i64, minutes: i64| day_start(day) + minutes * 60_000;
        let times = BTreeMap::from([
            (13, at(slot.day - 14, 22 * 60)),
            (14, at(slot.day - 7, 22 * 60)),
            (15, at(slot.day, 22 * 60 + 5)),
            (16, at(slot.day + 14, 22 * 60)),
        ]);
        assert_eq!(episode_on(&a, &slot, &times), Some(15));

        // An episode a day late is still the week's.
        let late = BTreeMap::from([(15, at(slot.day + 1, 22 * 60))]);
        assert_eq!(episode_on(&a, &slot, &late), Some(15));
        // Anything further off is another broadcast: counted from the start instead.
        let far = BTreeMap::from([(99, at(slot.day + 7, 22 * 60))]);
        assert_eq!(episode_on(&a, &slot, &far), Some(14));
    }

    #[test]
    fn the_run_is_over_by_the_end_date_or_only_without_one_by_the_listing() {
        let today = days_from_civil(2026, 10, 1);
        let over = |end: Option<&str>, unlisted: bool| {
            run_over(&anime(3, Some("22:00"), None, end), unlisted, today)
        };
        // The date has passed, whatever the listing says.
        assert_eq!(
            over(Some("2026-09-30"), false),
            Some(Over::EndDate("2026-09-30".into()))
        );
        // On the end date, and before it, the run is not over: even unlisted.
        assert_eq!(over(Some("2026-10-01"), true), None);
        assert_eq!(over(Some("2026-12-01"), true), None);
        // A month alone reaches its last day.
        assert_eq!(over(Some("2026-10"), false), None);
        assert_eq!(
            over(Some("2026-09"), false),
            Some(Over::EndDate("2026-09".into()))
        );
        // Without an end date only the listing says; an unreadable one says nothing.
        assert_eq!(over(None, false), None);
        assert_eq!(over(None, true), Some(Over::Unlisted));
        assert_eq!(over(Some("soon"), true), None);
    }
}
