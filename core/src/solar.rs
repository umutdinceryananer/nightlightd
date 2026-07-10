//! Solar elevation — the sun's angle above the horizon (issue #6).
//!
//! This answers "has the sun set?" with an angle rather than a clock time,
//! which handles seasons and latitudes for free. [`solar_elevation`] returns
//! degrees: positive when the sun is up, negative once it is below the horizon.

// Ported from gammastep's `src/solar.c`, which in turn ports the JavaScript
// solar-position algorithm published by the U.S. National Oceanic &
// Atmospheric Administration, based on equations from Jean Meeus,
// "Astronomical Algorithms".
//
//   Copyright (c) 2010  Jon Lund Steffensen <jonlst@gmail.com>
//   SPDX-License-Identifier: GPL-3.0-or-later
//
// nightlightd is likewise GPL-3.0-or-later, so the port is licence-compatible.

use std::f64::consts::PI;

/// Degrees to radians.
fn rad(degrees: f64) -> f64 {
    degrees * (PI / 180.0)
}

/// Radians to degrees.
fn deg(radians: f64) -> f64 {
    radians * (180.0 / PI)
}

/// Julian day from a Unix timestamp (seconds since the epoch).
fn jd_from_epoch(t: f64) -> f64 {
    t / 86400.0 + 2440587.5
}

/// Julian centuries since J2000.0, from a Julian day.
fn jcent_from_jd(jd: f64) -> f64 {
    (jd - 2451545.0) / 36525.0
}

/// Julian day from Julian centuries since J2000.0.
fn jd_from_jcent(t: f64) -> f64 {
    36525.0 * t + 2451545.0
}

/// Geometric mean longitude of the sun, in radians. `t` is Julian centuries.
fn sun_geom_mean_lon(t: f64) -> f64 {
    rad((280.46646 + t * (36000.76983 + t * 0.0003032)) % 360.0)
}

/// Geometric mean anomaly of the sun, in radians.
fn sun_geom_mean_anomaly(t: f64) -> f64 {
    rad(357.52911 + t * (35999.05029 - t * 0.0001537))
}

/// Eccentricity of Earth's orbit (unitless).
fn earth_orbit_eccentricity(t: f64) -> f64 {
    0.016708634 - t * (0.000042037 + t * 0.0000001267)
}

/// Equation of the centre of the sun, in radians (first three terms).
fn sun_equation_of_center(t: f64) -> f64 {
    let m = sun_geom_mean_anomaly(t);
    let c = m.sin() * (1.914602 - t * (0.004817 + 0.000014 * t))
        + (2.0 * m).sin() * (0.019993 - 0.000101 * t)
        + (3.0 * m).sin() * 0.000289;
    rad(c)
}

/// True longitude of the sun, in radians.
fn sun_true_lon(t: f64) -> f64 {
    sun_geom_mean_lon(t) + sun_equation_of_center(t)
}

/// Apparent longitude of the sun, in radians.
fn sun_apparent_lon(t: f64) -> f64 {
    let o = sun_true_lon(t);
    rad(deg(o) - 0.00569 - 0.00478 * rad(125.04 - 1934.136 * t).sin())
}

/// Mean obliquity of the ecliptic, in radians.
fn mean_ecliptic_obliquity(t: f64) -> f64 {
    let sec = 21.448 - t * (46.815 + t * (0.00059 - t * 0.001813));
    rad(23.0 + (26.0 + sec / 60.0) / 60.0)
}

/// Obliquity of the ecliptic corrected for nutation, in radians.
fn obliquity_corr(t: f64) -> f64 {
    let e_0 = mean_ecliptic_obliquity(t);
    let omega = 125.04 - t * 1934.136;
    rad(deg(e_0) + 0.00256 * rad(omega).cos())
}

/// Declination of the sun, in radians.
fn solar_declination(t: f64) -> f64 {
    let e = obliquity_corr(t);
    let lambda = sun_apparent_lon(t);
    (e.sin() * lambda.sin()).asin()
}

/// Difference between true solar time and mean solar time, in minutes.
fn equation_of_time(t: f64) -> f64 {
    let epsilon = obliquity_corr(t);
    let l_0 = sun_geom_mean_lon(t);
    let e = earth_orbit_eccentricity(t);
    let m = sun_geom_mean_anomaly(t);
    let y = (epsilon / 2.0).tan().powi(2);

    let eq_time = y * (2.0 * l_0).sin() - 2.0 * e * m.sin()
        + 4.0 * e * y * m.sin() * (2.0 * l_0).cos()
        - 0.5 * y * y * (4.0 * l_0).sin()
        - 1.25 * e * e * (2.0 * m).sin();
    4.0 * deg(eq_time)
}

/// Angular elevation, in radians, for the given hour angle.
fn elevation_from_hour_angle(lat: f64, decl: f64, ha: f64) -> f64 {
    let lat = rad(lat);
    (ha.cos() * lat.cos() * decl.cos() + lat.sin() * decl.sin()).asin()
}

/// Solar elevation, in radians, at Julian centuries `t`.
fn solar_elevation_from_time(t: f64, lat: f64, lon: f64) -> f64 {
    let jd = jd_from_jcent(t);
    let offset = (jd - jd.round() - 0.5) * 1440.0;
    let eq_time = equation_of_time(t);
    let ha = rad((720.0 - offset - eq_time) / 4.0 - lon);
    let decl = solar_declination(t);
    elevation_from_hour_angle(lat, decl, ha)
}

/// The sun's angular elevation above the horizon, in degrees, at the given
/// location and time.
///
/// `lat` and `lon` are in degrees (north and east positive). `unix_time` is
/// seconds since the Unix epoch in UTC. A positive result means the sun is
/// above the horizon; negative means it has set.
pub fn solar_elevation(lat: f64, lon: f64, unix_time: f64) -> f64 {
    let jd = jd_from_epoch(unix_time);
    deg(solar_elevation_from_time(jcent_from_jd(jd), lat, lon))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ankara: 39.93 N, 32.85 E, timezone UTC+3 (no DST since 2016).
    const ANKARA_LAT: f64 = 39.93;
    const ANKARA_LON: f64 = 32.85;

    // Unix timestamps (UTC). Local noon is 09:00 UTC (= 12:00 UTC+3).
    const JUN_21_NOON: f64 = 1_624_266_000.0; // 2021-06-21 12:00 local
    const DEC_21_NOON: f64 = 1_640_077_200.0; // 2021-12-21 12:00 local
    const JUN_22_MIDNIGHT: f64 = 1_624_309_200.0; // 2021-06-22 00:00 local

    #[test]
    fn ankara_high_at_summer_noon() {
        // Theoretical max is 90 - 39.93 + 23.44 = 73.5; ISSUES.md says ~72.
        let elev = solar_elevation(ANKARA_LAT, ANKARA_LON, JUN_21_NOON);
        assert!((69.0..=75.0).contains(&elev), "got {elev}");
    }

    #[test]
    fn ankara_low_at_winter_noon() {
        // Theoretical max is 90 - 39.93 - 23.44 = 26.6; ISSUES.md says ~26.
        let elev = solar_elevation(ANKARA_LAT, ANKARA_LON, DEC_21_NOON);
        assert!((23.0..=29.0).contains(&elev), "got {elev}");
    }

    #[test]
    fn summer_noon_higher_than_winter_noon() {
        let summer = solar_elevation(ANKARA_LAT, ANKARA_LON, JUN_21_NOON);
        let winter = solar_elevation(ANKARA_LAT, ANKARA_LON, DEC_21_NOON);
        assert!(summer > winter);
    }

    #[test]
    fn sun_below_horizon_at_night() {
        let elev = solar_elevation(ANKARA_LAT, ANKARA_LON, JUN_22_MIDNIGHT);
        assert!(elev < 0.0, "got {elev}");
    }

    #[test]
    fn elevation_is_finite_across_a_day() {
        for hour in 0..24 {
            let t = JUN_21_NOON + f64::from(hour) * 3600.0;
            assert!(solar_elevation(ANKARA_LAT, ANKARA_LON, t).is_finite());
        }
    }
}
