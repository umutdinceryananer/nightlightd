//! The transition curve: sun angle to target temperature (issue #8).
//!
//! Snapping between day and night as the sun crosses the horizon looks cheap.
//! Instead the target temperature eases across a band of solar elevations,
//! matching redshift's behaviour: at or above +3 degrees it is full daytime,
//! at or below -6 degrees full night, and it interpolates linearly between.
//! The brightness factor (GitHub #2) rides exactly the same band, easing
//! with the sun just as redshift and gammastep ease theirs — the band is
//! theirs to begin with.

/// Elevation (degrees) at or above which it is full daytime.
const DAY_ELEVATION: f64 = 3.0;
/// Elevation (degrees) at or below which it is full night.
const NIGHT_ELEVATION: f64 = -6.0;

/// The target colour temperature for a given solar `elevation` (degrees),
/// easing between `night_temp` and `day_temp`.
///
/// Above +3 degrees returns `day_temp` exactly; below -6 degrees returns
/// `night_temp` exactly; between the two it interpolates linearly, so the
/// result rises monotonically as the sun climbs (given `day_temp >=
/// night_temp`).
pub fn target_temperature(elevation: f64, day_temp: u32, night_temp: u32) -> u32 {
    let night = f64::from(night_temp);
    let day = f64::from(day_temp);
    (night + daylight_alpha(elevation) * (day - night)).round() as u32
}

/// The brightness factor for a given solar `elevation`, riding the same
/// transition band as the temperature (GitHub #2): `day` at full daytime,
/// `night` at full night, a linear ease between. Pure interpolation; the
/// clamping to a sane range happens where the values enter (config) and
/// where they are spent ([`build_ramp`](crate::color::build_ramp)).
pub fn target_brightness(elevation: f64, day: f64, night: f64) -> f64 {
    night + daylight_alpha(elevation) * (day - night)
}

/// Where the sun sits in the transition band: 1.0 at or above full daytime,
/// 0.0 at or below full night, easing linearly between. The one place the
/// band is defined, so everything that follows the sun follows it together.
fn daylight_alpha(elevation: f64) -> f64 {
    let span = DAY_ELEVATION - NIGHT_ELEVATION;
    ((elevation - NIGHT_ELEVATION) / span).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u32 = 6500;
    const NIGHT: u32 = 3500;

    #[test]
    fn full_day_at_and_above_threshold() {
        assert_eq!(target_temperature(3.0, DAY, NIGHT), DAY); // exact endpoint
        assert_eq!(target_temperature(45.0, DAY, NIGHT), DAY);
    }

    #[test]
    fn full_night_at_and_below_threshold() {
        assert_eq!(target_temperature(-6.0, DAY, NIGHT), NIGHT); // exact endpoint
        assert_eq!(target_temperature(-30.0, DAY, NIGHT), NIGHT);
    }

    #[test]
    fn midpoint_is_halfway_between_day_and_night() {
        // Halfway between -6 and +3 is -1.5.
        assert_eq!(target_temperature(-1.5, DAY, NIGHT), (DAY + NIGHT) / 2);
    }

    #[test]
    fn rises_monotonically_with_elevation() {
        let mut previous = 0;
        let mut elevation = -10.0;
        while elevation <= 6.0 {
            let temp = target_temperature(elevation, DAY, NIGHT);
            assert!(
                temp >= previous,
                "temperature dropped at {elevation} degrees"
            );
            previous = temp;
            elevation += 0.5;
        }
    }

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn brightness_hits_the_endpoints_exactly() {
        assert!(approx(target_brightness(3.0, 1.0, 0.9), 1.0));
        assert!(approx(target_brightness(45.0, 1.0, 0.9), 1.0));
        assert!(approx(target_brightness(-6.0, 1.0, 0.9), 0.9));
        assert!(approx(target_brightness(-30.0, 1.0, 0.9), 0.9));
    }

    #[test]
    fn brightness_midpoint_is_halfway() {
        assert!(approx(target_brightness(-1.5, 1.0, 0.9), 0.95));
    }

    /// The untouched-user guarantee again: equal day and night factors mean
    /// the sun moves nothing.
    #[test]
    fn identity_brightness_never_moves() {
        let mut elevation = -30.0;
        while elevation <= 45.0 {
            assert!(approx(target_brightness(elevation, 1.0, 1.0), 1.0));
            elevation += 1.5;
        }
    }

    /// Brightness and temperature share one band, by construction and by
    /// test: wherever the temperature sits between its bounds, brightness
    /// sits at the same fraction between its own.
    #[test]
    fn brightness_rides_the_temperature_curve() {
        let mut elevation = -10.0;
        while elevation <= 6.0 {
            let temp = target_temperature(elevation, DAY, NIGHT);
            let temp_alpha = f64::from(temp - NIGHT) / f64::from(DAY - NIGHT);
            let bright = target_brightness(elevation, 1.0, 0.5);
            let bright_alpha = (bright - 0.5) / 0.5;
            assert!(
                (temp_alpha - bright_alpha).abs() < 1e-3,
                "curves diverged at {elevation} degrees"
            );
            elevation += 0.25;
        }
    }
}
