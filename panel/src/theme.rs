//! The panel's colours, derived from the screen's own temperature.
//!
//! This is the dashboard's "live" theme ported to egui, and it is deliberately
//! the same arithmetic rather than a similar-looking hand-picked palette: the
//! accent is the applied temperature run through the same display mapping, and
//! every other tone is mixed from that accent by the same fractions. Open the
//! panel and the dashboard side by side at dusk and they warm together.
//!
//! Why the temperature at all: every other night light is grey chrome around a
//! number. Here the window is wearing what it is doing.

use eframe::egui::Color32;
use nightlightd_core::color::temperature_to_rgb;

/// The window's colours for one moment.
pub struct Palette {
    /// The page behind everything: a near-black shade of the accent.
    pub bg: Color32,
    /// A card surface, one step lighter, so panels rise without borders.
    pub surface: Color32,
    /// The state card's ground: the accent pushed far enough into the page
    /// that the block reads as the temperature it is announcing, rather than
    /// as another grey box with a number in it.
    pub hero: Color32,
    /// Widget grounds: slider rails, button faces.
    pub raised: Color32,
    /// Default text: near-white, faintly tinted toward the accent.
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
    /// stopped or mismatched daemon is not applying a temperature, so the
    /// card must not be wearing one — this is the only tone here that does
    /// not follow the screen.
    pub warn_ground: Color32,
}

/// The accent for an applied temperature. Raw 6500 K is pure white — honest,
/// but on screen it reads as no colour at all — so the working range is
/// squeezed into a band that keeps daytime a soft gold and night a deep
/// orange. Same constants as the dashboard's, so the two never disagree.
const DISPLAY_MIN: f64 = 1900.0;
const DISPLAY_MAX: f64 = 4300.0;

/// The dashboard's live theme carries a fixed cool secondary, so numbers stay
/// legible against an accent that is warm at every hour.
const SECONDARY: (u8, u8, u8) = (130, 170, 190);

pub fn display_tint(kelvin: u32) -> Color32 {
    let kelvin = f64::from(kelvin.clamp(1500, 6500));
    let display = DISPLAY_MIN + (kelvin - 1500.0) / 5000.0 * (DISPLAY_MAX - DISPLAY_MIN);
    let (r, g, b) = temperature_to_rgb(display.round() as u32);
    Color32::from_rgb(to_u8(r), to_u8(g), to_u8(b))
}

impl Palette {
    /// The palette for an applied temperature, or the daytime one when the
    /// daemon cannot be reached — a panel that cannot read the screen should
    /// not invent a mood for it.
    pub fn live(applied: Option<u32>) -> Self {
        let accent = display_tint(applied.unwrap_or(6500));
        let a = (accent.r(), accent.g(), accent.b());
        Self {
            bg: mix(BLACK, a, 0.10),
            surface: mix(BLACK, a, 0.15),
            hero: mix(BLACK, a, 0.26),
            raised: mix(BLACK, a, 0.21),
            text: mix(WHITE, a, 0.16),
            accent,
            accent2: Color32::from_rgb(SECONDARY.0, SECONDARY.1, SECONDARY.2),
            muted: mix(BLACK, a, 0.62),
            faint: mix(BLACK, a, 0.32),
            warn_ground: mix(BLACK, (235, 150, 60), 0.24),
        }
    }
}

const BLACK: (u8, u8, u8) = (0, 0, 0);
const WHITE: (u8, u8, u8) = (255, 255, 255);

fn to_u8(channel: f64) -> u8 {
    (channel * 255.0).round().clamp(0.0, 255.0) as u8
}

fn mix(base: (u8, u8, u8), tint: (u8, u8, u8), amount: f64) -> Color32 {
    let channel = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * amount) as u8;
    Color32::from_rgb(
        channel(base.0, tint.0),
        channel(base.1, tint.1),
        channel(base.2, tint.2),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The point of the live accent: it has to be visibly different at the
    /// two ends of the day, or it is decoration pretending to be information.
    #[test]
    fn the_accent_warms_from_day_to_night() {
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

    /// An unreachable daemon gets the daytime palette rather than a random
    /// one, and the tones stay ordered however warm the accent is.
    #[test]
    fn the_tones_stay_ordered_at_every_temperature() {
        for kelvin in [1500, 2800, 4200, 6500] {
            let pal = Palette::live(Some(kelvin));
            let lum = |c: Color32| u32::from(c.r()) + u32::from(c.g()) + u32::from(c.b());
            assert!(lum(pal.bg) < lum(pal.surface), "surface must lift off bg");
            assert!(lum(pal.surface) < lum(pal.raised));
            assert!(lum(pal.raised) < lum(pal.hero), "the state card leads");
            assert!(lum(pal.faint) < lum(pal.muted));
            assert!(lum(pal.muted) < lum(pal.text));
        }
        // The notice ground is the one tone that must not move with the
        // accent: a card that is not applying a temperature cannot wear one.
        let warm = Palette::live(Some(1700)).warn_ground;
        let cool = Palette::live(Some(6500)).warn_ground;
        assert_eq!(warm, cool);

        let unreachable = Palette::live(None);
        assert_eq!(unreachable.accent, display_tint(6500));
    }
}
