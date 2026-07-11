//! The D-Bus service: `org.nightlightd.Daemon` (#18).
//!
//! The methods only mutate the shared state and wake the poll loop — they never
//! touch the screen, so the loop stays the single owner of screen access. zbus's
//! blocking connection runs its reactor on its own thread, leaving our
//! synchronous poll loop untouched.

use zbus::blocking::Connection;
use zbus::blocking::connection::Builder;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::interface;

use nightlightd_core::mode::Mode;

use crate::state::{Shared, lock};
use crate::waker::Waker;

const OBJECT_PATH: &str = "/org/nightlightd/Daemon";
const WELL_KNOWN_NAME: &str = "org.nightlightd.Daemon";

/// Backs the D-Bus interface: a handle to the shared state and the waker.
struct Daemon {
    state: Shared,
    waker: Waker,
}

#[interface(name = "org.nightlightd.Daemon")]
impl Daemon {
    /// Pin a manual temperature (kelvin) and turn the filter on.
    fn set_temperature(&self, kelvin: u32) {
        {
            let mut state = lock(&self.state);
            state.override_temp = Some(kelvin);
            state.enabled = true;
        }
        self.waker.wake();
    }

    /// Turn the filter on or off (idempotent).
    fn set_enabled(&self, enabled: bool) {
        lock(&self.state).enabled = enabled;
        self.waker.wake();
    }

    /// Flip the filter on/off.
    fn toggle(&self) {
        {
            let mut state = lock(&self.state);
            state.enabled = !state.enabled;
        }
        self.waker.wake();
    }

    /// Switch mode. `"auto"` clears any manual override and follows the sun.
    fn set_mode(&self, mode: String) {
        {
            let mut state = lock(&self.state);
            if mode == "auto" {
                state.override_temp = None;
                state.mode = Mode::Automatic;
            }
        }
        self.waker.wake();
    }

    /// Report `(enabled, current_temperature)`.
    fn get_status(&self) -> (bool, u32) {
        let state = lock(&self.state);
        (state.enabled, state.current_temp)
    }
}

/// Serves the interface and claims the well-known name as the single-instance
/// lock (#19). Returns `Ok(Some(conn))` when this process owns the name — the
/// connection must be kept alive for the daemon's lifetime — or `Ok(None)` when
/// another instance already owns it, so the caller can exit cleanly.
///
/// `DoNotQueue` makes a second instance fail at once instead of waiting in the
/// bus's queue, and the name is never replaced, so the first daemon keeps it.
pub fn serve(state: Shared, waker: Waker) -> zbus::Result<Option<Connection>> {
    let connection = Builder::session()?
        .serve_at(OBJECT_PATH, Daemon { state, waker })?
        .build()?;
    match connection.request_name_with_flags(WELL_KNOWN_NAME, RequestNameFlags::DoNotQueue.into()) {
        Ok(RequestNameReply::PrimaryOwner) => Ok(Some(connection)),
        // Another instance already owns the name. With DoNotQueue, zbus reports
        // this either as a non-primary reply or as the NameTaken error.
        Ok(_) | Err(zbus::Error::NameTaken) => Ok(None),
        Err(other) => Err(other),
    }
}
