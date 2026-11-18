//! Single instance: the panel claims a D-Bus name the way the daemon does
//! (#19), so a second "Settings…" click never opens a second window. If a panel
//! is already running, the new process asks it to raise itself and then exits.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nightlightd_core::bus::{self, Service};
use zbus::blocking::Connection;
use zbus::blocking::connection::Builder;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::{interface, proxy};

const PATH: &str = "/org/nightlightd/Panel";

/// Serves `Present`: sets a flag the egui loop watches so it raises the window.
struct Presenter {
    focus: Arc<AtomicBool>,
}

#[interface(name = "org.nightlightd.Panel")]
impl Presenter {
    /// Asks the running panel to come to the front.
    fn present(&self) {
        self.focus.store(true, Ordering::Relaxed);
    }
}

/// A blocking proxy for an already-running panel's `Present`.
///
/// Handed its bus name rather than holding one, for the reason the daemon's
/// proxies are (#55).
#[proxy(
    interface = "org.nightlightd.Panel",
    default_path = "/org/nightlightd/Panel"
)]
trait Panel {
    fn present(&self) -> zbus::Result<()>;
}

/// Claims the panel name. Returns `Some(conn)` when this is the only panel —
/// keep the connection alive for the process's lifetime — or `None` when one is
/// already running, in which case that one is asked to raise itself and this
/// process should exit. `focus` is set (on the owner) whenever `Present` is
/// called; the egui loop clears it and focuses the window.
pub fn acquire(focus: Arc<AtomicBool>) -> Option<Connection> {
    let connection = Builder::session()
        .ok()?
        .serve_at(PATH, Presenter { focus })
        .ok()?
        .build()
        .ok()?;
    // Below the app ID inside a Flatpak, org.nightlightd natively (#55).
    let name = bus::name(Service::Panel);
    match connection.request_name_with_flags(name.as_str(), RequestNameFlags::DoNotQueue.into()) {
        Ok(RequestNameReply::PrimaryOwner) => Some(connection),
        _ => {
            // Another panel owns the name: ask it to come forward, then bow out.
            if let Ok(proxy) = PanelProxyBlocking::new(&connection, name) {
                let _ = proxy.present();
            }
            None
        }
    }
}
