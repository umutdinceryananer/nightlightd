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

    /// Switch mode. `"auto"` clears any manual override, returns to the
    /// configured sun-following mode (keeping a manual location from the
    /// config), and turns the filter on — "follow the sun" implies there is
    /// something to see, so clients need no compensating SetEnabled call.
    fn set_mode(&self, mode: String) {
        {
            let mut state = lock(&self.state);
            if mode == "auto" {
                state.override_temp = None;
                state.mode = state.configured_mode;
                state.enabled = true;
            }
        }
        self.waker.wake();
    }

    /// Set the daytime target temperature (kelvin) — the top of the automatic
    /// curve — then persist it and re-apply. Never drops below the night
    /// target, so the band stays ordered whatever a client sends.
    fn set_day_temp(&self, kelvin: u32) {
        {
            let mut state = lock(&self.state);
            state.day_temp = kelvin.max(state.night_temp);
        }
        persist(&self.state);
        self.waker.wake();
    }

    /// Set the night target temperature (kelvin) — the bottom of the automatic
    /// curve — then persist it and re-apply. Never rises above the day target.
    fn set_night_temp(&self, kelvin: u32) {
        {
            let mut state = lock(&self.state);
            state.night_temp = kelvin.min(state.day_temp);
        }
        persist(&self.state);
        self.waker.wake();
    }

    /// Pin a manual location (degrees) and persist it; the sun is followed
    /// there from now on, including after a trip through "auto". Out-of-range
    /// values are clamped, not rejected.
    fn set_location(&self, latitude: f64, longitude: f64) {
        {
            let mut state = lock(&self.state);
            let mode = Mode::ManualLocation {
                lat: latitude.clamp(-90.0, 90.0),
                lon: longitude.clamp(-180.0, 180.0),
            };
            state.mode = mode;
            state.configured_mode = mode;
        }
        persist(&self.state);
        self.waker.wake();
    }

    /// Return to deriving the location from the timezone, and persist that
    /// (the saved coordinates are removed from the config).
    fn clear_location(&self) {
        {
            let mut state = lock(&self.state);
            state.mode = Mode::Automatic;
            state.configured_mode = Mode::Automatic;
        }
        persist(&self.state);
        self.waker.wake();
    }

    /// Report a full snapshot: on/off, the applied temperature, what is driving
    /// it, and where the sun is now (a check that the clock and timezone agree
    /// with reality).
    fn get_status(&self) -> Status {
        let mut state = lock(&self.state);
        let source = describe_source(&state);
        // Actively tracking the sun: on, no manual override, not a fixed temp.
        let following =
            state.enabled && state.override_temp.is_none() && !matches!(state.mode, Mode::Fixed(_));
        // The poll loop keeps the automatic-mode cache warm; the fresh lookup
        // only covers a status call arriving before the first apply.
        let location = match state.mode {
            Mode::Fixed(_) => None,
            Mode::ManualLocation { lat, lon } => Some((lat, lon)),
            Mode::Automatic => state.location.or_else(location_from_timezone),
        };
        if matches!(state.mode, Mode::Automatic) {
            state.location = location;
        }
        match location {
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

/// Writes the current day/night temperatures (and any configured manual
/// location) back to the config file so a settings change survives a restart.
/// The coordinates come from `configured_mode`, not the live mode, so a trip
/// through auto never deletes them. A damaged file is never written over —
/// the user's hand-written settings are still in it. A write failure is
/// logged, not fatal.
fn persist(state: &Shared) {
    let config = {
        let state = lock(state);
        if state.config_damaged {
            tracing::warn!(
                "not saving: the config file on disk failed to load; fix it and restart"
            );
            return;
        }
        let (latitude, longitude) = match state.configured_mode {
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
            configured_mode: Mode::Automatic,
            config_damaged: false,
            day_temp: 6500,
            night_temp: 3500,
            current_temp: 6500,
            location: None,
        }
    }

    fn daemon(state: State) -> Daemon {
        use std::sync::{Arc, Mutex};
        Daemon {
            state: Arc::new(Mutex::new(state)),
            waker: crate::waker::waker().expect("eventfd"),
        }
    }

    #[test]
    fn auto_enables_and_restores_the_configured_mode() {
        use std::sync::{Arc, Mutex};
        // A manual-location user who turned the filter off and pinned a temp.
        let configured = Mode::ManualLocation {
            lat: 39.93,
            lon: 32.85,
        };
        let mut s = state(false, Some(2200), Mode::Automatic);
        s.configured_mode = configured;
        let shared = Arc::new(Mutex::new(s));
        let daemon = Daemon {
            state: Arc::clone(&shared),
            waker: crate::waker::waker().expect("eventfd"),
        };
        daemon.set_mode("auto".into());
        let s = lock(&shared);
        // Back on, following the sun, at the *configured* location.
        assert!(s.enabled);
        assert_eq!(s.override_temp, None);
        assert_eq!(s.mode, configured);
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
    fn status_reads_the_cached_location_in_automatic_mode() {
        let mut s = state(true, None, Mode::Automatic);
        s.location = Some((10.0, 20.0));
        let status = daemon(s).get_status();
        assert!(status.has_location);
        assert_eq!((status.latitude, status.longitude), (10.0, 20.0));
    }

    #[test]
    fn status_has_no_location_in_fixed_mode() {
        let status = daemon(state(true, None, Mode::Fixed(2800))).get_status();
        assert!(!status.has_location);
    }

    #[test]
    fn set_location_pins_and_clear_returns_to_timezone() {
        // config_damaged keeps persist() away from the real config file.
        let mut s = state(true, None, Mode::Automatic);
        s.config_damaged = true;
        let d = daemon(s);
        // Out-of-range input is clamped, not rejected.
        d.set_location(95.0, -200.0);
        {
            let s = lock(&d.state);
            let expected = Mode::ManualLocation {
                lat: 90.0,
                lon: -180.0,
            };
            assert_eq!(s.mode, expected);
            assert_eq!(s.configured_mode, expected);
        }
        d.clear_location();
        let s = lock(&d.state);
        assert_eq!(s.mode, Mode::Automatic);
        assert_eq!(s.configured_mode, Mode::Automatic);
    }

    #[test]
    fn day_and_night_bounds_stay_ordered() {
        // Try to invert the band from both ends; the daemon must refuse.
        // config_damaged keeps persist() away from the real config file.
        let mut s = state(true, None, Mode::Automatic);
        s.config_damaged = true;
        let d = daemon(s);
        d.set_night_temp(7000); // above day (6500) -> clamped to 6500
        d.set_day_temp(2000); // below night -> clamped to night
        let s = lock(&d.state);
        assert!(s.day_temp >= s.night_temp);
        assert_eq!(s.night_temp, 6500);
        assert_eq!(s.day_temp, 6500);
    }
}
