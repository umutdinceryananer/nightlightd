//! One-time import of a gammastep or redshift config (GitHub #2).
//!
//! A gammastep user installing this daemon should find their old settings
//! already in place, not retype them. When our config file does not exist at
//! all, `config::load` asks here first: the gammastep INI is tried, then
//! redshift's (same format, same keys — gammastep is its fork). Whatever is
//! understood is translated, written to our config.toml once (a visible
//! artifact, not live magic), and logged. Anything that goes wrong quietly
//! yields nothing: an import must never make startup worse than defaults.

use std::path::PathBuf;

use crate::config::Config;

/// Tries the known INI locations and translates the first that parses into a
/// [`Config`], alongside the name of the tool it came from.
pub fn from_incumbents() -> Option<(Config, &'static str)> {
    for (tool, path) in candidates() {
        if let Ok(text) = std::fs::read_to_string(&path)
            && let Some(config) = translate(&text)
        {
            return Some((config, tool));
        }
    }
    None
}

/// The configs worth trying, in order: gammastep (maintained), then redshift
/// (archived, but its file often outlives it on disk).
fn candidates() -> Vec<(&'static str, PathBuf)> {
    let Some(base) = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    else {
        return Vec::new();
    };
    vec![
        ("gammastep", base.join("gammastep").join("config.ini")),
        ("redshift", base.join("redshift").join("redshift.conf")),
    ]
}

/// Translates the INI text into a [`Config`], starting from our defaults so
/// anything the file does not mention keeps them. Returns `None` when the
/// file contributes nothing we understand — an empty import is no import.
fn translate(text: &str) -> Option<Config> {
    let mut config = Config::default();
    let mut imported = false;
    let mut section = String::new();
    let (mut lat, mut lon) = (None, None);

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = name.to_ascii_lowercase();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
        match (section.as_str(), key.as_str()) {
            ("general", "temp-day") => {
                if let Ok(kelvin) = value.parse() {
                    config.day_temp = kelvin;
                    imported = true;
                }
            }
            ("general", "temp-night") => {
                if let Ok(kelvin) = value.parse() {
                    config.night_temp = kelvin;
                    imported = true;
                }
            }
            ("general", "gamma") => {
                if let Some(gamma) = parse_gamma(value) {
                    config.gamma = gamma;
                    imported = true;
                }
            }
            ("general", "brightness-day") => {
                if let Ok(brightness) = value.parse() {
                    config.day_brightness = brightness;
                    imported = true;
                }
            }
            ("general", "brightness-night") => {
                if let Ok(brightness) = value.parse() {
                    config.night_brightness = brightness;
                    imported = true;
                }
            }
            ("general", "fade") => {
                // gammastep and redshift use 0/1 here (#44). Anything else
                // is not understood and contributes nothing.
                if value == "0" || value == "1" {
                    config.fade = value == "1";
                    imported = true;
                }
            }
            ("manual", "lat") => lat = value.parse().ok(),
            ("manual", "lon") => lon = value.parse().ok(),
            _ => {}
        }
    }

    // A manual location only counts as a pair, like our own config rule.
    if let (Some(lat), Some(lon)) = (lat, lon) {
        config.latitude = Some(lat);
        config.longitude = Some(lon);
        imported = true;
    }
    imported.then_some(config)
}

/// gammastep's gamma is one value or an R:G:B triple; we carry a single
/// exponent, so a triple becomes its average (logged by the caller as an
/// approximation the user can refine).
fn parse_gamma(value: &str) -> Option<f64> {
    if !value.contains(':') {
        return value.parse().ok();
    }
    let parts: Vec<f64> = value
        .split(':')
        .map(|part| part.trim().parse())
        .collect::<Result<_, _>>()
        .ok()?;
    if parts.is_empty() {
        return None;
    }
    Some(parts.iter().sum::<f64>() / parts.len() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mumuskeh's actual file from GitHub #2, verbatim.
    const MUMUSKEH: &str = "\
[general]
temp-day=6000
temp-night=4000
gamma=0.9
brightness-night=0.9
adjustment-method=randr
location-provider=geoclue2
";

    #[test]
    fn the_first_daily_drivers_config_translates() {
        let config = translate(MUMUSKEH).unwrap();
        assert_eq!(config.day_temp, 6000);
        assert_eq!(config.night_temp, 4000);
        assert_eq!(config.gamma, 0.9);
        assert_eq!(config.day_brightness, 1.0);
        assert_eq!(config.night_brightness, 0.9);
        assert_eq!(config.latitude, None);
    }

    #[test]
    fn a_manual_location_imports_as_a_pair() {
        let text = "[manual]\nlat=41.0\nlon=29.0\n";
        let config = translate(text).unwrap();
        assert_eq!(config.latitude, Some(41.0));
        assert_eq!(config.longitude, Some(29.0));
    }

    #[test]
    fn a_lone_coordinate_does_not_count() {
        assert!(translate("[manual]\nlat=41.0\n").is_none());
    }

    /// A gammastep user who turned the fade off arrives here with it off;
    /// one who left it on, or never mentioned it, gets our default.
    #[test]
    fn the_fade_switch_translates() {
        let off = translate("[general]\nfade=0\n").unwrap();
        assert!(!off.fade);
        let on = translate("[general]\nfade=1\n").unwrap();
        assert!(on.fade);
        // Not understood: contributes nothing, alone it is no import.
        assert!(translate("[general]\nfade=maybe\n").is_none());
    }

    #[test]
    fn a_gamma_triple_averages() {
        let config = translate("[general]\ngamma=0.8:0.9:1.0\n").unwrap();
        assert!((config.gamma - 0.9).abs() < 1e-9);
    }

    #[test]
    fn junk_and_unknown_keys_import_nothing() {
        assert!(translate("").is_none());
        assert!(translate("[general]\nadjustment-method=randr\n").is_none());
        assert!(translate("complete nonsense ][ =\n").is_none());
    }

    #[test]
    fn comments_and_case_are_tolerated() {
        let text = "; a comment\n[General]\nTemp-Day = 5900\n# another\n";
        let config = translate(text).unwrap();
        assert_eq!(config.day_temp, 5900);
    }
}
