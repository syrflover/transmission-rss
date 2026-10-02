//! Anissia's notation of the weekly schedule's days: its week numbering and
//! its dates, which may know the month only. The Seoul calendar they are read
//! against is `trss_core::calendar`.

use trss_core::calendar::{civil_from_days, days_from_civil};

/// The weekday counted from Monday of Anissia's week number (0 is Sunday).
pub fn weekday_of_anissia(week: u8) -> u8 {
    (week + 6) % 7
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
    fn anissia_counts_its_week_from_sunday() {
        assert_eq!(weekday_of_anissia(0), 6);
        assert_eq!(weekday_of_anissia(1), 0);
        assert_eq!(weekday_of_anissia(3), 2);
        assert_eq!(weekday_of_anissia(6), 5);
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
