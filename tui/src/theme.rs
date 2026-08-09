//! The dashboard's palette: `core::theme` in ratatui's colours.
//!
//! The table, the tone ladder and the live-accent arithmetic all live in core,
//! because the panel wears the same themes and neither interface owns them.
//! What is left here is the conversion, and the two names the terminal uses
//! for tones the shared ladder calls something else.

use nightlightd_core::theme::{self, Rgb};
use ratatui::style::Color;

pub use nightlightd_core::theme::{THEMES, index_of};

/// Everything a frame needs, in ratatui's colour type.
pub struct Palette {
    /// The painted screen background: a near-black shade of the accent.
    pub bg: Color,
    /// Default text: near-white, faintly tinted toward the accent.
    pub text: Color,
    /// Emphasis: titles, the curve, chips, the wordmark.
    pub accent: Color,
    /// Data values: times, numbers, coordinates.
    pub accent2: Color,
    /// Chrome: borders, labels, secondary text.
    pub muted: Color,
    /// Barely-there: the world map, the now-line.
    pub faint: Color,
    /// The card surface, so content sits on raised panels without borders.
    pub surface: Color,
    /// The elevated surface behind floating overlays (the theme popup),
    /// standing in for a border. The shared ladder's `raised`.
    pub overlay: Color,
    /// State good — constant across themes.
    pub ok: Color,
    /// State bad — constant across themes.
    pub err: Color,
}

/// The palette for a theme index. `applied_kelvin` feeds the live theme; fixed
/// themes ignore it. An index past the end of the table gives `live` rather
/// than nothing.
pub fn palette(index: usize, applied_kelvin: Option<u32>) -> Palette {
    let pal = theme::at(index).palette(applied_kelvin);
    Palette {
        bg: rgb(pal.bg),
        text: rgb(pal.text),
        accent: rgb(pal.accent),
        accent2: rgb(pal.accent2),
        muted: rgb(pal.muted),
        faint: rgb(pal.faint),
        surface: rgb(pal.surface),
        overlay: rgb(pal.raised),
        ok: rgb(pal.ok),
        err: rgb(pal.err),
    }
}

/// The display tint for a temperature, for the curve's gradient fill and every
/// kelvin figure printed anywhere.
pub fn display_tint(kelvin: u32) -> Color {
    rgb(theme::display_tint(kelvin))
}

fn rgb((r, g, b): Rgb) -> Color {
    Color::Rgb(r, g, b)
}
