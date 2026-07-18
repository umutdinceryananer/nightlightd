//! The "today" tab's data: the day's solar milestones, computed from the same
//! core maths the daemon runs — crossings of the transition thresholds, not
//! hand-set times. This is the honest version of a milestone table: it derives
//! from where the sun actually is at this location on this day.

use nightlightd_core::solar::solar_elevation;
use nightlightd_core::transition::target_temperature;

/// One solar event of the day.
pub struct Milestone {
    pub name: &'static str,
    /// Local hour of the day, fractional (13.5 = 13:30).
    pub hour: f64,
    /// The colour temperature the filter lands on at that moment.
    pub kelvin: u32,
}

impl Milestone {
    pub fn hhmm(&self) -> String {
        let minutes = (self.hour * 60.0).round() as i64;
        format!("{:02}:{:02}", minutes / 60, minutes % 60)
    }
}

/// The transition thresholds, shared with core's transition curve.
const NIGHT_ELEVATION: f64 = -6.0;
const DAY_ELEVATION: f64 = 3.0;

/// Computes today's milestones for a location. `midnight` is the unix time of
/// local midnight. Missing events (polar day/night) are simply absent; the
/// list is sorted by time.
pub fn milestones(
    latitude: f64,
    longitude: f64,
    midnight: f64,
    day_temp: u32,
    night_temp: u32,
) -> Vec<Milestone> {
    // Sample the day per minute; crossings are interpolated between samples.
    let elevation_at = |hour: f64| solar_elevation(latitude, longitude, midnight + hour * 3600.0);
    let samples: Vec<f64> = (0..=1440)
        .map(|m| elevation_at(f64::from(m) / 60.0))
        .collect();
    let kelvin_of = |elevation: f64| target_temperature(elevation, day_temp, night_temp);

    let mut events: Vec<Milestone> = Vec::new();
    let mut add_crossing = |name: &'static str, threshold: f64, upward: bool| {
        for minute in 1..samples.len() {
            let (previous, current) = (samples[minute - 1], samples[minute]);
            let crossed = if upward {
                previous < threshold && current >= threshold
            } else {
                previous > threshold && current <= threshold
            };
            if crossed {
                // Linear interpolation inside the minute for a stable time.
                let fraction = (threshold - previous) / (current - previous);
                let hour = (minute as f64 - 1.0 + fraction) / 60.0;
                events.push(Milestone {
                    name,
                    hour,
                    kelvin: kelvin_of(threshold),
                });
                return;
            }
        }
    };

    add_crossing("night ends", NIGHT_ELEVATION, true);
    add_crossing("sunrise", 0.0, true);
    add_crossing("full day", DAY_ELEVATION, true);
    add_crossing("fade begins", DAY_ELEVATION, false);
    add_crossing("sunset", 0.0, false);
    add_crossing("full night", NIGHT_ELEVATION, false);

    // Solar noon: the sample with the highest sun.
    if let Some((minute, &elevation)) = samples.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1))
        && elevation > NIGHT_ELEVATION
    {
        events.push(Milestone {
            name: "solar noon",
            hour: minute as f64 / 60.0,
            kelvin: kelvin_of(elevation),
        });
    }

    events.sort_by(|a, b| a.hour.total_cmp(&b.hour));
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISTANBUL_LAT: f64 = 41.02;
    const ISTANBUL_LON: f64 = 28.97;
    /// 2021-06-21 00:00 in Istanbul (+03) as unix time.
    const SUMMER_MIDNIGHT: f64 = 1_624_222_800.0;

    #[test]
    fn a_summer_day_in_istanbul_has_all_seven_events_in_order() {
        let events = milestones(ISTANBUL_LAT, ISTANBUL_LON, SUMMER_MIDNIGHT, 6500, 2800);
        assert_eq!(events.len(), 7, "expected all seven events");
        for pair in events.windows(2) {
            assert!(pair[0].hour <= pair[1].hour, "events must be sorted");
        }
        let hour_of = |name: &str| {
            events
                .iter()
                .find(|e| e.name == name)
                .unwrap_or_else(|| panic!("missing {name}"))
                .hour
        };
        // Plausibility, not precision: summer Istanbul.
        assert!((4.0..7.0).contains(&hour_of("sunrise")), "sunrise");
        assert!((11.0..15.0).contains(&hour_of("solar noon")), "noon");
        assert!((19.0..22.0).contains(&hour_of("sunset")), "sunset");
        // The bounds land where the curve says they land.
        let night_ends = events
            .iter()
            .find(|e| e.name == "night ends")
            .expect("night ends");
        assert_eq!(night_ends.kelvin, 2800);
        let full_day = events
            .iter()
            .find(|e| e.name == "full day")
            .expect("full day");
        assert_eq!(full_day.kelvin, 6500);
    }

    #[test]
    fn hhmm_formats_fractional_hours() {
        let m = Milestone {
            name: "x",
            hour: 13.5,
            kelvin: 5000,
        };
        assert_eq!(m.hhmm(), "13:30");
    }
}
