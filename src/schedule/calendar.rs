//! Days and weeks of the Seoul calendar, which is the weekly schedule's clock
//! (Anissia's times are Asia/Seoul). A day is its count since 1970-01-01.

use crate::{
    store::history::Millis,
    subscriptions::{civil_from_days, DAY_MS, KST_OFFSET_MS},
};

/// The Seoul day a moment (Unix ms) is in.
pub fn day_of(ms: Millis) -> i64 {
    (ms + KST_OFFSET_MS).div_euclid(DAY_MS)
}

/// The moment (Unix ms) a Seoul day begins.
pub fn day_start(day: i64) -> Millis {
    day * DAY_MS - KST_OFFSET_MS
}

/// The day count of a civil date (proleptic Gregorian).
pub fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (i64::from(month) + 9) % 12;
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The weekday of a day, counted from Monday (0) to Sunday (6): the week of
/// the schedule begins on Monday.
pub fn weekday(day: i64) -> u8 {
    // 1970-01-01 was a Thursday.
    (day + 3).rem_euclid(7) as u8
}

/// The weekday counted from Monday of Anissia's week number (0 is Sunday).
pub fn weekday_of_anissia(week: u8) -> u8 {
    (week + 6) % 7
}

/// The Monday of the week a day is in.
pub fn week_start(day: i64) -> i64 {
    day - i64::from(weekday(day))
}

/// `2026-10-01` for a day.
pub fn date_text(day: i64) -> String {
    let (year, month, date) = civil_from_days(day);
    format!("{year:04}-{month:02}-{date:02}")
}

/// A date Anissia gave, which may know the month only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartialDate {
    Day(i64),
    Month(i32, u32),
}

impl PartialDate {
    /// `YYYY-MM-DD` or `YYYY-MM`.
    pub fn parse(text: &str) -> Option<PartialDate> {
        let mut parts = text.splitn(3, '-');
        let year: i32 = parts.next()?.parse().ok()?;
        let month: u32 = parts.next()?.parse().ok()?;
        if !(1..=12).contains(&month) {
            return None;
        }
        match parts.next() {
            None => Some(PartialDate::Month(year, month)),
            Some(day) => {
                let day: u32 = day.parse().ok()?;
                let valid = (1..=31).contains(&day)
                    && civil_from_days(days_from_civil(year, month, day)) == (year, month, day);
                valid.then(|| PartialDate::Day(days_from_civil(year, month, day)))
            }
        }
    }

    /// Whether `day` is before the date (the month's first day for a month).
    pub fn is_after(self, day: i64) -> bool {
        match self {
            PartialDate::Day(d) => d > day,
            PartialDate::Month(year, month) => {
                let (y, m, _) = civil_from_days(day);
                (year, month) > (y, m)
            }
        }
    }

    /// Whether `day` is after the date (the month's last day for a month).
    pub fn is_before(self, day: i64) -> bool {
        match self {
            PartialDate::Day(d) => d < day,
            PartialDate::Month(year, month) => {
                let (y, m, _) = civil_from_days(day);
                (year, month) < (y, m)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_and_dates_go_both_ways() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 10, 1), 20_362 + 365);
        for day in [-800, -1, 0, 59, 60, 11_016, 20_000, 20_787, 30_000] {
            let (y, m, d) = civil_from_days(day);
            assert_eq!(days_from_civil(y, m, d), day, "{y}-{m}-{d}");
        }
        assert_eq!(date_text(days_from_civil(2024, 2, 29)), "2024-02-29");
    }

    #[test]
    fn a_week_runs_from_monday_to_sunday() {
        // 2026-09-28 is a Monday and 2026-10-01 a Thursday.
        let monday = days_from_civil(2026, 9, 28);
        assert_eq!(weekday(monday), 0);
        assert_eq!(weekday(days_from_civil(2026, 10, 1)), 3);
        assert_eq!(weekday(days_from_civil(2026, 10, 4)), 6);
        assert_eq!(week_start(days_from_civil(2026, 10, 4)), monday);
        assert_eq!(week_start(monday), monday);
        assert_eq!(week_start(days_from_civil(2026, 10, 5)), monday + 7);
        // Anissia counts from Sunday.
        assert_eq!(weekday_of_anissia(0), 6);
        assert_eq!(weekday_of_anissia(1), 0);
        assert_eq!(weekday_of_anissia(3), 2);
        assert_eq!(weekday_of_anissia(6), 5);
    }

    #[test]
    fn a_moment_is_in_the_seoul_day_it_falls_in() {
        let day = days_from_civil(2026, 10, 1);
        assert_eq!(day_of(day_start(day)), day);
        assert_eq!(day_of(day_start(day) - 1), day - 1);
        assert_eq!(day_of(day_start(day + 1) - 1), day);
        // 00:00 in Seoul is 15:00 UTC the day before.
        assert_eq!(day_start(day) % DAY_MS, 15 * 60 * 60 * 1000);
    }

    #[test]
    fn a_date_may_know_its_month_only() {
        assert_eq!(
            PartialDate::parse("2026-10-07"),
            Some(PartialDate::Day(days_from_civil(2026, 10, 7)))
        );
        assert_eq!(
            PartialDate::parse("2026-10"),
            Some(PartialDate::Month(2026, 10))
        );
        for bad in [
            "",
            "2026",
            "2026-13",
            "2026-00",
            "2026-02-30",
            "x-1-1",
            "2026-10-",
        ] {
            assert_eq!(PartialDate::parse(bad), None, "{bad:?}");
        }

        let oct_7 = days_from_civil(2026, 10, 7);
        let by_day = PartialDate::Day(oct_7);
        assert!(by_day.is_after(oct_7 - 1) && !by_day.is_after(oct_7));
        assert!(by_day.is_before(oct_7 + 1) && !by_day.is_before(oct_7));
        let by_month = PartialDate::Month(2026, 10);
        assert!(by_month.is_after(days_from_civil(2026, 9, 30)));
        assert!(!by_month.is_after(days_from_civil(2026, 10, 1)));
        assert!(by_month.is_before(days_from_civil(2026, 11, 1)));
        assert!(!by_month.is_before(days_from_civil(2026, 10, 31)));
    }
}
