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
    /// Print the daemon's status.
    Status,
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
        Request::Status => {
            print_status(&proxy.get_status()?);
            Ok(())
        }
    }
}

/// Prints the daemon snapshot: the headline on the first line, then the details
/// worth eyeballing indented under it.
fn print_status(status: &Status) {
    let onoff = if status.enabled { "on" } else { "off" };
    println!("nightlightd: {onoff}, {} K", status.temperature);
    println!("  source: {}", status.source);
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
