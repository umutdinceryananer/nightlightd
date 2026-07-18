//! The theme system (design roadmap, phase A).
//!
//! One accent colour and everything else derived from it by arithmetic — the
//! one-hue cohesion trick, computed instead of hand-picked. The signature is
//! the `live` theme (the default): its accent is the actual colour the screen
//! is filtered to right now, via the same blackbody table the daemon applies.
//! At 6500 K the interface is neutral; as the screen warms at night, so does
//! the interface. Fixed themes cover people who want stable colours (and give
//! the README its gallery).
//!
//! Semantic colours (ok green / err red) stay constant across themes on
//! purpose: state must never be swallowed by a palette.

use nightlightd_core::color::temperature_to_rgb;
use ratatui::style::Color;

/// Everything a frame needs, derived from one accent.
pub struct Palette {
    /// Emphasis: titles, the curve, chips, gauge fill, the wordmark.
    pub accent: Color,
    /// Chrome: borders, labels, secondary text.
    pub muted: Color,
    /// Barely-there: gauge track, the now-line.
    pub faint: Color,
    /// State good — constant across themes.
    pub ok: Color,
    /// State bad — constant across themes.
    pub err: Color,
}

/// A named theme: a fixed accent, or `None` for the live screen tint.
pub struct Theme {
    pub name: &'static str,
    accent: Option<(u8, u8, u8)>,
}

/// Cycle order for the `T` key; `live` first because it is the identity.
pub const THEMES: &[Theme] = &[
    Theme {
        name: "live",
        accent: None,
    },
    Theme {
        name: "ember",
        accent: Some((255, 170, 90)),
    },
    Theme {
        name: "gruvbox",
        accent: Some((250, 189, 47)),
    },
    Theme {
        name: "nord",
        accent: Some((136, 192, 208)),
    },
    Theme {
        name: "tokyo-night",
        accent: Some((122, 162, 247)),
    },
    Theme {
        name: "phosphor",
        accent: Some((51, 255, 102)),
    },
    Theme {
        name: "synthwave",
        accent: Some((255, 110, 199)),
    },
];

/// The index of a theme by name, for `--theme`.
pub fn index_of(name: &str) -> Option<usize> {
    THEMES.iter().position(|theme| theme.name == name)
}

impl Theme {
    /// Resolves the palette. `applied_kelvin` feeds the live theme; fixed
    /// themes ignore it.
    pub fn palette(&self, applied_kelvin: Option<u32>) -> Palette {
        let (r, g, b) = match self.accent {
            Some(rgb) => rgb,
            None => {
                let (r, g, b) = temperature_to_rgb(applied_kelvin.unwrap_or(6500));
                (to_u8(r), to_u8(g), to_u8(b))
            }
        };
        Palette {
            accent: Color::Rgb(r, g, b),
            muted: scaled(r, g, b, 0.55),
            faint: scaled(r, g, b, 0.28),
            ok: Color::Rgb(90, 220, 120),
            err: Color::Rgb(240, 90, 90),
        }
    }
}

fn to_u8(channel: f64) -> u8 {
    (channel * 255.0).round().clamp(0.0, 255.0) as u8
}

fn scaled(r: u8, g: u8, b: u8, factor: f64) -> Color {
    let scale = |v: u8| (f64::from(v) * factor).round() as u8;
    Color::Rgb(scale(r), scale(g), scale(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_theme_is_neutral_by_day_and_warm_at_night() {
        let live = &THEMES[0];
        // 6500 K is the neutral white point -> a neutral interface.
        let day = live.palette(Some(6500));
        assert_eq!(day.accent, Color::Rgb(255, 255, 255));
        // 2800 K is visibly warm -> red full, blue well below.
        let Color::Rgb(r, _, b) = live.palette(Some(2800)).accent else {
            panic!("accent must be rgb");
        };
        assert_eq!(r, 255);
        assert!(b < 160, "blue channel {b} should be suppressed at 2800 K");
    }

    #[test]
    fn every_theme_is_reachable_by_name() {
        for (index, theme) in THEMES.iter().enumerate() {
            assert_eq!(index_of(theme.name), Some(index));
        }
        assert_eq!(index_of("nope"), None);
    }
}
