//! The transition curve: sun angle to target temperature (issue #8).
//!
//! Snapping between day and night as the sun crosses the horizon looks cheap.
//! Instead the target temperature eases across a band of solar elevations,
//! matching redshift's behaviour: at or above +3 degrees it is full daytime,
//! at or below -6 degrees full night, and it interpolates linearly between.

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
    let span = DAY_ELEVATION - NIGHT_ELEVATION;
    let alpha = ((elevation - NIGHT_ELEVATION) / span).clamp(0.0, 1.0);

    let night = f64::from(night_temp);
    let day = f64::from(day_temp);
    (night + alpha * (day - night)).round() as u32
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
}
