//! Talking to the running daemon over D-Bus.
//!
//! The contract is the D-Bus interface `org.nightlightd.Daemon`, not any Rust
//! type — so this re-declares the proxy and a matching `Status` rather than
//! depending on `cli`. A third-party client would do exactly the same. The cost
//! is that if the interface ever changes, the tray breaks; it degrades quietly
//! when that or a missing daemon happens.

use nightlightd_core::transition::{Band, phase};
use serde::Deserialize;
use zbus::proxy;
use zbus::zvariant::Type;

/// A snapshot from the daemon. Field order must match `GetStatus`'s return on
/// the wire (`cli`'s `status::Status`); the names here are ours. Every field is
/// part of that layout, so all must stay even though the tray reads only some —
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
    pub day_temp: u32,
    pub night_temp: u32,
    pub gamma: f64,
    pub brightness: f64,
    pub day_brightness: f64,
    pub night_brightness: f64,
}

impl Status {
    /// A few lines for the hover tooltip: on/off and the applied temperature,
    /// what is driving it, and — when a location is known — where the sun is.
    /// This is the tray's version of `--status`. `band` names the sun's phase;
    /// the tray used to hold its own +3 and -6, which meant a configured band
    /// (#39) got a tooltip saying "night" over a screen still warming.
    pub fn describe(&self, band: Band) -> String {
        let onoff = if self.enabled { "on" } else { "off" };
        let mut text = format!("{onoff} · {} K\n{}", self.temperature, self.source);
        if self.has_location {
            text.push_str(&format!(
                "\nsun {:+.1}° ({}) at {:.1}°, {:.1}°",
                self.elevation,
                phase(self.elevation, band),
                self.latitude,
                self.longitude,
            ));
        }
        text
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
    fn toggle(&self) -> zbus::Result<()>;
    fn set_enabled(&self, enabled: bool) -> zbus::Result<()>;
    fn set_temperature(&self, kelvin: u32) -> zbus::Result<()>;
    fn set_mode(&self, mode: &str) -> zbus::Result<()>;
    fn set_fade(&self, fade: bool) -> zbus::Result<()>;
    fn get_fade(&self) -> zbus::Result<bool>;
    fn get_transition_band(&self) -> zbus::Result<(f64, f64)>;
}

/// A live handle to the daemon: the session-bus connection plus a proxy.
pub struct Client {
    /// Kept alongside the proxy: the single-instance name lives on it.
    connection: zbus::blocking::Connection,
    proxy: DaemonProxyBlocking<'static>,
}

impl Client {
    /// Connects to the session bus and builds the proxy. This succeeds even
    /// when the daemon is not running — the bus is what must be present; calls
    /// then fail per-request, which [`status`](Self::status) reports as `None`.
    pub fn connect() -> zbus::Result<Self> {
        let connection = zbus::blocking::Connection::session()?;
        let proxy = DaemonProxyBlocking::new(&connection)?;
        Ok(Self { connection, proxy })
    }

    /// Whether something owns the daemon's bus name right now. Asked only
    /// after a failed status read (#42): owned but unreadable means this
    /// tray and the daemon are different versions, which deserves a
    /// different tooltip than "not running".
    pub fn daemon_on_bus(&self) -> bool {
        zbus::blocking::fdo::DBusProxy::new(&self.connection)
            .ok()
            .and_then(|fdo| {
                let name = zbus::names::BusName::try_from("org.nightlightd.Daemon").ok()?;
                fdo.name_has_owner(name).ok()
            })
            .unwrap_or(false)
    }

    /// Claims the tray's own well-known name as a single-instance lock,
    /// the daemon's #19 medicine applied to the tray (GitHub #1): on
    /// distros where the session bus is the per-user bus, a tray from the
    /// previous login survives logout (it holds no X connection), and every
    /// new login's autostart would add another icon. Returns `false` when
    /// another tray already owns the name, so the caller can exit quietly.
    pub fn claim_tray_name(&self) -> zbus::Result<bool> {
        match self.connection.request_name_with_flags(
            "org.nightlightd.Tray",
            zbus::fdo::RequestNameFlags::DoNotQueue.into(),
        ) {
            Ok(zbus::fdo::RequestNameReply::PrimaryOwner) => Ok(true),
            Ok(_) | Err(zbus::Error::NameTaken) => Ok(false),
            Err(other) => Err(other),
        }
    }

    /// The transition band (#39) the daemon is running, already `sane()`d so
    /// the tooltip names the sun's phase by the band actually applied. Falls
    /// back to the default against a daemon too old to answer.
    pub fn band(&self) -> Band {
        self.proxy
            .get_transition_band()
            .map(|(day, night)| {
                Band {
                    day_elevation: day,
                    night_elevation: night,
                }
                .sane()
            })
            .unwrap_or_default()
    }

    /// The current status, or `None` when the daemon cannot be reached.
    pub fn status(&self) -> Option<Status> {
        self.proxy.get_status().ok()
    }

    /// Flips the filter on or off. Errors (a stopped daemon) are swallowed —
    /// the click must never crash the tray; the next status read shows the
    /// real state.
    pub fn toggle(&self) {
        let _ = self.proxy.toggle();
    }

    /// Returns to following the sun. The daemon's "auto" clears the override
    /// and turns the filter on itself, so one call carries the whole intent.
    pub fn follow_the_sun(&self) {
        let _ = self.proxy.set_mode("auto");
    }

    /// Turns the fade walk (#44) on or off. Errors swallowed.
    pub fn set_fade(&self, fade: bool) {
        let _ = self.proxy.set_fade(fade);
    }

    /// Whether the fade walk is on, or `None` when the daemon is unreachable
    /// or too old to know `GetFade` — the caller then shows no fade item at
    /// all rather than a checkbox that lies.
    pub fn fade(&self) -> Option<bool> {
        self.proxy.get_fade().ok()
    }

    /// Pins `kelvin`, freezing the screen there and leaving the sun — what
    /// unticking "Automatic" does. Errors are swallowed like the rest.
    pub fn hold(&self, kelvin: u32) {
        let _ = self.proxy.set_temperature(kelvin);
    }

    /// Turns the filter on or off explicitly, so a menu item does what its
    /// label showed rather than blindly flipping. Errors swallowed.
    pub fn set_enabled(&self, enabled: bool) {
        let _ = self.proxy.set_enabled(enabled);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Must match the daemon's `cli/src/status.rs` (and the panel's copy) —
    /// see the pin there. A mismatch makes get_status fail while writes still
    /// work, which reads as "daemon not running" with a working left-click.
    #[test]
    fn wire_signature_is_pinned() {
        assert_eq!(Status::SIGNATURE.to_string(), "(busdbddbuudddd)");
    }

    fn somewhere_at(elevation: f64) -> Status {
        Status {
            enabled: true,
            temperature: 4000,
            source: "auto".into(),
            elevation,
            has_location: true,
            latitude: 41.02,
            longitude: 28.97,
            following: true,
            day_temp: 6500,
            night_temp: 1700,
            gamma: 1.0,
            brightness: 1.0,
            day_brightness: 1.0,
            night_brightness: 1.0,
        }
    }

    /// The tooltip used to carry its own +3 and -6, so a configured band
    /// (#39) got a word that described a different program. One elevation,
    /// three bands, three different truths.
    #[test]
    fn the_tooltip_names_the_phase_by_the_configured_band() {
        let dusk = somewhere_at(-6.5);
        assert!(dusk.describe(Band::default()).contains("(night)"));
        let deep = Band {
            day_elevation: 3.0,
            night_elevation: -14.0,
        };
        assert!(dusk.describe(deep).contains("(transition)"));
        let eager = Band {
            day_elevation: -7.0,
            night_elevation: -8.0,
        };
        assert!(dusk.describe(eager).contains("(day)"));
    }

    /// Without a location there is no sun to describe, and no phase word to
    /// get wrong — the tooltip stays two lines.
    #[test]
    fn a_placeless_tooltip_says_nothing_about_the_sun() {
        let mut status = somewhere_at(-6.5);
        status.has_location = false;
        let text = status.describe(Band::default());
        assert!(!text.contains("sun"));
        assert_eq!(text.lines().count(), 2);
    }
}
