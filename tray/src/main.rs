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

use ksni::blocking::TrayMethods;

/// The tray icon. Holds nothing yet: this step only makes it appear.
struct NightLight;

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
}

fn main() {
    // The handle must stay alive: dropping it takes the icon away.
    let _handle = match NightLight.spawn() {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!("nightlight-tray: cannot show the tray icon: {error}");
            std::process::exit(1);
        }
    };

    // ksni serves the icon from its own thread; keep this one alive.
    loop {
        std::thread::park();
    }
}
