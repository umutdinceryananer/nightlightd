//! Talking to the daemon over D-Bus.
//!
//! Like the tray and the panel, the TUI re-declares the slice of
//! `org.nightlightd.Daemon` it uses: the contract is the interface, not a Rust
//! type. It reads the status and sends the few controls the keybindings need.

use serde::Deserialize;
use zbus::blocking::Connection;
use zbus::proxy;
use zbus::zvariant::Type;

/// A snapshot from the daemon. Field order must match `GetStatus`'s wire
/// layout (`cli`'s `status::Status`); every field is part of that layout, so
/// all must stay even though the TUI reads only some — hence
/// `allow(dead_code)`.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize, Type)]
pub struct Status {
    pub enabled: bool,
    pub temperature: u32,
    pub source: String,
    pub elevation: f64,
    pub has_location: bool,
    pub latitude: f64,
    pub longitude: f64,
    pub following: bool,
    pub day_temp: u32,
    pub night_temp: u32,
    pub gamma: f64,
    pub brightness: f64,
    pub day_brightness: f64,
    pub night_brightness: f64,
}

/// The slice of the daemon interface the TUI uses. `zbus::proxy` generates
/// `DaemonProxyBlocking` from these signatures.
#[proxy(
    interface = "org.nightlightd.Daemon",
    default_service = "org.nightlightd.Daemon",
    default_path = "/org/nightlightd/Daemon"
)]
trait Daemon {
    fn get_status(&self) -> zbus::Result<Status>;
    fn toggle(&self) -> zbus::Result<()>;
    fn set_mode(&self, mode: &str) -> zbus::Result<()>;
    fn set_day_temp(&self, kelvin: u32) -> zbus::Result<()>;
    fn set_night_temp(&self, kelvin: u32) -> zbus::Result<()>;
    fn set_gamma(&self, gamma: f64) -> zbus::Result<()>;
    fn set_brightness(&self, day: f64, night: f64) -> zbus::Result<()>;
    fn set_fade(&self, fade: bool) -> zbus::Result<()>;
    fn get_fade(&self) -> zbus::Result<bool>;
    fn set_transition_band(&self, day: f64, night: f64) -> zbus::Result<()>;
    fn get_transition_band(&self) -> zbus::Result<(f64, f64)>;
    fn set_location(&self, latitude: f64, longitude: f64) -> zbus::Result<()>;
    fn clear_location(&self) -> zbus::Result<()>;
    fn get_outputs(&self) -> zbus::Result<Vec<(u32, u16)>>;
}

/// A live handle to the daemon: the session-bus connection plus a proxy.
pub struct Client {
    proxy: DaemonProxyBlocking<'static>,
    /// The bus's own interface, for one question: does anything own the
    /// daemon's name (#42)?
    fdo: zbus::blocking::fdo::DBusProxy<'static>,
}

impl Client {
    /// Connects to the session bus and builds the proxy. Succeeds even when
    /// the daemon is not running — the bus is what must exist; calls then fail
    /// per-request and are reported as `None` / swallowed.
    pub fn connect() -> zbus::Result<Self> {
        let connection = Connection::session()?;
        let proxy = DaemonProxyBlocking::new(&connection)?;
        let fdo = zbus::blocking::fdo::DBusProxy::new(&connection)?;
        Ok(Self { proxy, fdo })
    }

    /// Whether something owns the daemon's bus name right now. Asked only
    /// after a failed status read (#42): owned but unreadable means this
    /// client and the daemon are different versions, which deserves a
    /// different message than "not running".
    pub fn daemon_on_bus(&self) -> bool {
        zbus::names::BusName::try_from("org.nightlightd.Daemon")
            .ok()
            .and_then(|name| self.fdo.name_has_owner(name).ok())
            .unwrap_or(false)
    }

    /// The current status, or `None` when the daemon cannot be reached.
    pub fn status(&self) -> Option<Status> {
        self.proxy.get_status().ok()
    }

    /// Flips the filter on or off. Errors (a stopped daemon) are swallowed —
    /// a keypress must never crash the TUI.
    pub fn toggle(&self) {
        let _ = self.proxy.toggle();
    }

    /// Returns to following the sun. The daemon's "auto" clears the override
    /// and turns the filter on itself.
    pub fn follow_the_sun(&self) {
        let _ = self.proxy.set_mode("auto");
    }

    /// Sets the daytime target temperature; persisted by the daemon (which
    /// also keeps the band ordered). Errors swallowed.
    pub fn set_day_temp(&self, kelvin: u32) {
        let _ = self.proxy.set_day_temp(kelvin);
    }

    /// Sets the night target temperature; persisted by the daemon (which also
    /// keeps the band ordered). Errors swallowed.
    pub fn set_night_temp(&self, kelvin: u32) {
        let _ = self.proxy.set_night_temp(kelvin);
    }

    /// Sets the gamma exponent (GitHub #2); the daemon clamps and persists.
    pub fn set_gamma(&self, gamma: f64) {
        let _ = self.proxy.set_gamma(gamma);
    }

    /// Sets the brightness bounds; the daemon clamps and persists them.
    pub fn set_brightness(&self, day: f64, night: f64) {
        let _ = self.proxy.set_brightness(day, night);
    }

    /// Turns the fade walk (#44) on or off; the daemon persists it.
    pub fn set_fade(&self, fade: bool) {
        let _ = self.proxy.set_fade(fade);
    }

    /// Whether the fade walk is on, or `None` when the daemon is unreachable
    /// or predates `GetFade` — the settings row then shows a dash and the
    /// toggle does nothing.
    pub fn fade(&self) -> Option<bool> {
        self.proxy.get_fade().ok()
    }

    /// The transition band (#39), or `None` against a daemon that is
    /// unreachable or predates `GetTransitionBand` — the drawings then fall
    /// back to the default band.
    /// Sets the transition band (#45); the daemon carries the pair verbatim
    /// and persists it. Errors swallowed.
    pub fn set_band(&self, day: f64, night: f64) {
        let _ = self.proxy.set_transition_band(day, night);
    }

    pub fn band(&self) -> Option<(f64, f64)> {
        self.proxy.get_transition_band().ok()
    }

    /// Pins a manual location (degrees); the daemon clamps and persists it.
    pub fn set_location(&self, latitude: f64, longitude: f64) {
        let _ = self.proxy.set_location(latitude, longitude);
    }

    /// Returns to deriving the location from the timezone.
    pub fn clear_location(&self) {
        let _ = self.proxy.clear_location();
    }

    /// The active outputs as `(crtc, gamma_ramp_size)`, or `None` when the
    /// daemon cannot be reached.
    pub fn outputs(&self) -> Option<Vec<(u32, u16)>> {
        self.proxy.get_outputs().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Must match the daemon's `cli/src/status.rs` (and the tray's and the
    /// panel's copies) — see the pin there. A mismatch makes get_status fail
    /// while writes still work.
    #[test]
    fn wire_signature_is_pinned() {
        assert_eq!(Status::SIGNATURE.to_string(), "(busdbddbuudddd)");
    }
}
