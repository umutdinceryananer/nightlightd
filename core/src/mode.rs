//! The three operating modes, tying the pieces together (issue #9).
//!
//! Everything automatic needs an escape hatch, so `core` supports three ways to
//! decide the target temperature:
//!
//! * [`Mode::Automatic`] — locate via the timezone, then follow the sun.
//! * [`Mode::ManualLocation`] — the user supplies latitude and longitude.
//! * [`Mode::Fixed`] — ignore the sun and pin a temperature.
//!
//! [`resolve_temperature`] is the single entry point that turns a mode and a
//! moment in time into a colour temperature.

use crate::location::location_from_timezone;
use crate::solar::solar_elevation;
use crate::transition::target_temperature;

/// How the target temperature should be decided.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// Derive the location from the system timezone, then follow the sun.
    Automatic,
    /// Follow the sun at a location the user supplied, in degrees.
    ManualLocation { lat: f64, lon: f64 },
    /// Ignore the sun and hold a fixed colour temperature (kelvin).
    Fixed(u32),
}

/// Resolves the target colour temperature for `mode` at `unix_time` (seconds
/// since the epoch, UTC).
///
/// `day_temp` and `night_temp` bound the automatic and manual modes. If
/// automatic mode cannot determine a location, it degrades quietly to
/// `day_temp` — a neutral screen — rather than failing.
pub fn resolve_temperature(mode: Mode, unix_time: f64, day_temp: u32, night_temp: u32) -> u32 {
    match mode {
        Mode::Fixed(temp) => temp,
        Mode::ManualLocation { lat, lon } => {
            temperature_at(lat, lon, unix_time, day_temp, night_temp)
        }
        Mode::Automatic => match location_from_timezone() {
            Some((lat, lon)) => temperature_at(lat, lon, unix_time, day_temp, night_temp),
            None => day_temp,
        },
    }
}

/// Follows the sun at a known location: solar elevation, then the transition
/// curve.
fn temperature_at(lat: f64, lon: f64, unix_time: f64, day_temp: u32, night_temp: u32) -> u32 {
    let elevation = solar_elevation(lat, lon, unix_time);
    target_temperature(elevation, day_temp, night_temp)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANKARA_LAT: f64 = 39.93;
    const ANKARA_LON: f64 = 32.85;
    const SUMMER_NOON: f64 = 1_624_266_000.0; // 2021-06-21 12:00 Ankara
    const SUMMER_MIDNIGHT: f64 = 1_624_309_200.0; // 2021-06-22 00:00 Ankara
    const DAY: u32 = 6500;
    const NIGHT: u32 = 3500;

    #[test]
    fn fixed_mode_ignores_the_sun() {
        assert_eq!(
            resolve_temperature(Mode::Fixed(2800), SUMMER_NOON, DAY, NIGHT),
            2800
        );
        assert_eq!(
            resolve_temperature(Mode::Fixed(2800), SUMMER_MIDNIGHT, DAY, NIGHT),
            2800
        );
    }

    #[test]
    fn manual_location_neutral_by_day_warm_at_night() {
        let mode = Mode::ManualLocation {
            lat: ANKARA_LAT,
            lon: ANKARA_LON,
        };
        // Sun high at noon -> full day_temp; below the horizon at night ->
        // full night_temp.
        assert_eq!(resolve_temperature(mode, SUMMER_NOON, DAY, NIGHT), DAY);
        assert_eq!(
            resolve_temperature(mode, SUMMER_MIDNIGHT, DAY, NIGHT),
            NIGHT
        );
    }

    #[test]
    fn automatic_mode_stays_within_bounds_and_never_panics() {
        // Whether or not this machine's location resolves, the result is a
        // temperature within the configured band.
        let temp = resolve_temperature(Mode::Automatic, SUMMER_NOON, DAY, NIGHT);
        assert!((NIGHT..=DAY).contains(&temp), "got {temp}");
    }
}
