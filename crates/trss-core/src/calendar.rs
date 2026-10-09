//! Days and weeks of the Seoul calendar, which is the clock of the weekly
//! schedule and of the quarters (Anissia's times are Asia/Seoul). A day is its
//! count since 1970-01-01.

use crate::Millis;

/// Seoul is nine hours ahead of UTC, all year.
pub const KST_OFFSET_MS: i64 = 9 * 60 * 60 * 1000;
pub const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// The civil date of a day count since 1970-01-01 (proleptic Gregorian).
pub fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
    (year, month, day)
}

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

/// The Monday of the week a day is in.
pub fn week_start(day: i64) -> i64 {
    day - i64::from(weekday(day))
}

/// `2026-10-01` for a day.
pub fn date_text(day: i64) -> String {
    let (year, month, date) = civil_from_days(day);
    format!("{year:04}-{month:02}-{date:02}")
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
    fn days_since_the_epoch_become_calendar_dates() {
        assert_eq!(date_text(0), "1970-01-01");
        assert_eq!(date_text(-1), "1969-12-31");
        assert_eq!(date_text(19_723), "2024-01-01");
        assert_eq!(date_text(19_782), "2024-02-29");
        assert_eq!(date_text(19_783), "2024-03-01");
        assert_eq!(date_text(20_362), "2025-10-01");
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
}
