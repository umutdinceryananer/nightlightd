//! The panel's palette: `core::theme` in egui's colours.
//!
//! The table and the tone ladder live in core, so the panel and the dashboard
//! wear the same eight themes rather than two sets that merely started out
//! alike. Open them side by side on `nord` and they are the same nord; leave
//! both on `live` at dusk and they warm together.
//!
//! Why the screen's temperature is the default accent at all: every other
//! night light is grey chrome around a number. Here the window is wearing what
//! it is doing.

use eframe::egui::Color32;
use nightlightd_core::theme::{self, Rgb};

pub use nightlightd_core::theme::{THEMES, index_of};

/// The window's colours for one moment.
pub struct Palette {
    /// The page behind everything.
    pub bg: Color32,
    /// A card surface, so panels rise without borders.
    pub surface: Color32,
    /// The state card's ground: the lead surface, wearing the accent rather
    /// than being another grey box with a number in it.
    pub hero: Color32,
    /// Widget grounds: slider rails, button faces, badges.
    pub raised: Color32,
    /// Default text.
    pub text: Color32,
    /// Emphasis: the title, the curve, a filled slider.
    pub accent: Color32,
    /// Data values, in a second hue so numbers do not compete with the title.
    pub accent2: Color32,
    /// Chrome: labels, secondary text.
    pub muted: Color32,
    /// Barely there: hour ticks, rules.
    pub faint: Color32,
    /// The ground under a card that is a notice rather than a reading. A
    /// stopped or mismatched daemon is not applying a temperature, so the card
    /// must not be wearing one — and it must not be wearing the theme either,
    /// or on `nord` a warning is just another blue panel. This is the one tone
    /// here that follows neither the screen nor the theme.
    pub warn_ground: Color32,
}

impl Palette {
    /// The palette for a theme and an applied temperature. The temperature
    /// only reaches the live theme; the fixed ones ignore it. `None` — an
    /// unreachable daemon — gets the daytime accent rather than a random one,
    /// because a panel that cannot read the screen should not invent a mood
    /// for it.
    pub fn of(index: usize, applied: Option<u32>) -> Self {
        let pal = theme::at(index).palette(applied);
        Self {
            bg: rgb(pal.bg),
            surface: rgb(pal.surface),
            hero: rgb(pal.hero),
            raised: rgb(pal.raised),
            text: rgb(pal.text),
            accent: rgb(pal.accent),
            accent2: rgb(pal.accent2),
            muted: rgb(pal.muted),
            faint: rgb(pal.faint),
            warn_ground: rgb(mix_black((235, 150, 60), 0.24)),
        }
    }
}

/// The tint a temperature is printed in, wherever one is printed. Deliberately
/// outside the theme: a kelvin figure's colour is data, not chrome, and 2800 K
/// has to look like 2800 K on `phosphor` too.
pub fn display_tint(kelvin: u32) -> Color32 {
    rgb(theme::display_tint(kelvin))
}

fn rgb((r, g, b): Rgb) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn mix_black((r, g, b): Rgb, amount: f64) -> Rgb {
    let channel = |c: u8| (f64::from(c) * amount) as u8;
    (channel(r), channel(g), channel(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ladder itself is core's contract and core tests it. What is the
    /// panel's own is the tone that must escape the theme: a warning has to
    /// look like a warning on all eight, or it is just another card.
    #[test]
    fn the_notice_ground_follows_neither_theme_nor_screen() {
        let reference = Palette::of(0, Some(6500)).warn_ground;
        for (index, theme) in THEMES.iter().enumerate() {
            for kelvin in [1500, 2800, 6500] {
                assert_eq!(
                    Palette::of(index, Some(kelvin)).warn_ground,
                    reference,
                    "{} at {kelvin} K",
                    theme.name
                );
            }
        }
        // Warmer than any of the grounds it can sit among, so it reads as a
        // flag rather than as the next card down.
        assert!(reference.r() > reference.b());
    }

    /// A kelvin figure is data. It has to be the same colour on every theme,
    /// or the same number means two things in two windows.
    #[test]
    fn a_temperature_is_printed_in_its_own_colour_on_every_theme() {
        let day = display_tint(6500);
        let night = display_tint(1700);
        assert_eq!(day.r(), 255, "daytime should still be a warm white");
        assert!(
            night.b() + 40 < day.b(),
            "night ({}) must be visibly deeper than day ({})",
            night.b(),
            day.b()
        );
    }
}
