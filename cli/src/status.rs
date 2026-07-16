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
}
