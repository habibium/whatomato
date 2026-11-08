//! The main city of each time zone, for a sun schedule that names no place of its own. From tzdata,
//! by `just zone-cities`.

use std::{collections::HashMap, sync::LazyLock};

use chrono_tz::Tz;

use super::sun::Place;

pub struct City {
    pub name: &'static str,
    pub place: Place,
}

static CITIES: LazyLock<HashMap<&str, City>> = LazyLock::new(|| {
    TABLE
        .lines()
        .filter(|line| !line.starts_with('#'))
        .filter_map(row)
        .collect()
});

const TABLE: &str = include_str!("zone-cities.tsv");

/// A line of the table: zone, latitude, longitude, city
fn row(line: &'static str) -> Option<(&'static str, City)> {
    let mut fields = line.split('\t');
    let zone = fields.next()?;
    let latitude = fields.next()?.parse().ok()?;
    let longitude = fields.next()?.parse().ok()?;
    let name = fields.next()?;
    Some((
        zone,
        City {
            name,
            place: Place {
                latitude,
                longitude,
            },
        },
    ))
}

/// `None` for a zone with no city, like UTC
#[must_use]
pub fn city(zone: Tz) -> Option<&'static City> {
    CITIES.get(zone.name())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_line_of_the_table_reads() {
        let lines = TABLE.lines().filter(|line| !line.starts_with('#')).count();
        assert_eq!(CITIES.len(), lines);
    }

    #[test]
    fn a_zone_and_its_old_names_share_a_city() {
        let dhaka = city(chrono_tz::Asia::Dhaka).unwrap();
        assert_eq!(dhaka.name, "Dhaka");
        assert_eq!(
            (dhaka.place.latitude, dhaka.place.longitude),
            (23.7167, 90.4167)
        );
        assert_eq!(city(chrono_tz::Asia::Calcutta).unwrap().name, "Kolkata");
        assert!(city(chrono_tz::UTC).is_none());
    }
}
