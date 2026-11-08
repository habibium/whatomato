//! How a recurring message repeats, and when it next goes out. Each kind is a variant of
//! [`Recurrence`] with a module of its own. Rows hold it as JSON, so a new kind needs no migration.

mod cron;
mod days;
mod places;
mod sun;

use std::{iter, str::FromStr};

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
pub use sun::{Place, Sun, SunEvent, event_on};
use utoipa::ToSchema;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Recurrence {
    /// On every match of a cron expression
    Cron {
        /// 5 fields: minute hour day month weekday
        expression: String,
    },
    /// On the days `days` picks, at a time that moves with the sun
    Sun(Sun),
}

impl Recurrence {
    /// When it next goes out, strictly after `after`, its days read in `timezone`. `None` once it
    /// never will, or for a zone that isn't one, which only a row written outside the API can hold.
    #[must_use]
    pub fn next_after(&self, timezone: &str, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let zone = Tz::from_str(timezone).ok()?;
        match self {
            Self::Cron { expression } => cron::next_run(expression, zone, after),
            Self::Sun(sun) => sun.next_after(zone, after),
        }
    }

    /// The next `count` times it goes out after `after`, soonest first
    #[must_use]
    pub fn next_runs(
        &self,
        timezone: &str,
        after: DateTime<Utc>,
        count: usize,
    ) -> Vec<DateTime<Utc>> {
        iter::successors(self.next_after(timezone, after), |&run| {
            self.next_after(timezone, run)
        })
        .take(count)
        .collect()
    }

    /// Why it can't schedule anything in `timezone`, if so
    #[must_use]
    pub fn problem(&self, timezone: &str) -> Option<&'static str> {
        let zone = Tz::from_str(timezone).ok()?;
        let problem = match self {
            Self::Cron { expression } => cron::problem(expression),
            Self::Sun(sun) => sun.problem(zone),
        };
        problem.or_else(|| {
            self.next_after(timezone, Utc::now())
                .is_none()
                .then_some("never matches a date")
        })
    }

    /// As it's stored: trimmed, and a place rounded off
    #[must_use]
    pub fn normalized(&self) -> Self {
        match self {
            Self::Cron { expression } => Self::Cron {
                expression: expression.trim().to_owned(),
            },
            Self::Sun(sun) => Self::Sun(sun.normalized()),
        }
    }

    /// The place a sun schedule names, if it names one
    #[must_use]
    pub const fn place(&self) -> Option<Place> {
        match self {
            Self::Sun(sun) => sun.place,
            Self::Cron { .. } => None,
        }
    }

    /// For a sun schedule, the main city of `timezone`: where it watches the sun from unless it
    /// names a place of its own
    #[must_use]
    pub fn city(&self, timezone: &str) -> Option<&'static str> {
        match self {
            Self::Sun(_) => {
                let zone = Tz::from_str(timezone).ok()?;
                places::city(zone).map(|city| city.name)
            }
            Self::Cron { .. } => None,
        }
    }
}

/// Where the main city of `zone` is, for sun times where nothing names a place
#[must_use]
pub fn city_place(zone: Tz) -> Option<Place> {
    places::city(zone).map(|city| city.place)
}

/// Whether `timezone` is an IANA zone name
#[must_use]
pub fn is_timezone(timezone: &str) -> bool {
    Tz::from_str(timezone).is_ok()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use serde_json::json;

    use super::*;

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .unwrap()
    }

    fn cron(expression: &str) -> Recurrence {
        Recurrence::Cron {
            expression: expression.into(),
        }
    }

    #[test]
    fn reads_and_writes_as_tagged_json() {
        let sun = json!({
            "type": "sun",
            "event": "sunset",
            "offset_minutes": -10,
            "days": "* * 0-4",
            "place": null,
        });
        let recurrence: Recurrence = serde_json::from_value(sun.clone()).unwrap();
        assert_eq!(serde_json::to_value(&recurrence).unwrap(), sun);

        let cron = json!({ "type": "cron", "expression": "0 9 * * *" });
        let recurrence: Recurrence = serde_json::from_value(cron.clone()).unwrap();
        assert_eq!(serde_json::to_value(&recurrence).unwrap(), cron);
    }

    #[test]
    fn the_next_runs_follow_one_another() {
        assert_eq!(
            cron("0 9 * * *").next_runs("UTC", at(2026, 11, 5, 12, 0), 3),
            [
                at(2026, 11, 6, 9, 0),
                at(2026, 11, 7, 9, 0),
                at(2026, 11, 8, 9, 0)
            ]
        );
    }

    #[test]
    fn a_date_that_never_comes_is_refused() {
        assert_eq!(
            cron("0 0 31 2 *").problem("UTC"),
            Some("never matches a date")
        );
        assert_eq!(cron("0 9 * * *").problem("UTC"), None);
    }

    #[test]
    fn only_a_sun_schedule_has_a_city() {
        let sun = Recurrence::Sun(Sun {
            event: SunEvent::Sunrise,
            offset_minutes: 0,
            days: "* * *".into(),
            place: None,
        });
        assert_eq!(sun.city("Asia/Dhaka"), Some("Dhaka"));
        assert_eq!(sun.city("Asia/Calcutta"), Some("Kolkata"));
        assert_eq!(sun.city("UTC"), None);
        assert_eq!(cron("0 9 * * *").city("Asia/Dhaka"), None);
    }

    #[test]
    fn zones_are_iana_names() {
        assert!(is_timezone("Asia/Dhaka"));
        assert!(is_timezone("UTC"));
        assert!(!is_timezone("Mars/Base"));
        assert!(!is_timezone(""));
    }
}
