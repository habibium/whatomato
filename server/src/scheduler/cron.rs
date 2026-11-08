//! Five-field cron expressions, read in an IANA time zone

use std::str::FromStr;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use croner::{Cron, parser::CronParser};

/// Minute granularity on purpose: anything finer would risk a WhatsApp ban
fn parse(expression: &str) -> Option<Cron> {
    if expression.split_whitespace().count() != 5 {
        return None;
    }
    CronParser::new().parse(expression).ok()
}

/// Whether `timezone` is an IANA zone name
#[must_use]
pub fn is_timezone(timezone: &str) -> bool {
    Tz::from_str(timezone).is_ok()
}

/// Why `expression` can't schedule anything in `timezone`, if so
#[must_use]
pub fn problem(expression: &str, timezone: &str) -> Option<&'static str> {
    if parse(expression).is_none() {
        return Some("must be a cron expression with 5 fields: minute hour day month weekday");
    }
    next_run(expression, timezone, Utc::now())
        .is_none()
        .then_some("never matches a date")
}

/// When `expression` next matches, strictly after `after`. `None` if it never does, or if either
/// argument is invalid, which only a row written outside the API can be.
#[must_use]
pub fn next_run(expression: &str, timezone: &str, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let cron = parse(expression)?;
    let zone = Tz::from_str(timezone).ok()?;
    let next = cron
        .find_next_occurrence(&after.with_timezone(&zone), false)
        .ok()?;
    Some(next.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .unwrap()
    }

    #[test]
    fn a_daily_time_is_read_in_the_schedules_zone() {
        assert_eq!(
            next_run("0 9 * * *", "Asia/Dhaka", at(2026, 11, 5, 2, 0)),
            Some(at(2026, 11, 5, 3, 0))
        );
        assert_eq!(
            next_run("0 9 * * *", "UTC", at(2026, 11, 5, 2, 0)),
            Some(at(2026, 11, 5, 9, 0))
        );
    }

    #[test]
    fn the_next_run_is_strictly_after_the_time_given() {
        assert_eq!(
            next_run("0 9 * * *", "UTC", at(2026, 11, 5, 9, 0)),
            Some(at(2026, 11, 6, 9, 0))
        );
    }

    #[test]
    fn a_daily_time_keeps_to_the_clock_across_daylight_saving() {
        // London moves from BST to GMT on 2026-10-25
        assert_eq!(
            next_run("0 9 * * *", "Europe/London", at(2026, 10, 24, 9, 0)),
            Some(at(2026, 10, 25, 9, 0))
        );
        assert_eq!(
            next_run("0 9 * * *", "Europe/London", at(2026, 10, 24, 7, 0)),
            Some(at(2026, 10, 24, 8, 0))
        );
    }

    #[test]
    fn weekdays_count_from_sunday_as_zero() {
        // 2026-11-05 is a Thursday
        assert_eq!(
            next_run("30 8 * * 1,5", "UTC", at(2026, 11, 5, 12, 0)),
            Some(at(2026, 11, 6, 8, 30))
        );
        assert_eq!(
            next_run("30 8 * * 0", "UTC", at(2026, 11, 5, 12, 0)),
            Some(at(2026, 11, 8, 8, 30))
        );
    }

    #[test]
    fn only_five_fields_pass() {
        assert_eq!(problem("0 9 * * *", "UTC"), None);
        for expression in [
            "",
            "* * * *",
            "*/5 * * * * *",
            "@daily",
            "99 * * * *",
            "nope",
        ] {
            assert!(problem(expression, "UTC").is_some(), "{expression}");
        }
    }

    #[test]
    fn a_date_that_never_comes_is_refused() {
        assert_eq!(problem("0 0 31 2 *", "UTC"), Some("never matches a date"));
    }

    // The web's schedule builder writes each of these forms
    #[test]
    fn the_builders_schedules_match_as_described() {
        // 2026-11-05 is a Thursday
        let from = at(2026, 11, 5, 12, 0);
        for (expression, next) in [
            ("0 9 * * 1-4", at(2026, 11, 9, 9, 0)),
            ("0 9 * * 0-4", at(2026, 11, 8, 9, 0)),
            ("0 9 * * 1,3,5", at(2026, 11, 6, 9, 0)),
            ("0 9 15 * *", at(2026, 11, 15, 9, 0)),
            ("0 9 L * *", at(2026, 11, 30, 9, 0)),
            ("0 9 * * 1#1", at(2026, 12, 7, 9, 0)),
            ("0 9 * * 5L", at(2026, 11, 27, 9, 0)),
            ("0 9 12 3 *", at(2027, 3, 12, 9, 0)),
            ("0 9 29 2 *", at(2028, 2, 29, 9, 0)),
        ] {
            assert_eq!(
                next_run(expression, "UTC", from),
                Some(next),
                "{expression}"
            );
        }
    }

    #[test]
    fn zones_are_iana_names() {
        assert!(is_timezone("Asia/Dhaka"));
        assert!(is_timezone("UTC"));
        assert!(!is_timezone("Mars/Base"));
        assert!(!is_timezone(""));
    }
}
