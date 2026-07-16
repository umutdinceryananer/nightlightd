//! Talking to the daemon over D-Bus.
//!
//! Like the tray, the panel re-declares the slice of `org.nightlightd.Daemon`
//! it uses rather than sharing a Rust type: the contract is the interface. It
//! reads the status (to keep the slider in step with automatic mode) and sends
//! a temperature when the user drags.

use serde::Deserialize;
use zbus::blocking::Connection;
use zbus::proxy;
use zbus::zvariant::Type;

/// A snapshot from the daemon. Field order must match `GetStatus`'s wire layout
/// (`cli`'s `status::Status`); the panel only reads `temperature` and
/// `following`, but every field is part of that layout, so all must stay —
/// hence `allow(dead_code)`.
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
}

/// The slice of the daemon interface the panel uses. `zbus::proxy` generates
/// `DaemonProxyBlocking` from these signatures.
#[proxy(
    interface = "org.nightlightd.Daemon",
    default_service = "org.nightlightd.Daemon",
    default_path = "/org/nightlightd/Daemon"
)]
trait Daemon {
    fn get_status(&self) -> zbus::Result<Status>;
    fn set_temperature(&self, kelvin: u32) -> zbus::Result<()>;
    fn set_enabled(&self, enabled: bool) -> zbus::Result<()>;
    fn set_mode(&self, mode: &str) -> zbus::Result<()>;
}

/// A live handle to the daemon: the session-bus connection plus a proxy.
pub struct Client {
    proxy: DaemonProxyBlocking<'static>,
}

impl Client {
    /// Connects to the session bus and builds the proxy. Succeeds even when the
    /// daemon is not running — the bus is what must exist; calls then fail
    /// per-request and are reported as `None` / swallowed.
    pub fn connect() -> zbus::Result<Self> {
        let connection = Connection::session()?;
        let proxy = DaemonProxyBlocking::new(&connection)?;
        Ok(Self { proxy })
    }

    /// The current status, or `None` when the daemon cannot be reached.
    pub fn status(&self) -> Option<Status> {
        self.proxy.get_status().ok()
    }

    /// Pins a manual temperature and turns the filter on. Errors (a stopped
    /// daemon) are swallowed — dragging the slider must never crash the panel.
    pub fn set_temperature(&self, kelvin: u32) {
        let _ = self.proxy.set_temperature(kelvin);
    }

    /// Hands control back to the sun: on, and following again.
    pub fn follow_the_sun(&self) {
        let _ = self.proxy.set_enabled(true);
        let _ = self.proxy.set_mode("auto");
    }
}
