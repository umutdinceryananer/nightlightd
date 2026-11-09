//! The status snapshot the daemon returns to `--status`.
//!
//! Everything the user might want to eyeball without reaching for `journalctl`:
//! is the filter on, what temperature is applied, what is driving it, and — the
//! sanity check that the daemon's clock and timezone are right — where the sun
//! is right now. The daemon fills this in (it owns the state); the client only
//! prints it.

use serde::{Deserialize, Serialize};
use zbus::zvariant::Type;

/// A point-in-time snapshot of the daemon, returned by `GetStatus`.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct Status {
    /// Whether the filter is on. When off, the screen is left neutral.
    pub enabled: bool,
    /// The temperature currently applied to the screen (kelvin).
    pub temperature: u32,
    /// What is driving the temperature: `"auto (following the sun)"`,
    /// `"manual override (2800 K)"`, `"off (screen left neutral)"`, ...
    pub source: String,
    /// The current solar elevation in degrees. Only meaningful when
    /// [`has_location`](Self::has_location) is set.
    pub elevation: f64,
    /// Whether a location could be resolved, so `elevation`, `latitude` and
    /// `longitude` are real. `false` in fixed mode or when the timezone lookup
    /// fails.
    pub has_location: bool,
    /// Resolved latitude in degrees (valid only when `has_location`).
    pub latitude: f64,
    /// Resolved longitude in degrees (valid only when `has_location`).
    pub longitude: f64,
    /// Whether the filter is actively tracking the sun right now: on, no manual
    /// override, and not a fixed temperature. This is what the tray's
    /// "Automatic" checkbox reflects.
    pub following: bool,
    /// The configured daytime temperature bound (kelvin) — the top of the
    /// automatic curve. The panel needs it to draw the day/night curve.
    pub day_temp: u32,
    /// The configured night temperature bound (kelvin) — the bottom of the
    /// automatic curve.
    pub night_temp: u32,
    /// The gamma exponent in force (GitHub #2); 1.0 is the identity.
    pub gamma: f64,
    /// The brightness factor currently applied to the screen.
    pub brightness: f64,
    /// The configured daytime brightness bound.
    pub day_brightness: f64,
    /// The configured night brightness bound.
    pub night_brightness: f64,
    /// Whether a change of target eases onto the screen (#44).
    pub fade: bool,
    /// The solar elevation at or above which it is full daytime (#39), as
    /// configured; a nonsensical pair is repaired where it is spent.
    pub day_elevation: f64,
    /// The solar elevation at or below which it is full night, as configured.
    pub night_elevation: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tray, the panel and the dashboard re-declare this struct by hand;
    /// all copies must serialize to exactly this D-Bus signature or clients
    /// half-break (reads fail, writes still work). Drift shipped once — commit
    /// c177090 — and this pin is what turns the next drift into a red test
    /// instead of a silent "daemon not running". The same literal is pinned in
    /// each client's daemon.rs; change all four together.
    #[test]
    fn wire_signature_is_pinned() {
        assert_eq!(Status::SIGNATURE.to_string(), "(busdbddbuuddddbdd)");
    }
}
