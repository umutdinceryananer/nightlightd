//! The day's solar milestones: the moments the sun crosses the transition
//! band's bounds and the horizon, with the temperature the screen lands on at
//! each. Not hand-set times — every one of them is derived from where the sun
//! actually is at this location on this day.
//!
//! It lives here rather than in an interface because two of them want it and
//! neither owns it, and because it is what this crate is for: numbers, no
//! display, testable without a screen.

use crate::solar::solar_elevation;
use crate::transition::{Band, target_temperature};

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

/// Solar noon's floor: "is there any daylight today" is astronomy, not
/// configuration, so it stays the default night bound. The crossings
/// themselves use the configured band (#39), already `sane()`d by the
/// caller so the schedule shows what the daemon actually applies.
const NIGHT_ELEVATION: f64 = -6.0;

/// Computes today's milestones for a location. `midnight` is the unix time of
/// local midnight. Missing events (polar day/night) are simply absent; the
/// list is sorted by time.
pub fn milestones(
    latitude: f64,
    longitude: f64,
    midnight: f64,
    band: Band,
    day_temp: u32,
    night_temp: u32,
) -> Vec<Milestone> {
    // Sample the day per minute; crossings are interpolated between samples.
    let elevation_at = |hour: f64| solar_elevation(latitude, longitude, midnight + hour * 3600.0);
    let samples: Vec<f64> = (0..=1440)
        .map(|m| elevation_at(f64::from(m) / 60.0))
        .collect();
    let kelvin_of = |elevation: f64| target_temperature(elevation, band, day_temp, night_temp);

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

    add_crossing("night ends", band.night_elevation, true);
    add_crossing("sunrise", 0.0, true);
    add_crossing("full day", band.day_elevation, true);
    add_crossing("fade begins", band.day_elevation, false);
    add_crossing("sunset", 0.0, false);
    add_crossing("full night", band.night_elevation, false);

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

/// The hour of a named milestone, if the day has one.
pub fn hour_of(events: &[Milestone], name: &str) -> Option<f64> {
    events.iter().find(|e| e.name == name).map(|e| e.hour)
}

/// How long the sun spends above the horizon, in hours.
///
/// [`None`] on a day it never crosses at all. A polar day and a polar night
/// both look like "no sunrise and no sunset" from here, and neither has a
/// length this function could honestly return — the caller knows which one it
/// is looking at from the elevation, and this does not pretend to.
pub fn day_length(events: &[Milestone]) -> Option<f64> {
    Some(hour_of(events, "sunset")? - hour_of(events, "sunrise")?)
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
        let events = milestones(
            ISTANBUL_LAT,
            ISTANBUL_LON,
            SUMMER_MIDNIGHT,
            Band::default(),
            6500,
            2800,
        );
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

    /// The longest day of the year in Istanbul is a shade under fifteen
    /// hours, the shortest a shade over nine. Two solstices six months apart
    /// are the cheapest way to catch a day length that is inverted, off by a
    /// timezone, or measuring the wrong pair of crossings.
    ///
    /// A few minutes short of what an almanac prints, and correctly so: this
    /// crosses on the sun's geometric centre, where published tables add
    /// about -0.833° for refraction and the upper limb. Widening the bounds
    /// to swallow that difference would only hide the day it stops being the
    /// explanation.
    #[test]
    fn the_day_is_longest_at_midsummer_and_shortest_at_midwinter() {
        let day_at = |midnight: f64| {
            day_length(&milestones(
                ISTANBUL_LAT,
                ISTANBUL_LON,
                midnight,
                Band::default(),
                6500,
                2800,
            ))
        };
        // 2021-12-21 00:00 in Istanbul (+03) as unix time.
        const WINTER_MIDNIGHT: f64 = 1_640_034_000.0;
        let summer = day_at(SUMMER_MIDNIGHT).expect("midsummer has a sunrise");
        let winter = day_at(WINTER_MIDNIGHT).expect("midwinter has a sunrise");
        assert!((14.8..15.1).contains(&summer), "midsummer was {summer}h");
        assert!((8.9..9.2).contains(&winter), "midwinter was {winter}h");
    }

    /// Above the Arctic circle in June the sun never sets, so there is no
    /// length to report — and the honest answer is nothing, not zero and not
    /// twenty-four.
    #[test]
    fn a_polar_day_has_no_length_to_give() {
        let events = milestones(78.22, 15.65, SUMMER_MIDNIGHT, Band::default(), 6500, 2800);
        assert_eq!(day_length(&events), None);
        assert_eq!(hour_of(&events, "sunrise"), None);
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
