//! The named themes, and the tone ladder each one derives.
//!
//! One accent colour and a whole palette derived from it by arithmetic —
//! including the *background*: every surface is a shade of the accent, which
//! is what makes a palette read as designed rather than as coloured text on
//! someone's default.
//!
//! The signature is `live` (the default): its accent follows the actual colour
//! the screen is being filtered to, through the same blackbody table the
//! daemon applies. The raw tint is nearly pure white by day, which looks like
//! no theme at all, so the display accent compresses the working range
//! 1500–6500 K into 1900–4300 K: soft gold at noon, deep candle-orange at
//! night. The interface always has character, and still warms with the screen.
//!
//! It lives in this crate, and in plain `(r, g, b)` rather than any toolkit's
//! colour type, because two interfaces wear it and neither owns it. The
//! dashboard converts to ratatui's `Color`, the panel to egui's `Color32`, and
//! a theme added here appears in both — which is the entire point of themes
//! that share their names across two programs.

use crate::color::temperature_to_rgb;

/// A colour, in the only form this crate has an opinion about.
pub type Rgb = (u8, u8, u8);

/// Everything a frame needs, derived from one accent.
pub struct Palette {
    /// The page behind everything: the darkest ground.
    pub bg: Rgb,
    /// A card surface, one step lighter, so content sits on raised panels
    /// without needing borders.
    pub surface: Rgb,
    /// Widget grounds and floating overlays: a visibly lighter shade again,
    /// standing in for a border.
    pub raised: Rgb,
    /// The lead surface: the one block on screen that is announcing something
    /// rather than containing something.
    pub hero: Rgb,
    /// Default text: near-white, faintly tinted toward the accent.
    pub text: Rgb,
    /// Emphasis: titles, the curve, chips, the wordmark.
    pub accent: Rgb,
    /// Data values: times, numbers, coordinates. A second hue where the theme
    /// carries one; a lighter shade of the accent where it does not.
    pub accent2: Rgb,
    /// Chrome: borders, labels, secondary text.
    pub muted: Rgb,
    /// Barely-there: hour ticks, rules, the world map.
    pub faint: Rgb,
    /// State good — constant across themes, on purpose. State must never be
    /// swallowed by a palette.
    pub ok: Rgb,
    /// State bad — constant across themes, for the same reason.
    pub err: Rgb,
}

/// A named theme: a fixed accent (or `None` for the live screen tint), an
/// optional secondary hue for data values, and an optional designed page base.
/// `None` for the secondary keeps monochrome themes monochrome; `None` for the
/// base derives a near-black shade of the accent (the warm original look),
/// while `Some` pins the neutral dark ground the modern editor palettes are
/// built on.
pub struct Theme {
    pub name: &'static str,
    accent: Option<Rgb>,
    secondary: Option<Rgb>,
    base: Option<Rgb>,
}

/// Cycle order; `live` first because it is the identity.
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

/// The index of a theme by name.
pub fn index_of(name: &str) -> Option<usize> {
    THEMES.iter().position(|theme| theme.name == name)
}

/// The theme at `index`, falling back to `live` — a saved name that no longer
/// exists must give you the default, not nothing.
pub fn at(index: usize) -> &'static Theme {
    THEMES.get(index).unwrap_or(&THEMES[0])
}

/// The display tint for a temperature: the blackbody colour after compressing
/// the working range into [`LIVE_DISPLAY_MIN`]–[`LIVE_DISPLAY_MAX`]. Raw
/// 6500 K is pure white — honest, but on screen it reads as no colour at all;
/// this keeps daytime a soft gold and night a deep orange. Shared by the live
/// theme's accent and by every temperature printed anywhere, so the two always
/// agree.
///
/// Past neutral (#41) the tint stops rather than turning blue. This scale
/// answers "how warm is the screen", and above 6500 K the answer is "not at
/// all"; a bluish accent would be a second, opposite meaning on the one
/// colour every interface reads warmth from.
pub fn display_tint(kelvin: u32) -> Rgb {
    let kelvin = f64::from(kelvin.clamp(1500, 6500));
    let display =
        LIVE_DISPLAY_MIN + (kelvin - 1500.0) / 5000.0 * (LIVE_DISPLAY_MAX - LIVE_DISPLAY_MIN);
    let (r, g, b) = temperature_to_rgb(display.round() as u32);
    (to_u8(r), to_u8(g), to_u8(b))
}

impl Theme {
    /// Whether this theme's accent comes from the screen rather than from the
    /// table — the one theme that has to be redrawn as the day goes on.
    pub fn is_live(&self) -> bool {
        self.accent.is_none()
    }

    /// Resolves the palette. `applied_kelvin` feeds the live theme; fixed
    /// themes ignore it.
    pub fn palette(&self, applied_kelvin: Option<u32>) -> Palette {
        let accent = self
            .accent
            .unwrap_or_else(|| display_tint(applied_kelvin.unwrap_or(6500)));
        // A designed base lightens toward white for elevation and pulls the
        // chrome tones from accent-over-base; a derived base shades everything
        // from the accent alone, as the original look did. The hero is the
        // exception on a designed ground: it lifts toward the *accent* rather
        // than toward white, because it is the one surface whose job is to
        // wear a colour rather than to hold content.
        let (bg, surface, raised, hero, muted, faint) = match self.base {
            Some(base) => (
                base,
                mix(base, WHITE, 0.06),
                mix(base, WHITE, 0.14),
                mix(base, accent, 0.30),
                // As bright over the base as the derived branch is over black,
                // or secondary text turns illegible on the designed grounds.
                mix(base, accent, 0.62),
                mix(base, accent, 0.34),
            ),
            None => (
                mix(BLACK, accent, 0.10),
                mix(BLACK, accent, 0.15),
                mix(BLACK, accent, 0.21),
                mix(BLACK, accent, 0.26),
                mix(BLACK, accent, 0.62),
                mix(BLACK, accent, 0.32),
            ),
        };
        Palette {
            bg,
            surface,
            raised,
            hero,
            text: mix(WHITE, accent, 0.16),
            accent,
            accent2: match self.secondary {
                Some(secondary) => secondary,
                None => mix(WHITE, accent, 0.55),
            },
            muted,
            faint,
            ok: (90, 220, 120),
            err: (240, 90, 90),
        }
    }
}

const BLACK: Rgb = (0, 0, 0);
const WHITE: Rgb = (255, 255, 255);

fn to_u8(channel: f64) -> u8 {
    (channel * 255.0).round().clamp(0.0, 255.0) as u8
}

/// Linear blend from `base` toward `tint` by `amount` (0.0 = base, 1.0 = tint).
fn mix(base: Rgb, tint: Rgb, amount: f64) -> Rgb {
    let channel = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * amount) as u8;
    (
        channel(base.0, tint.0),
        channel(base.1, tint.1),
        channel(base.2, tint.2),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn luminance((r, g, b): Rgb) -> u16 {
        u16::from(r) + u16::from(g) + u16::from(b)
    }

    #[test]
    fn live_theme_has_character_by_day_and_deepens_at_night() {
        let live = at(0);
        assert!(live.is_live());
        // Daytime maps to soft gold — never the washed-out pure white of the
        // raw 6500 K blackbody point.
        let (r, _, b) = live.palette(Some(6500)).accent;
        assert_eq!(r, 255);
        assert!((120..240).contains(&b), "day blue channel {b} is not gold");
        // Night is visibly deeper than day.
        let (_, _, night_b) = live.palette(Some(2800)).accent;
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
        // An index out of the table is the default, not a panic: a saved name
        // outlives the release that removed the theme it named.
        assert_eq!(at(THEMES.len()).name, "live");
    }

    /// The ladder has to hold on every theme, or a card vanishes into the page
    /// on the one nobody tested. This is the whole contract the two interfaces
    /// lean on when they stop asking for colours by name.
    #[test]
    fn every_theme_keeps_its_surfaces_in_order() {
        for theme in THEMES {
            let pal = theme.palette(Some(3000));
            let name = theme.name;
            assert!(
                luminance(pal.bg) < 180,
                "{name} bg must stay a dark ground, got {:?}",
                pal.bg
            );
            assert!(luminance(pal.bg) < luminance(pal.surface), "{name} surface");
            assert!(
                luminance(pal.surface) < luminance(pal.raised),
                "{name} raised"
            );
            assert!(luminance(pal.raised) < luminance(pal.hero), "{name} hero");
            assert!(luminance(pal.faint) < luminance(pal.muted), "{name} muted");
            assert!(luminance(pal.muted) < luminance(pal.text), "{name} text");
        }
    }

    /// The live theme is the only one that moves, and it has to keep the
    /// ladder at both ends of the day, not just at the midpoint the loop
    /// above happens to sample.
    #[test]
    fn the_live_ladder_holds_at_every_temperature() {
        for kelvin in [1500, 2800, 4200, 6500] {
            let pal = at(0).palette(Some(kelvin));
            assert!(luminance(pal.bg) < luminance(pal.surface), "{kelvin} K");
            assert!(luminance(pal.surface) < luminance(pal.raised), "{kelvin} K");
            assert!(luminance(pal.raised) < luminance(pal.hero), "{kelvin} K");
            assert!(luminance(pal.muted) < luminance(pal.text), "{kelvin} K");
        }
        // An unreachable daemon gets the daytime palette rather than a random
        // one — a client that cannot read the screen must not invent a mood.
        assert_eq!(at(0).palette(None).accent, display_tint(6500));
    }
}
