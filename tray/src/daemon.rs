//! Talking to the running daemon over D-Bus.
//!
//! The contract is the D-Bus interface `org.nightlightd.Daemon`, not any Rust
//! type — so this re-declares the proxy and a matching `Status` rather than
//! depending on `cli`. A third-party client would do exactly the same. The cost
//! is that if the interface ever changes, the tray breaks; it degrades quietly
//! when that or a missing daemon happens.

use serde::Deserialize;
use zbus::proxy;
use zbus::zvariant::Type;

/// A snapshot from the daemon. Field order must match `GetStatus`'s return on
/// the wire (`cli`'s `status::Status`); the names here are ours. Every field is
/// part of that layout, so all of them must stay even if unused here.
#[derive(Debug, Clone, Deserialize, Type)]
pub struct Status {
    pub enabled: bool,
    pub temperature: u32,
    pub source: String,
    pub elevation: f64,
    pub has_location: bool,
    pub latitude: f64,
    pub longitude: f64,
}

impl Status {
    /// A few lines for the hover tooltip: on/off and the applied temperature,
    /// what is driving it, and — when a location is known — where the sun is.
    /// This is the tray's version of `--status`.
    pub fn describe(&self) -> String {
        let onoff = if self.enabled { "on" } else { "off" };
        let mut text = format!("{onoff} · {} K\n{}", self.temperature, self.source);
        if self.has_location {
            text.push_str(&format!(
                "\nsun {:+.1}° ({}) at {:.1}°, {:.1}°",
                self.elevation,
                sun_phase(self.elevation),
                self.latitude,
                self.longitude,
            ));
        }
        text
    }
}

/// Names the part of the day for a solar elevation, matching the daemon's
/// transition thresholds (full day at +3°, full night at -6°).
fn sun_phase(elevation: f64) -> &'static str {
    if elevation >= 3.0 {
        "day"
    } else if elevation <= -6.0 {
        "night"
    } else {
        "transition"
    }
}

/// The slice of the daemon interface the tray uses. `zbus::proxy` generates
/// `DaemonProxyBlocking` from these signatures.
#[proxy(
    interface = "org.nightlightd.Daemon",
    default_service = "org.nightlightd.Daemon",
    default_path = "/org/nightlightd/Daemon"
)]
trait Daemon {
    fn get_status(&self) -> zbus::Result<Status>;
}

/// A live handle to the daemon: the session-bus connection plus a proxy.
pub struct Client {
    proxy: DaemonProxyBlocking<'static>,
}

impl Client {
    /// Connects to the session bus and builds the proxy. This succeeds even
    /// when the daemon is not running — the bus is what must be present; calls
    /// then fail per-request, which [`status`](Self::status) reports as `None`.
    pub fn connect() -> zbus::Result<Self> {
        let connection = zbus::blocking::Connection::session()?;
        let proxy = DaemonProxyBlocking::new(&connection)?;
        Ok(Self { proxy })
    }

    /// The current status, or `None` when the daemon cannot be reached.
    pub fn status(&self) -> Option<Status> {
        self.proxy.get_status().ok()
    }
}
