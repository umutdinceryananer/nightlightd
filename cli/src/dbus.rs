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

use nightlightd_core::location::location_from_timezone;
use nightlightd_core::mode::Mode;
use nightlightd_core::solar::solar_elevation;

use crate::config::{self, Config};
use crate::state::{Shared, State, lock};
use crate::status::Status;
use crate::waker::Waker;
use crate::x11::unix_now;

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

    /// Set the daytime target temperature (kelvin) — the top of the automatic
    /// curve — then persist it and re-apply.
    fn set_day_temp(&self, kelvin: u32) {
        lock(&self.state).day_temp = kelvin;
        persist(&self.state);
        self.waker.wake();
    }

    /// Set the night target temperature (kelvin) — the bottom of the automatic
    /// curve — then persist it and re-apply.
    fn set_night_temp(&self, kelvin: u32) {
        lock(&self.state).night_temp = kelvin;
        persist(&self.state);
        self.waker.wake();
    }

    /// Report a full snapshot: on/off, the applied temperature, what is driving
    /// it, and where the sun is now (a check that the clock and timezone agree
    /// with reality).
    fn get_status(&self) -> Status {
        let state = lock(&self.state);
        let source = describe_source(&state);
        // Actively tracking the sun: on, no manual override, not a fixed temp.
        let following =
            state.enabled && state.override_temp.is_none() && !matches!(state.mode, Mode::Fixed(_));
        match location_of(state.mode) {
            Some((latitude, longitude)) => Status {
                enabled: state.enabled,
                temperature: state.current_temp,
                source,
                elevation: solar_elevation(latitude, longitude, unix_now()),
                has_location: true,
                latitude,
                longitude,
                following,
                day_temp: state.day_temp,
                night_temp: state.night_temp,
            },
            None => Status {
                enabled: state.enabled,
                temperature: state.current_temp,
                source,
                elevation: 0.0,
                has_location: false,
                latitude: 0.0,
                longitude: 0.0,
                following,
                day_temp: state.day_temp,
                night_temp: state.night_temp,
            },
        }
    }
}

/// Writes the current day/night temperatures (and any manual location) back to
/// the config file so a settings change survives a restart. A write failure is
/// logged, not fatal.
fn persist(state: &Shared) {
    let config = {
        let state = lock(state);
        let (latitude, longitude) = match state.mode {
            Mode::ManualLocation { lat, lon } => (Some(lat), Some(lon)),
            _ => (None, None),
        };
        Config {
            day_temp: state.day_temp,
            night_temp: state.night_temp,
            latitude,
            longitude,
        }
    };
    if let Err(error) = config::save(&config) {
        tracing::warn!("could not save config: {error}");
    }
}

/// Describes what is currently driving the temperature, for the status readout:
/// off wins over a manual override, which wins over the sun-following mode.
fn describe_source(state: &State) -> String {
    if !state.enabled {
        "off (screen left neutral)".to_string()
    } else if let Some(kelvin) = state.override_temp {
        format!("manual override ({kelvin} K)")
    } else {
        match state.mode {
            Mode::Automatic => "auto (following the sun)".to_string(),
            Mode::ManualLocation { .. } => "auto (manual location)".to_string(),
            Mode::Fixed(kelvin) => format!("fixed ({kelvin} K)"),
        }
    }
}

/// The location the daemon would use for the sun, if any: the given coordinates
/// in manual mode, the timezone's coordinate in automatic mode, and nothing in
/// fixed mode (where the sun is not consulted).
fn location_of(mode: Mode) -> Option<(f64, f64)> {
    match mode {
        Mode::Fixed(_) => None,
        Mode::ManualLocation { lat, lon } => Some((lat, lon)),
        Mode::Automatic => location_from_timezone(),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn state(enabled: bool, override_temp: Option<u32>, mode: Mode) -> State {
        State {
            enabled,
            override_temp,
            mode,
            day_temp: 6500,
            night_temp: 3500,
            current_temp: 6500,
        }
    }

    #[test]
    fn source_off_wins_over_everything() {
        let s = state(false, Some(2800), Mode::Automatic);
        assert_eq!(describe_source(&s), "off (screen left neutral)");
    }

    #[test]
    fn source_override_wins_over_mode() {
        let s = state(true, Some(2800), Mode::Automatic);
        assert_eq!(describe_source(&s), "manual override (2800 K)");
    }

    #[test]
    fn source_describes_each_mode() {
        assert_eq!(
            describe_source(&state(true, None, Mode::Automatic)),
            "auto (following the sun)"
        );
        assert_eq!(
            describe_source(&state(true, None, Mode::Fixed(2800))),
            "fixed (2800 K)"
        );
    }

    #[test]
    fn location_is_none_for_fixed_mode() {
        assert_eq!(location_of(Mode::Fixed(2800)), None);
        assert_eq!(
            location_of(Mode::ManualLocation {
                lat: 39.93,
                lon: 32.85
            }),
            Some((39.93, 32.85))
        );
    }
}
