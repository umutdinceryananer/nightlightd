//! Shared daemon state (#18).
//!
//! The poll loop reads it to decide what to apply; the D-Bus handlers write it
//! and then wake the loop. It is the only state shared between the two threads,
//! and it never touches the screen — so the poll loop stays the single owner of
//! screen access.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nightlightd_core::mode::Mode;

/// The daemon's live state.
pub struct State {
    /// Whether the filter is on. When off, the screen is left neutral.
    pub enabled: bool,
    /// A manual temperature override (from `SetTemperature`); `None` follows the
    /// sun. `SetMode("auto")` clears it.
    pub override_temp: Option<u32>,
    /// The location mode used when following the sun.
    pub mode: Mode,
    /// Daytime temperature bound (kelvin), from the config.
    pub day_temp: u32,
    /// Night temperature bound (kelvin), from the config.
    pub night_temp: u32,
    /// The last temperature actually applied — reported by `GetStatus`.
    pub current_temp: u32,
}

/// State shared between the poll loop and the D-Bus reactor thread.
pub type Shared = Arc<Mutex<State>>;

/// Locks the shared state, recovering from a poisoned mutex rather than
/// panicking — a night light must not die because some thread panicked.
pub fn lock(state: &Shared) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}
