//! Sunrise and sunset, worked out for each day from the date and the place alone, so nothing is
//! fetched and nothing can go stale.

use chrono::{DateTime, DurationRound, NaiveDate, TimeDelta, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use sunrise::{Coordinates, SolarDay, SolarEvent};
use utoipa::ToSchema;

use super::{days::Days, places};

const OFFSET_LIMIT: i32 = 12 * 60;
/// Matching days to look through for one with the sun up and down. Polar night and the midnight
/// sun last half a year at most.
const SEARCH_DAYS: usize = 400;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SunEvent {
    /// The sun's upper edge clearing the horizon
    Sunrise,
    /// The sun's upper edge dropping below the horizon
    Sunset,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Place {
    /// Degrees north, from -90 to 90
    pub latitude: f64,
    /// Degrees east, from -180 to 180
    pub longitude: f64,
}

impl Place {
    const fn coordinates(self) -> Option<Coordinates> {
        Coordinates::new(self.latitude, self.longitude)
    }

    /// To a tenth of a degree, about 10 km: within seconds for the sun, and no closer to someone's home
    fn rounded(self) -> Self {
        let round = |degrees: f64| (degrees * 10.0).round() / 10.0;
        Self {
            latitude: round(self.latitude),
            longitude: round(self.longitude),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct Sun {
    pub event: SunEvent,
    /// Minutes after the sun rises or sets, or before when negative; up to 12 hours either way
    pub offset_minutes: i32,
    /// Cron's day fields: day of the month, month, weekday. `* * *` for every day.
    pub days: String,
    /// Where the sun is watched from; `null` for the main city of the message's time zone
    #[schema(required = true)]
    pub place: Option<Place>,
}

/// When the sun rises or sets at `place` on `day` in `zone`, to the minute as cron sends and as
/// `{{time}}` reads. `None` on a day it doesn't, as in polar night, or at a place that isn't one.
#[must_use]
pub fn event_on(event: SunEvent, day: NaiveDate, place: Place, zone: Tz) -> Option<DateTime<Utc>> {
    let coordinates = place.coordinates()?;
    let event = match event {
        SunEvent::Sunrise => SolarEvent::Sunrise,
        SunEvent::Sunset => SolarEvent::Sunset,
    };
    // The crate counts solar days, which can sit a date off the zone's far from its meridian
    [day.pred_opt()?, day, day.succ_opt()?]
        .into_iter()
        .filter_map(|solar| SolarDay::new(coordinates, solar).event_time(event))
        .find(|at| at.with_timezone(&zone).date_naive() == day)?
        .duration_round(TimeDelta::minutes(1))
        .ok()
}

impl Sun {
    pub(super) fn problem(&self, zone: Tz) -> Option<&'static str> {
        if Days::parse(&self.days).is_none() {
            return Some("needs its days as cron's day of the month, month and weekday fields");
        }
        if self.offset_minutes.abs() > OFFSET_LIMIT {
            return Some("can move the time by 12 hours at most");
        }
        match self.place {
            Some(place) if place.coordinates().is_none() => {
                Some("needs a latitude from -90 to 90 and a longitude from -180 to 180")
            }
            None if places::city(zone).is_none() => {
                Some("needs a place, as its time zone has no city to go by")
            }
            _ => None,
        }
    }

    pub(super) fn next_after(&self, zone: Tz, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let days = Days::parse(&self.days)?;
        let place = self
            .place
            .or_else(|| places::city(zone).map(|city| city.place))?;
        let offset = TimeDelta::minutes(self.offset_minutes.into());
        // The day before too, since an offset can carry a send over midnight
        let mut from = after
            .checked_sub_signed(offset)?
            .with_timezone(&zone)
            .date_naive()
            .pred_opt()?;
        for _ in 0..SEARCH_DAYS {
            let day = days.first_from(from)?;
            if let Some(event) = event_on(self.event, day, place, zone) {
                let send = event.checked_add_signed(offset)?;
                if send > after {
                    return Some(send);
                }
            }
            from = day.succ_opt()?;
        }
        None
    }

    pub(super) fn normalized(&self) -> Self {
        Self {
            event: self.event,
            offset_minutes: self.offset_minutes,
            days: self.days.split_whitespace().collect::<Vec<_>>().join(" "),
            place: self.place.map(Place::rounded),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .unwrap()
    }

    fn sun(event: SunEvent, offset_minutes: i32, days: &str, place: Option<Place>) -> Sun {
        Sun {
            event,
            offset_minutes,
            days: days.into(),
            place,
        }
    }

    const TROMSØ: Place = Place {
        latitude: 69.65,
        longitude: 18.96,
    };

    /// Published times are to the minute and round differently, so a minute either way agrees
    fn assert_near(actual: Option<DateTime<Utc>>, expected: DateTime<Utc>) {
        let actual = actual.expect("a time");
        assert!(
            (actual - expected).abs() <= TimeDelta::minutes(1),
            "{actual} is not within a minute of {expected}"
        );
    }

    #[test]
    fn matches_the_published_times() {
        use chrono_tz::{Asia::Dhaka, Europe::London};

        // From sunrise-sunset.org, to the nearest minute in UTC
        let every_day = |event| sun(event, 0, "* * *", None);
        assert_near(
            every_day(SunEvent::Sunrise).next_after(Dhaka, at(2026, 11, 7, 12, 0)),
            at(2026, 11, 8, 0, 8),
        );
        assert_near(
            every_day(SunEvent::Sunset).next_after(Dhaka, at(2026, 11, 8, 1, 0)),
            at(2026, 11, 8, 11, 16),
        );
        assert_near(
            every_day(SunEvent::Sunrise).next_after(London, at(2026, 6, 21, 0, 0)),
            at(2026, 6, 21, 3, 43),
        );
        assert_near(
            every_day(SunEvent::Sunset).next_after(London, at(2026, 6, 21, 12, 0)),
            at(2026, 6, 21, 20, 22),
        );
    }

    #[test]
    fn sends_on_the_minute() {
        let sunset = sun(SunEvent::Sunset, 0, "* * *", None);
        let next = sunset
            .next_after(chrono_tz::Asia::Dhaka, at(2026, 11, 8, 1, 0))
            .unwrap();
        assert_eq!(next.timestamp() % 60, 0);
    }

    #[test]
    fn the_next_run_is_strictly_after_the_time_given() {
        let zone = chrono_tz::Asia::Dhaka;
        let sunset = sun(SunEvent::Sunset, 0, "* * *", None);
        let today = sunset.next_after(zone, at(2026, 11, 8, 1, 0)).unwrap();
        let tomorrow = sunset.next_after(zone, today).unwrap();
        assert_eq!(
            tomorrow.date_naive(),
            today.date_naive().succ_opt().unwrap()
        );
    }

    #[test]
    fn an_offset_moves_the_send_and_the_days_pick_the_day_of_the_sun() {
        let zone = chrono_tz::Asia::Dhaka;
        // 2026-11-06 is a Friday; Sunday's sunrise is at 6:08 local, 00:08 UTC
        let sunday_to_thursday = sun(SunEvent::Sunrise, -10, "* * 0-4", None);
        assert_near(
            sunday_to_thursday.next_after(zone, at(2026, 11, 6, 12, 0)),
            at(2026, 11, 7, 23, 58),
        );
        // Seven hours before Sunday's sunrise is late on Saturday
        let night_before = sun(SunEvent::Sunrise, -7 * 60, "* * 0", None);
        assert_near(
            night_before.next_after(zone, at(2026, 11, 7, 12, 0)),
            at(2026, 11, 7, 17, 8),
        );
    }

    #[test]
    fn a_day_without_the_event_is_skipped() {
        let zone = chrono_tz::Europe::Oslo;
        // Polar night in Tromsø runs from late November to mid-January
        let sunrise = sun(SunEvent::Sunrise, 0, "* * *", Some(TROMSØ));
        let next = sunrise.next_after(zone, at(2026, 12, 1, 0, 0)).unwrap();
        assert!(
            next > at(2027, 1, 10, 0, 0) && next < at(2027, 1, 20, 0, 0),
            "{next}"
        );
        // And the midnight sun from late May to late July
        let sunset = sun(SunEvent::Sunset, 0, "* * *", Some(TROMSØ));
        let next = sunset.next_after(zone, at(2026, 6, 21, 0, 0)).unwrap();
        assert!(
            next > at(2026, 7, 15, 0, 0) && next < at(2026, 7, 31, 0, 0),
            "{next}"
        );
    }

    #[test]
    fn problems_are_named() {
        let zone = chrono_tz::UTC;
        let fine = sun(SunEvent::Sunset, 30, "* * *", Some(TROMSØ));
        assert_eq!(fine.problem(zone), None);
        for (wrong, problem) in [
            (
                sun(SunEvent::Sunset, 0, "0 9 * * *", Some(TROMSØ)),
                "needs its days as cron's day of the month, month and weekday fields",
            ),
            (
                sun(SunEvent::Sunset, 12 * 60 + 1, "* * *", Some(TROMSØ)),
                "can move the time by 12 hours at most",
            ),
            (
                sun(
                    SunEvent::Sunset,
                    0,
                    "* * *",
                    Some(Place {
                        latitude: 91.0,
                        longitude: 0.0,
                    }),
                ),
                "needs a latitude from -90 to 90 and a longitude from -180 to 180",
            ),
            (
                sun(SunEvent::Sunset, 0, "* * *", None),
                "needs a place, as its time zone has no city to go by",
            ),
        ] {
            assert_eq!(wrong.problem(zone), Some(problem));
        }
    }

    #[test]
    fn a_place_is_kept_to_a_tenth_of_a_degree() {
        let place = Place {
            latitude: 23.8103,
            longitude: 90.4125,
        };
        let normalized = sun(SunEvent::Sunset, 0, " *  * 0-4 ", Some(place)).normalized();
        assert_eq!(normalized.days, "* * 0-4");
        assert_eq!(
            normalized.place,
            Some(Place {
                latitude: 23.8,
                longitude: 90.4,
            })
        );
    }
}
