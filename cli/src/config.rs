//! The config file (issue #17): `~/.config/nightlightd/config.toml`.
//!
//! The daemon must run with no config at all, on sensible defaults — a program
//! that requires a config file is a program nobody uses. A missing file falls
//! back silently; a malformed one prints a clear error and then falls back too.
//! Lives in `cli`, not `core`, so `core` stays dependency-free.

use std::path::PathBuf;

use nightlightd_core::mode::Mode;
use nightlightd_core::transition::{Band, Bands};
use serde::{Deserialize, Serialize};

/// User settings, all optional. Missing fields take the defaults below.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    /// Daytime temperature (kelvin).
    pub day_temp: u32,
    /// Night temperature (kelvin).
    pub night_temp: u32,
    /// Gamma exponent bending the ramp curve, constant across the day
    /// (GitHub #2). 1.0 is the identity. Kept verbatim here and clamped
    /// where it is spent, so a save never rewrites a hand-written value.
    pub gamma: f64,
    /// Brightness factor in full daylight; 1.0 leaves the screen at full
    /// output.
    pub day_brightness: f64,
    /// Brightness factor at full night. The screen eases between the two
    /// factors on the same solar curve as the temperature.
    pub night_brightness: f64,
    /// Whether a target change eases onto the screen over a couple of
    /// seconds (#44). Off means every change lands in one write, the
    /// behaviour redshift's own `fade=0` selects.
    pub fade: bool,
    /// Solar elevation (degrees) at or above which it is full daytime
    /// (#39). Kept verbatim; a nonsensical pair degrades to the default
    /// where it is spent, in core. This pair is the evening's, and the
    /// morning's too unless a `dawn_` value below says otherwise (#47).
    pub day_elevation: f64,
    /// Solar elevation at or below which it is full night. Lowering it
    /// lands full night deeper into dusk — redshift's `elevation-low`.
    pub night_elevation: f64,
    /// The morning's full-day bound, when it differs from the evening's
    /// (#47). Absent means the pair above holds at both ends of the day,
    /// which is what every file written before #47 says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dawn_day_elevation: Option<f64>,
    /// The morning's full-night bound, likewise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dawn_night_elevation: Option<f64>,
    /// Manual latitude in degrees; pins the location instead of the timezone.
    /// Omitted from the written file when absent (TOML has no null).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    /// Manual longitude in degrees; used together with `latitude`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f64>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            day_temp: 6500,
            night_temp: 4500,
            gamma: 1.0,
            day_brightness: 1.0,
            night_brightness: 1.0,
            fade: true,
            day_elevation: 3.0,
            night_elevation: -6.0,
            dawn_day_elevation: None,
            dawn_night_elevation: None,
            latitude: None,
            longitude: None,
        }
    }
}

/// The four elevation fields as a config writes them: the evening's pair in
/// full, and a morning value only where it differs, so a file nobody split
/// stays the two lines it always was. The inverse of [`Config::bands`].
pub fn band_fields(bands: Bands) -> (f64, f64, Option<f64>, Option<f64>) {
    let Bands { dawn, dusk } = bands;
    let apart = |morning: f64, evening: f64| (morning != evening).then_some(morning);
    (
        dusk.day_elevation,
        dusk.night_elevation,
        apart(dawn.day_elevation, dusk.day_elevation),
        apart(dawn.night_elevation, dusk.night_elevation),
    )
}

impl Config {
    /// The transition bands the file describes (#47): the evening's pair,
    /// and the morning's, which is that same pair wherever a `dawn_` value
    /// does not say otherwise. Verbatim; core repairs what it cannot use.
    pub fn bands(&self) -> Bands {
        let dusk = Band {
            day_elevation: self.day_elevation,
            night_elevation: self.night_elevation,
        };
        let dawn = Band {
            day_elevation: self.dawn_day_elevation.unwrap_or(dusk.day_elevation),
            night_elevation: self.dawn_night_elevation.unwrap_or(dusk.night_elevation),
        };
        Bands { dawn, dusk }
    }

    /// The operating mode the config implies: a manual location when both
    /// coordinates are given and both are finite numbers, otherwise automatic
    /// (derived from the timezone). TOML can spell `nan` and `inf`, and a
    /// location made of either would paint the screen the deepest red the
    /// table has.
    pub fn mode(&self) -> Mode {
        match (self.latitude, self.longitude) {
            (Some(lat), Some(lon)) if lat.is_finite() && lon.is_finite() => {
                Mode::ManualLocation { lat, lon }
            }
            _ => Mode::Automatic,
        }
    }
}

/// What `load` produced: the config to run with, and whether the file on disk
/// is damaged (existed but could not be read or parsed). A damaged file must
/// never be overwritten by a later save — the user's hand-written settings are
/// still in it, wrong by one typo.
pub struct Loaded {
    pub config: Config,
    pub damaged: bool,
}

/// Loads the config. A missing file yields defaults silently; an unreadable or
/// malformed file prints a clear error, yields defaults, and is flagged
/// `damaged` so nothing ever saves over it. Never fails.
pub fn load() -> Loaded {
    let Some(path) = config_path() else {
        return Loaded {
            config: Config::default(),
            damaged: false,
        };
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // No config of ours at all: the one moment a gammastep or
            // redshift file may be translated in (GitHub #2). The result is
            // written to disk immediately, so this happens exactly once and
            // leaves a visible artifact rather than live magic.
            if let Some((config, tool)) = crate::import::from_incumbents() {
                match save(&config) {
                    Ok(()) => tracing::info!(
                        "imported your {tool} settings into {} (day {} K / night {} K, gamma {}, brightness {}/{})",
                        path.display(),
                        config.day_temp,
                        config.night_temp,
                        config.gamma,
                        config.day_brightness,
                        config.night_brightness
                    ),
                    Err(error) => {
                        tracing::warn!("imported {tool} settings but could not save them: {error}")
                    }
                }
                return Loaded {
                    config,
                    damaged: false,
                };
            }
            return Loaded {
                config: Config::default(),
                damaged: false,
            };
        }
        Err(error) => {
            tracing::warn!(
                "{}: {error}; using defaults, will not overwrite",
                path.display()
            );
            return Loaded {
                config: Config::default(),
                damaged: true,
            };
        }
    };
    match toml::from_str(&text) {
        Ok(config) => Loaded {
            config,
            damaged: false,
        },
        Err(error) => {
            tracing::warn!(
                "{}: {error}; using defaults, will not overwrite",
                path.display()
            );
            Loaded {
                config: Config::default(),
                damaged: true,
            }
        }
    }
}

/// Writes `config` to the config file, creating the directory if needed.
/// Writes a sibling temp file and renames it into place, so a crash mid-write
/// can never leave a truncated config behind. Degrades to an error the caller
/// can log rather than panicking.
pub fn save(config: &Config) -> std::io::Result<()> {
    let Some(path) = config_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = toml::to_string(config).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)
}

/// `$XDG_CONFIG_HOME/nightlightd/config.toml`, or `~/.config/...` as a fallback.
fn config_path() -> Option<PathBuf> {
    nightlightd_core::paths::config_file("config.toml")
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
    fn coordinates_that_are_not_numbers_stay_automatic() {
        // TOML can spell nan and inf, and neither is a place on Earth.
        for text in [
            "latitude = nan\nlongitude = 32.85\n",
            "latitude = 39.93\nlongitude = nan\n",
            "latitude = inf\nlongitude = 32.85\n",
            "latitude = 39.93\nlongitude = -inf\n",
        ] {
            let config: Config = toml::from_str(text).unwrap();
            assert_eq!(config.mode(), Mode::Automatic, "{text}");
        }
    }

    #[test]
    fn shaping_defaults_are_identity() {
        let config = Config::default();
        assert_eq!(config.gamma, 1.0);
        assert_eq!(config.day_brightness, 1.0);
        assert_eq!(config.night_brightness, 1.0);
    }

    /// A file written before #47 has one pair, and it must mean what it
    /// always meant: the same band morning and evening.
    #[test]
    fn a_file_with_one_pair_runs_it_at_both_ends_of_the_day() {
        let config: Config = toml::from_str("night_elevation = -12.0").unwrap();
        let bands = config.bands();
        assert!(!bands.split());
        assert_eq!(bands.dusk.night_elevation, -12.0);
        assert_eq!(bands.dawn.night_elevation, -12.0);
    }

    /// A morning value overrides only its own bound; the other comes from
    /// the evening's pair.
    #[test]
    fn a_dawn_value_sets_only_the_morning() {
        let config: Config =
            toml::from_str("night_elevation = -12.0\ndawn_night_elevation = -3.0").unwrap();
        let bands = config.bands();
        assert_eq!(bands.dawn.night_elevation, -3.0);
        assert_eq!(bands.dawn.day_elevation, 3.0);
        assert_eq!(bands.dusk.night_elevation, -12.0);
    }

    /// Written back, a band nobody split stays two lines, and a split one
    /// comes back as the same two bands.
    #[test]
    fn bands_round_trip_through_the_file() {
        let write = |bands: Bands| {
            let (day_elevation, night_elevation, dawn_day_elevation, dawn_night_elevation) =
                band_fields(bands);
            Config {
                day_elevation,
                night_elevation,
                dawn_day_elevation,
                dawn_night_elevation,
                ..Config::default()
            }
        };
        let together = write(Bands::default());
        assert!(!toml::to_string(&together).unwrap().contains("dawn"));

        let split = Bands {
            dawn: Band {
                day_elevation: 1.0,
                night_elevation: -2.0,
            },
            ..Bands::default()
        };
        let text = toml::to_string(&write(split)).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.bands(), split);
    }

    /// The band defaults to redshift's pair (#39); a hand-written pair
    /// parses and survives a save round trip verbatim, even a nonsensical
    /// one — judging it is core's job, at spend time.
    #[test]
    fn the_band_defaults_and_round_trips_verbatim() {
        let config = Config::default();
        assert_eq!(config.day_elevation, 3.0);
        assert_eq!(config.night_elevation, -6.0);
        let config: Config = toml::from_str("night_elevation = -12.0").unwrap();
        assert_eq!(config.night_elevation, -12.0);
        assert_eq!(config.day_elevation, 3.0);
        let inverted: Config =
            toml::from_str("day_elevation = -9.0\nnight_elevation = 4.0").unwrap();
        let text = toml::to_string(&inverted).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.day_elevation, -9.0);
        assert_eq!(back.night_elevation, 4.0);
    }

    /// The fade ships on (#44); a hand-written `fade = false` parses and
    /// survives a save round trip, so a persist never turns it back on.
    #[test]
    fn the_fade_defaults_on_and_off_round_trips() {
        assert!(Config::default().fade);
        let config: Config = toml::from_str("fade = false").unwrap();
        assert!(!config.fade);
        let text = toml::to_string(&config).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert!(!back.fade);
    }

    #[test]
    fn shaping_fields_parse_and_the_rest_stay_default() {
        // Mumuskeh's actual setup from GitHub #2, translated to our keys.
        let text = "gamma = 0.9\nnight_brightness = 0.9\n";
        let config: Config = toml::from_str(text).unwrap();
        assert_eq!(config.gamma, 0.9);
        assert_eq!(config.day_brightness, 1.0);
        assert_eq!(config.night_brightness, 0.9);
    }

    #[test]
    fn shaping_fields_survive_a_save_round_trip() {
        let config = Config {
            gamma: 0.85,
            night_brightness: 0.9,
            ..Config::default()
        };
        let text = toml::to_string(&config).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back, config);
    }

    #[test]
    fn malformed_config_is_rejected_by_the_parser() {
        // `load()` turns this into a warning and defaults; here we confirm the
        // parse itself fails.
        assert!(toml::from_str::<Config>("night_temp = \"warm\"").is_err());
    }
}
