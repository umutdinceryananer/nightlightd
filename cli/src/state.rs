//! Shared daemon state (#18).
//!
//! The poll loop reads it to decide what to apply; the D-Bus handlers write it
//! and then wake the loop. It is the only state shared between the two threads,
//! and it never touches the screen — so the poll loop stays the single owner of
//! screen access.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nightlightd_core::color::{MAX_TEMPERATURE, MIN_TEMPERATURE};
use nightlightd_core::mode::Mode;
use nightlightd_core::transition::Band;

/// Holds a temperature to what the blackbody table can actually render, for
/// every door into this state: the config file at startup and the D-Bus
/// setters afterwards.
///
/// The controls stop at core's `UI_TEMPERATURE_RANGE`, well inside this, but
/// the wire is open to anyone with `busctl` and the file is open to anyone
/// with an editor. Clamping as a value enters rather than at the ramp write
/// is what keeps `GetStatus` honest: a `day_temp = 90000` typo was otherwise
/// reported, and drawn on every curve, all day long while the screen quietly
/// wore 25000 — measured, not supposed. The file itself is still read
/// verbatim; this normalises what the daemon *runs*, and never edits a line
/// under its author unless something else asks for a save.
pub fn renderable(kelvin: u32) -> u32 {
    kelvin.clamp(MIN_TEMPERATURE, MAX_TEMPERATURE)
}

/// The daemon's live state.
pub struct State {
    /// Whether the filter is on. When off, the screen is left neutral.
    pub enabled: bool,
    /// A manual temperature override (from `SetTemperature`); `None` follows the
    /// sun. `SetMode("auto")` clears it.
    pub override_temp: Option<u32>,
    /// The location mode used when following the sun.
    pub mode: Mode,
    /// The mode the config file asked for. `SetMode("auto")` returns to this —
    /// a manual-location user's coordinates must survive a trip through auto —
    /// and persisting derives the saved coordinates from it.
    pub configured_mode: Mode,
    /// Whether the config file on disk failed to load. When set, nothing ever
    /// saves over it: the user's hand-written file is wrong by one typo, not
    /// worthless.
    pub config_damaged: bool,
    /// Daytime temperature bound (kelvin), from the config.
    pub day_temp: u32,
    /// Night temperature bound (kelvin), from the config.
    pub night_temp: u32,
    /// Gamma exponent from the config (GitHub #2), constant across the day.
    /// Carried verbatim so persisting never rewrites a hand-written value.
    pub gamma: f64,
    /// Daytime brightness factor from the config.
    pub day_brightness: f64,
    /// Night brightness factor from the config.
    pub night_brightness: f64,
    /// The last temperature actually applied — reported by `GetStatus`.
    pub current_temp: u32,
    /// The last brightness factor actually applied, for change detection and
    /// the status readout.
    pub current_brightness: f64,
    /// The last gamma exponent actually applied. Differs from `gamma` (the
    /// setting) only mid-fade (#38); the fade needs to know where the screen
    /// is, not where it is headed. Not on the wire.
    pub current_gamma: f64,
    /// Whether target changes ease onto the screen (#44). Read through the
    /// additive `GetFade`, not `GetStatus` — the consolidation promise is
    /// recorded under #34 in ISSUES.md.
    pub fade: bool,
    /// The transition band (#39), carried verbatim from the config; core
    /// degrades a nonsensical pair to the default where it is spent.
    pub band: Band,
    /// The last successfully resolved location (automatic mode). The poll loop
    /// keeps it warm; `GetStatus` reads it instead of re-parsing zone.tab on
    /// every call, and a transient lookup failure reuses it instead of
    /// blanking the screen.
    pub location: Option<(f64, f64)>,
    /// The active outputs as `(crtc, gamma_ramp_size)`, refreshed by every
    /// apply. `GetOutputs` reads this cache; empty until the first apply.
    pub outputs: Vec<(u32, u16)>,
}

/// State shared between the poll loop and the D-Bus reactor thread.
pub type Shared = Arc<Mutex<State>>;

/// Locks the shared state, recovering from a poisoned mutex rather than
/// panicking — a night light must not die because some thread panicked.
pub fn lock(state: &Shared) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}
