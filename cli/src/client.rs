//! The D-Bus client: a short-lived invocation that messages a running daemon
//! (#20). No `--daemon` flag means the binary acts as a client.

use zbus::blocking::Connection;
use zbus::proxy;

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
    fn get_status(&self) -> zbus::Result<(bool, u32)>;
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
            let (enabled, temperature) = proxy.get_status()?;
            let state = if enabled { "on" } else { "off" };
            println!("nightlightd: {state}, {temperature} K");
            Ok(())
        }
    }
}
