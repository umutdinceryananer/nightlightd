//! Deriving an approximate location from the system timezone (issue #7).
//!
//! This is the tool's headline trick: no network, no Geoclue, no permission
//! prompt, no question asked. It reads the timezone the system already knows
//! (`Europe/Istanbul`) and looks its representative coordinate up in the IANA
//! `zone.tab` file that ships on every Linux install. An error of a degree or
//! so shifts sunset by a few minutes — nobody notices.
//!
//! Everything degrades to [`None`] rather than panicking when a file is
//! missing or malformed.

// Ported from nightlightd's own upstream contribution to gammastep,
// `src/location-timezone.c`.
//
//   Copyright (c) 2026  Umut Dincer Yananer <umutdncr@gmail.com>
//   SPDX-License-Identifier: GPL-3.0-or-later

/// Parses one signed, packed ISO 6709 angle such as `+4101` or `-0740023`.
///
/// `deg_digits` is 2 for a latitude (`DDMM[SS]`) and 3 for a longitude
/// (`DDDMM[SS]`); minutes and seconds are base 60. Returns [`None`] if the
/// string is malformed.
fn parse_angle(s: &str, deg_digits: usize) -> Option<f64> {
    let sign = match s.as_bytes().first()? {
        b'+' => 1.0,
        b'-' => -1.0,
        _ => return None,
    };

    let digits = &s[1..];
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }

    // Either degrees + minutes, or degrees + minutes + seconds.
    let has_seconds = match digits.len() {
        n if n == deg_digits + 2 => false,
        n if n == deg_digits + 4 => true,
        _ => return None,
    };

    let deg: u32 = digits[..deg_digits].parse().ok()?;
    let min: u32 = digits[deg_digits..deg_digits + 2].parse().ok()?;
    let sec: u32 = if has_seconds {
        digits[deg_digits + 2..deg_digits + 4].parse().ok()?
    } else {
        0
    };

    Some(sign * (f64::from(deg) + f64::from(min) / 60.0 + f64::from(sec) / 3600.0))
}

/// Parses a packed ISO 6709 coordinate pair such as `+4101+02858` (latitude
/// then longitude, no separator) into `(lat, lon)` in degrees.
fn parse_coordinates(s: &str) -> Option<(f64, f64)> {
    if !matches!(s.as_bytes().first(), Some(b'+' | b'-')) {
        return None;
    }

    // The longitude begins at the second sign in the string.
    let split = s[1..].find(['+', '-'])? + 1;
    let lat = parse_angle(&s[..split], 2)?;
    let lon = parse_angle(&s[split..], 3)?;
    Some((lat, lon))
}

/// Looks up an IANA zone name in the text of a `zone.tab`-style file.
///
/// Lines are tab-separated (country codes, coordinate, TZ name, optional
/// comment); lines beginning with `#` are comments. Malformed lines are
/// skipped rather than aborting the search.
fn coordinate_from_zone_tab(contents: &str, zone: &str) -> Option<(f64, f64)> {
    for line in contents.lines() {
        if line.starts_with('#') {
            continue;
        }

        let mut fields = line.split('\t');
        let (Some(_codes), Some(coord), Some(name)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };

        if name == zone {
            return parse_coordinates(coord);
        }
    }
    None
}

/// Extracts an IANA zone name from a `TZ` environment value, e.g.
/// `Europe/Istanbul`, `:Europe/Istanbul`, or `:/usr/share/zoneinfo/Europe/Istanbul`.
/// Returns [`None`] if the result is empty.
fn zone_from_tz_value(value: &str) -> Option<&str> {
    // POSIX permits a leading colon.
    let value = value.strip_prefix(':').unwrap_or(value);
    // If it is a full path, keep only what follows "zoneinfo/".
    let name = value
        .split_once("zoneinfo/")
        .map_or(value, |(_, after)| after);
    (!name.is_empty()).then_some(name)
}

/// Determines the system's IANA zone name: `TZ` first (handy for testing),
/// then the `/etc/localtime` symlink, then `/etc/timezone` as plain text.
fn zone_name() -> Option<String> {
    zone_from_env()
        .or_else(zone_from_localtime)
        .or_else(zone_from_etc_timezone)
}

/// Zone name from the `TZ` environment variable, if set and non-empty.
fn zone_from_env() -> Option<String> {
    let tz = std::env::var("TZ").ok()?;
    zone_from_tz_value(&tz).map(String::from)
}

/// Zone name from the `/etc/localtime` symlink into the zoneinfo database.
fn zone_from_localtime() -> Option<String> {
    let target = std::fs::read_link("/etc/localtime").ok()?;
    let target = target.to_string_lossy();
    let (_, name) = target.split_once("zoneinfo/")?;
    (!name.is_empty()).then(|| name.to_owned())
}

/// Zone name from `/etc/timezone`, stored as plain text.
fn zone_from_etc_timezone() -> Option<String> {
    let contents = std::fs::read_to_string("/etc/timezone").ok()?;
    let name = contents.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// Reads the zoneinfo coordinate table and looks up `zone`. Tries `zone.tab`,
/// then `zone1970.tab`, using whichever opens first.
fn lookup_zone(zone: &str) -> Option<(f64, f64)> {
    const PATHS: [&str; 2] = [
        "/usr/share/zoneinfo/zone.tab",
        "/usr/share/zoneinfo/zone1970.tab",
    ];

    for path in PATHS {
        if let Ok(contents) = std::fs::read_to_string(path) {
            return coordinate_from_zone_tab(&contents, zone);
        }
    }
    None
}

/// The system's approximate location as `(latitude, longitude)` in degrees,
/// derived from its timezone. Returns [`None`] if the timezone or the zoneinfo
/// database cannot be read.
pub fn location_from_timezone() -> Option<(f64, f64)> {
    let zone = zone_name()?;
    lookup_zone(&zone)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn parses_latitude_without_seconds() {
        // +4101 -> 41 deg 01 min.
        assert!(approx(parse_angle("+4101", 2).unwrap(), 41.016_667));
    }

    #[test]
    fn parses_longitude_with_seconds() {
        // -0740023 -> -(74 deg 00 min 23 sec).
        assert!(approx(parse_angle("-0740023", 3).unwrap(), -74.006_389));
    }

    #[test]
    fn rejects_malformed_angles() {
        assert_eq!(parse_angle("4101", 2), None); // no sign
        assert_eq!(parse_angle("+41", 2), None); // too short
        assert_eq!(parse_angle("+41x1", 2), None); // non-digit
        assert_eq!(parse_angle("+", 2), None); // empty body
    }

    #[test]
    fn parses_a_coordinate_pair() {
        let (lat, lon) = parse_coordinates("+4101+02858").unwrap();
        assert!(approx(lat, 41.016_667));
        assert!(approx(lon, 28.966_667));
    }

    #[test]
    fn parses_a_coordinate_pair_with_seconds_and_negative_lon() {
        // America/New_York: +404251-0740023.
        let (lat, lon) = parse_coordinates("+404251-0740023").unwrap();
        assert!(approx(lat, 40.714_167));
        assert!(approx(lon, -74.006_389));
    }

    #[test]
    fn rejects_malformed_coordinate_pairs() {
        assert_eq!(parse_coordinates("+4101"), None); // no second sign
        assert_eq!(parse_coordinates(""), None); // empty
        assert_eq!(parse_coordinates("4101+02858"), None); // no leading sign
    }

    const SAMPLE: &str = "\
# tab-separated: codes, coordinate, TZ name, optional comment
AD\t+4230+00131\tEurope/Andorra
TR\t+4101+02858\tEurope/Istanbul
US\t+404251-0740023\tAmerica/New_York\tEastern (most areas)
this line is malformed and has no tabs
";

    #[test]
    fn looks_up_zones_in_a_table() {
        let (lat, lon) = coordinate_from_zone_tab(SAMPLE, "Europe/Istanbul").unwrap();
        assert!(approx(lat, 41.016_667) && approx(lon, 28.966_667));

        let (lat, lon) = coordinate_from_zone_tab(SAMPLE, "America/New_York").unwrap();
        assert!(approx(lat, 40.714_167) && approx(lon, -74.006_389));
    }

    #[test]
    fn missing_zone_and_bad_lines_yield_none_not_panic() {
        assert_eq!(coordinate_from_zone_tab(SAMPLE, "Missing/Zone"), None);
    }

    #[test]
    fn extracts_zone_name_from_tz_values() {
        assert_eq!(
            zone_from_tz_value("Europe/Istanbul"),
            Some("Europe/Istanbul")
        );
        assert_eq!(
            zone_from_tz_value(":/usr/share/zoneinfo/Europe/Istanbul"),
            Some("Europe/Istanbul")
        );
        assert_eq!(zone_from_tz_value(""), None);
        assert_eq!(zone_from_tz_value(":"), None);
    }

    #[test]
    fn real_zone_tab_is_plausible_if_present() {
        // Uses the system's zone.tab when available. On an environment without
        // tzdata it is simply absent, so we only assert when a coordinate came
        // back at all.
        if let Some((lat, lon)) = lookup_zone("Europe/Istanbul") {
            assert!((40.0..42.0).contains(&lat), "lat {lat}");
            assert!((28.0..30.0).contains(&lon), "lon {lon}");
        }
    }

    #[test]
    fn location_from_timezone_never_panics() {
        // Whatever this machine's timezone is, we get Some(..) or None.
        let _ = location_from_timezone();
    }
}
