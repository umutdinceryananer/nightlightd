//! The config file (issue #17): `~/.config/nightlightd/config.toml`.
//!
//! The daemon must run with no config at all, on sensible defaults — a program
//! that requires a config file is a program nobody uses. A missing file falls
//! back silently; a malformed one prints a clear error and then falls back too.
//! Lives in `cli`, not `core`, so `core` stays dependency-free.

use std::path::PathBuf;

use nightlightd_core::mode::Mode;
use serde::Deserialize;

/// User settings, all optional. Missing fields take the defaults below.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Daytime temperature (kelvin).
    pub day_temp: u32,
    /// Night temperature (kelvin).
    pub night_temp: u32,
    /// Manual latitude in degrees; pins the location instead of the timezone.
    pub latitude: Option<f64>,
    /// Manual longitude in degrees; used together with `latitude`.
    pub longitude: Option<f64>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            day_temp: 6500,
            night_temp: 4500,
            latitude: None,
            longitude: None,
        }
    }
}

impl Config {
    /// The operating mode the config implies: a manual location when both
    /// coordinates are given, otherwise automatic (derived from the timezone).
    pub fn mode(&self) -> Mode {
        match (self.latitude, self.longitude) {
            (Some(lat), Some(lon)) => Mode::ManualLocation { lat, lon },
            _ => Mode::Automatic,
        }
    }
}

/// Loads the config. A missing file yields defaults silently; a malformed file
/// prints a clear error and yields defaults. Never fails.
pub fn load() -> Config {
    let Some(path) = config_path() else {
        return Config::default();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Config::default();
    };
    match toml::from_str(&text) {
        Ok(config) => config,
        Err(error) => {
            tracing::warn!("{}: {error}; using defaults", path.display());
            Config::default()
        }
    }
}

/// `$XDG_CONFIG_HOME/nightlightd/config.toml`, or `~/.config/...` as a fallback.
fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("nightlightd").join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_is_all_defaults() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn partial_config_keeps_defaults_for_the_rest() {
        // A user's whole config might be just this one line.
        let config: Config = toml::from_str("night_temp = 2800").unwrap();
        assert_eq!(config.night_temp, 2800);
        assert_eq!(config.day_temp, 6500);
    }

    #[test]
    fn manual_coordinates_select_manual_location() {
        let text = "latitude = 39.93\nlongitude = 32.85\n";
        let config: Config = toml::from_str(text).unwrap();
        assert_eq!(
            config.mode(),
            Mode::ManualLocation {
                lat: 39.93,
                lon: 32.85
            }
        );
    }

    #[test]
    fn no_coordinates_means_automatic() {
        assert_eq!(Config::default().mode(), Mode::Automatic);
    }

    #[test]
    fn a_lone_coordinate_stays_automatic() {
        let config: Config = toml::from_str("latitude = 39.93").unwrap();
        assert_eq!(config.mode(), Mode::Automatic);
    }

    #[test]
    fn malformed_config_is_rejected_by_the_parser() {
        // `load()` turns this into a warning and defaults; here we confirm the
        // parse itself fails.
        assert!(toml::from_str::<Config>("night_temp = \"warm\"").is_err());
    }
}
