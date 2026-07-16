//! The tray icon (#23): an icon next to the clock, for people who will not
//! open a terminal.
//!
//! It is a *thin client* and holds no state of its own — the daemon owns
//! everything; this only asks and displays. It is a separate binary in a
//! separate crate on purpose: the daemon must never link the GUI stack, so if
//! the tray dies, the filter lives.
//!
//! Speaks StatusNotifierItem (SNI) over D-Bus via `ksni`, which XFCE's own
//! systray already hosts — no panel plugin for the user to install.

mod daemon;

use std::time::Duration;

use ksni::blocking::TrayMethods;

use crate::daemon::Status;

/// How often to re-read the daemon's status for the hover text. The temperature
/// only moves once a minute, so a few seconds keeps the tooltip fresh without
/// busy-polling.
const REFRESH: Duration = Duration::from_secs(5);

/// The tray icon. Holds only the last status it read — the daemon is the source
/// of truth; `None` means it could not be reached.
struct NightLight {
    status: Option<Status>,
}

impl ksni::Tray for NightLight {
    fn id(&self) -> String {
        env!("CARGO_PKG_NAME").into()
    }

    fn title(&self) -> String {
        "nightlightd".into()
    }

    /// A themed icon name rather than a bundled image, so the panel draws it in
    /// its own style. `night-light-symbolic` is Adwaita's, and the mainstream
    /// themes inherit from Adwaita. If some theme lacks it the icon goes blank,
    /// in which case we ship our own pixmap instead.
    fn icon_name(&self) -> String {
        "night-light-symbolic".into()
    }

    /// The hover text: on/off and the applied temperature, then what is driving
    /// it. Says so plainly when the daemon is not running.
    fn tool_tip(&self) -> ksni::ToolTip {
        let description = match &self.status {
            Some(status) => status.describe(),
            None => "daemon not running".into(),
        };
        ksni::ToolTip {
            title: "nightlightd".into(),
            description,
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
        }
    }
}

fn main() {
    // The bus connection is what must exist; the daemon may come and go, and
    // each read reports that. If even the session bus is absent there is no
    // desktop to draw into, so there is nothing useful to do.
    let client = match daemon::Client::connect() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("nightlight-tray: cannot reach the session bus: {error}");
            std::process::exit(1);
        }
    };

    let handle = match (NightLight {
        status: client.status(),
    })
    .spawn()
    {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!("nightlight-tray: cannot show the tray icon: {error}");
            std::process::exit(1);
        }
    };

    // Re-read the daemon and push the fresh status into the icon. ksni serves
    // the icon from its own thread; this one just keeps it current.
    loop {
        std::thread::sleep(REFRESH);
        let status = client.status();
        handle.update(|tray: &mut NightLight| tray.status = status.clone());
    }
}
