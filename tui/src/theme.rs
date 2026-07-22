//! The theme system (design roadmap, phase A).
//!
//! One accent colour and a full tone ladder derived from it by arithmetic —
//! including the *background*: the whole screen is painted in a near-black
//! shade of the accent, which is what makes a palette read as designed rather
//! than as coloured text on someone's terminal default.
//!
//! The signature is the `live` theme (the default): its accent follows the
//! actual colour the screen is filtered to, via the same blackbody table the
//! daemon applies. The raw blackbody tint is nearly pure white by day, which
//! looks like no theme at all, so the display accent compresses the working
//! range 1500–6500 K into 1900–4300 K: soft gold at noon, deep candle-orange
//! at night. The interface always has character, and still warms with the
//! screen.
//!
//! Semantic colours (ok green / err red) stay constant across themes on
//! purpose: state must never be swallowed by a palette.

use nightlightd_core::color::temperature_to_rgb;
use ratatui::style::Color;

/// Everything a frame needs, derived from one accent.
pub struct Palette {
    /// The painted screen background: a near-black shade of the accent.
    pub bg: Color,
    /// Default text: near-white, faintly tinted toward the accent.
    pub text: Color,
    /// Emphasis: titles, the curve, chips, the wordmark.
    pub accent: Color,
    /// Data values: times, numbers, coordinates. A second hue where the theme
    /// carries one; a lighter shade of the accent where it does not.
    pub accent2: Color,
    /// Chrome: borders, labels, secondary text.
    pub muted: Color,
    /// Barely-there: the world map, the now-line.
    pub faint: Color,
    /// The card surface: one step lighter than the page, so content sits on
    /// raised panels without needing borders.
    pub surface: Color,
    /// The elevated surface behind floating overlays (the theme popup): a
    /// visibly lighter shade again, standing in for a border.
    pub overlay: Color,
    /// State good — constant across themes.
    pub ok: Color,
    /// State bad — constant across themes.
    pub err: Color,
}

/// A named theme: a fixed accent (or `None` for the live screen tint), an
/// optional secondary hue for data values, and an optional designed page
/// base. `None` for the secondary keeps monochrome themes monochrome; `None`
/// for the base derives a near-black shade of the accent (the warm original
/// look), while `Some` pins the neutral dark ground the modern editor
/// palettes are built on.
pub struct Theme {
    pub name: &'static str,
    accent: Option<(u8, u8, u8)>,
    secondary: Option<(u8, u8, u8)>,
    base: Option<(u8, u8, u8)>,
}

/// Cycle order for the `T` key; `live` first because it is the identity.
pub const THEMES: &[Theme] = &[
    Theme {
        // Warm accent from the screen, cool steel for data: the screen warms,
        // the numbers stay calm.
        name: "live",
        accent: None,
        secondary: Some((130, 170, 190)),
        base: None,
    },
    Theme {
        // The real Tokyo Night ground (#1a1b26), storm blue and purple.
        name: "tokyo",
        accent: Some((122, 162, 247)),
        secondary: Some((187, 154, 247)),
        base: Some((26, 27, 38)),
    },
    Theme {
        // Catppuccin mocha: mauve on the classic deep mantle, teal data.
        name: "mocha",
        accent: Some((203, 166, 247)),
        secondary: Some((148, 226, 213)),
        base: Some((30, 30, 46)),
    },
    Theme {
        // Polar night ground, frost accent, aurora-yellow data.
        name: "nord",
        accent: Some((136, 192, 208)),
        secondary: Some((235, 203, 139)),
        base: Some((46, 52, 64)),
    },
    Theme {
        // Gruvbox dark on its true grey ground, not a yellowed black.
        name: "gruvbox",
        accent: Some((250, 189, 47)),
        secondary: Some((142, 192, 124)),
        base: Some((40, 40, 40)),
    },
    Theme {
        // Hot pink and cyan over a deep violet night.
        name: "synth",
        accent: Some((255, 110, 199)),
        secondary: Some((100, 220, 255)),
        base: Some((36, 23, 54)),
    },
    Theme {
        name: "ember",
        accent: Some((255, 170, 90)),
        secondary: Some((108, 190, 180)),
        base: None,
    },
    Theme {
        // Deliberately monochrome — a phosphor CRT has one colour.
        name: "phosphor",
        accent: Some((51, 255, 102)),
        secondary: None,
        base: None,
    },
];

/// The visual range the live accent moves in. The real filter range
/// (1500–6500 K) maps linearly into this, so daytime is gold, not white.
const LIVE_DISPLAY_MIN: f64 = 1900.0;
const LIVE_DISPLAY_MAX: f64 = 4300.0;

/// The index of a theme by name, for `--theme`.
pub fn index_of(name: &str) -> Option<usize> {
    THEMES.iter().position(|theme| theme.name == name)
}

/// The display tint for a temperature: the blackbody colour after compressing
/// the working range into [`LIVE_DISPLAY_MIN`]–[`LIVE_DISPLAY_MAX`]. Raw
/// 6500 K is pure white — honest, but on screen it reads as no colour at all;
/// this keeps daytime a soft gold and night a deep orange. Shared by the live
/// theme's accent and the curve's gradient fill, so the two always agree.
pub fn display_tint(kelvin: u32) -> Color {
    let kelvin = f64::from(kelvin.clamp(1500, 6500));
    let display =
        LIVE_DISPLAY_MIN + (kelvin - 1500.0) / 5000.0 * (LIVE_DISPLAY_MAX - LIVE_DISPLAY_MIN);
    let (r, g, b) = temperature_to_rgb(display.round() as u32);
    Color::Rgb(to_u8(r), to_u8(g), to_u8(b))
}

impl Theme {
    /// Resolves the palette. `applied_kelvin` feeds the live theme; fixed
    /// themes ignore it.
    pub fn palette(&self, applied_kelvin: Option<u32>) -> Palette {
        let accent = match self.accent {
            Some(rgb) => rgb,
            None => match display_tint(applied_kelvin.unwrap_or(6500)) {
                Color::Rgb(r, g, b) => (r, g, b),
                _ => (255, 170, 90),
            },
        };
        let secondary = self.secondary.unwrap_or(accent);
        // A designed base lightens toward white for elevation and pulls the
        // chrome tones from accent-over-base; a derived base shades everything
        // from the accent alone, as the original look did.
        let (bg, surface, overlay, muted, faint) = match self.base {
            Some(base) => (
                rgb(base),
                mix(base, (255, 255, 255), 0.06),
                mix(base, (255, 255, 255), 0.14),
                // As bright over the base as the derived branch is over black,
                // or secondary text turns illegible on the designed grounds.
                mix(base, accent, 0.62),
                mix(base, accent, 0.34),
            ),
            None => (
                mix((0, 0, 0), accent, 0.10),
                mix((0, 0, 0), accent, 0.15),
                mix((0, 0, 0), accent, 0.21),
                mix((0, 0, 0), accent, 0.62),
                mix((0, 0, 0), accent, 0.32),
            ),
        };
        Palette {
            bg,
            text: mix((255, 255, 255), accent, 0.16),
            accent: rgb(accent),
            accent2: match self.secondary {
                Some(_) => rgb(secondary),
                None => mix((255, 255, 255), accent, 0.55),
            },
            muted,
            faint,
            surface,
            overlay,
            ok: Color::Rgb(90, 220, 120),
            err: Color::Rgb(240, 90, 90),
        }
    }
}

fn to_u8(channel: f64) -> u8 {
    (channel * 255.0).round().clamp(0.0, 255.0) as u8
}

fn rgb((r, g, b): (u8, u8, u8)) -> Color {
    Color::Rgb(r, g, b)
}

/// Linear blend from `base` toward `tint` by `amount` (0.0 = base, 1.0 = tint).
fn mix(base: (u8, u8, u8), tint: (u8, u8, u8), amount: f64) -> Color {
    let channel = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * amount) as u8;
    Color::Rgb(
        channel(base.0, tint.0),
        channel(base.1, tint.1),
        channel(base.2, tint.2),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accent_rgb(palette: &Palette) -> (u8, u8, u8) {
        match palette.accent {
            Color::Rgb(r, g, b) => (r, g, b),
            _ => panic!("accent must be rgb"),
        }
    }

    #[test]
    fn live_theme_has_character_by_day_and_deepens_at_night() {
        let live = &THEMES[0];
        // Daytime maps to soft gold — never the washed-out pure white of the
        // raw 6500 K blackbody point.
        let (r, _, b) = accent_rgb(&live.palette(Some(6500)));
        assert_eq!(r, 255);
        assert!(
            (120..240).contains(&b),
            "day blue channel {b} should be gold"
        );
        // Night is visibly deeper than day.
        let (_, _, night_b) = accent_rgb(&live.palette(Some(2800)));
        assert!(
            night_b < b,
            "night ({night_b}) must be warmer than day ({b})"
        );
    }

    #[test]
    fn every_theme_is_reachable_by_name() {
        for (index, theme) in THEMES.iter().enumerate() {
            assert_eq!(index_of(theme.name), Some(index));
        }
        assert_eq!(index_of("nope"), None);
    }

    #[test]
    fn every_background_stays_dark() {
        // Derived bases are near-black; designed bases (nord is the palest)
        // must still read as a dark ground, never a mid grey.
        for theme in THEMES {
            let palette = theme.palette(Some(3000));
            let Color::Rgb(r, g, b) = palette.bg else {
                panic!("bg must be rgb");
            };
            assert!(
                u16::from(r) + u16::from(g) + u16::from(b) < 180,
                "{} bg must stay dark, got {r},{g},{b}",
                theme.name
            );
        }
    }
}
