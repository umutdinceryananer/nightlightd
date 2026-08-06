//! The D-Bus client: a short-lived invocation that messages a running daemon
//! (#20). No `--daemon` flag means the binary acts as a client.

use zbus::blocking::Connection;
use zbus::proxy;

use crate::status::Status;

/// A blocking proxy for the daemon's interface. The `proxy` macro generates
/// `DaemonProxyBlocking` from these method signatures.
#[proxy(
    interface = "org.nightlightd.Daemon",
    default_service = "org.nightlightd.Daemon",
    default_path = "/org/nightlightd/Daemon"
)]
trait Daemon {
    fn set_temperature(&self, kelvin: u32) -> zbus::Result<()>;
    fn set_enabled(&self, enabled: bool) -> zbus::Result<()>;
    fn toggle(&self) -> zbus::Result<()>;
    fn set_mode(&self, mode: &str) -> zbus::Result<()>;
    fn set_gamma(&self, gamma: f64) -> zbus::Result<()>;
    fn set_fade(&self, fade: bool) -> zbus::Result<()>;
    fn get_fade(&self) -> zbus::Result<bool>;
    fn set_transition_band(&self, day_elevation: f64, night_elevation: f64) -> zbus::Result<()>;
    fn get_transition_band(&self) -> zbus::Result<(f64, f64)>;
    fn set_brightness(&self, day: f64, night: f64) -> zbus::Result<()>;
    fn get_status(&self) -> zbus::Result<Status>;
}

/// What the client was asked to do.
pub enum Request {
    /// Pin a fixed temperature (kelvin).
    SetTemperature(u32),
    /// Flip the filter on/off.
    Toggle,
    /// Turn the filter on or off.
    SetEnabled(bool),
    /// Return to following the sun.
    Auto,
    /// Set the gamma exponent (GitHub #2).
    SetGamma(f64),
    /// Set the day and night brightness bounds (GitHub #2).
    SetBrightness(f64, f64),
    /// Turn the fade walk on or off (#44).
    SetFade(bool),
    /// Set the transition band's elevation bounds (#39).
    SetBand(f64, f64),
    /// Print the daemon's status.
    Status,
}

/// Whether something owns the daemon's bus name right now. Asked only after
/// a failed request (#42): owned but failing means this binary and the
/// daemon are different versions, which deserves a different error than
/// "not running".
pub fn daemon_on_bus() -> bool {
    Connection::session()
        .ok()
        .and_then(|connection| zbus::blocking::fdo::DBusProxy::new(&connection).ok())
        .and_then(|fdo| {
            let name = zbus::names::BusName::try_from("org.nightlightd.Daemon").ok()?;
            fdo.name_has_owner(name).ok()
        })
        .unwrap_or(false)
}

/// Sends `request` to the running daemon over the session bus.
pub fn send(request: Request) -> zbus::Result<()> {
    let connection = Connection::session()?;
    let proxy = DaemonProxyBlocking::new(&connection)?;
    match request {
        Request::SetTemperature(kelvin) => proxy.set_temperature(kelvin),
        Request::Toggle => proxy.toggle(),
        Request::SetEnabled(enabled) => proxy.set_enabled(enabled),
        Request::Auto => proxy.set_mode("auto"),
        Request::SetGamma(gamma) => proxy.set_gamma(gamma),
        Request::SetBrightness(day, night) => proxy.set_brightness(day, night),
        Request::SetFade(fade) => proxy.set_fade(fade),
        Request::SetBand(day, night) => proxy.set_transition_band(day, night),
        Request::Status => {
            // A daemon too old to know GetFade means the fade is not there
            // to report; defaulting to "on" keeps the line silent. Same for
            // the band: the default earns no ink.
            let fade = proxy.get_fade().unwrap_or(true);
            let band = proxy.get_transition_band().unwrap_or((3.0, -6.0));
            print_status(&proxy.get_status()?, fade, band);
            Ok(())
        }
    }
}

/// Prints the daemon snapshot: the headline on the first line, then the details
/// worth eyeballing indented under it.
fn print_status(status: &Status, fade: bool, band: (f64, f64)) {
    let onoff = if status.enabled { "on" } else { "off" };
    println!("nightlightd: {onoff}, {} K", status.temperature);
    println!("  source: {}", status.source);
    // The shaping factors earn a line only when they do something.
    if (status.gamma - 1.0).abs() > 1e-9 || (status.brightness - 1.0).abs() > 1e-9 {
        println!(
            "  ramp:   gamma {:.2}, brightness {:.2} (day {:.2} / night {:.2})",
            status.gamma, status.brightness, status.day_brightness, status.night_brightness
        );
    }
    // Same rule as the ramp line: the default earns no ink.
    if !fade {
        println!("  fade:   off");
    }
    if (band.0 - 3.0).abs() > 1e-9 || (band.1 + 6.0).abs() > 1e-9 {
        println!(
            "  band:   day above {:+.1}°, night below {:+.1}°",
            band.0, band.1
        );
    }
    if status.has_location {
        println!(
            "  sun:    {:+.1}° ({})",
            status.elevation,
            sun_phase(status.elevation)
        );
        println!(
            "  place:  {:.2}, {:.2} (resolved)",
            status.latitude, status.longitude
        );
    }
}

/// Names the part of the day for a solar elevation, matching the transition
/// curve's thresholds (full day at +3°, full night at -6°).
fn sun_phase(elevation: f64) -> &'static str {
    if elevation >= 3.0 {
        "day"
    } else if elevation <= -6.0 {
        "night"
    } else {
        "transition"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sun_phase_names_each_band() {
        assert_eq!(sun_phase(45.0), "day");
        assert_eq!(sun_phase(3.0), "day"); // exact endpoint
        assert_eq!(sun_phase(0.0), "transition");
        assert_eq!(sun_phase(-6.0), "night"); // exact endpoint
        assert_eq!(sun_phase(-20.0), "night");
    }
}
