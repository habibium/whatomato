//! The days a schedule goes out on, as cron's last three fields: day of the month, month, weekday.
//! `* * 0-4` is Sunday to Thursday, `L * *` the last day of every month.

use chrono::{NaiveDate, NaiveTime, TimeZone, Utc};
use croner::{Cron, parser::CronParser};

pub struct Days(Cron);

impl Days {
    #[must_use]
    pub fn parse(fields: &str) -> Option<Self> {
        if fields.split_whitespace().count() != 3 {
            return None;
        }
        // Any time of day would do, since only the date is read
        CronParser::new()
            .parse(&format!("0 12 {fields}"))
            .ok()
            .map(Self)
    }

    /// The first matching day on or after `from`
    #[must_use]
    pub fn first_from(&self, from: NaiveDate) -> Option<NaiveDate> {
        // A calendar without daylight saving, so no day is short of a noon
        let start = Utc.from_utc_datetime(&from.and_time(NaiveTime::MIN));
        let next = self.0.find_next_occurrence(&start, true).ok()?;
        Some(next.date_naive())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    #[test]
    fn finds_the_first_matching_day_from_the_one_given() {
        // 2026-11-05 is a Thursday
        let thursday = day(2026, 11, 5);
        for (fields, first) in [
            ("* * *", thursday),
            ("* * 0-4", thursday),
            ("* * 5,6", day(2026, 11, 6)),
            ("L * *", day(2026, 11, 30)),
            ("* * 1#1", day(2026, 12, 7)),
            ("29 2 *", day(2028, 2, 29)),
        ] {
            let days = Days::parse(fields).unwrap();
            assert_eq!(days.first_from(thursday), Some(first), "{fields}");
        }
    }

    #[test]
    fn only_three_fields_pass() {
        for fields in ["", "* *", "0 9 * * *", "32 * *", "* 13 *", "nope"] {
            assert!(Days::parse(fields).is_none(), "{fields}");
        }
    }
}
