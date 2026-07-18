//! The tray icon (#23): an icon next to the clock, for people who will not
//! open a terminal.
//!
//! It is a *thin client* and holds no state of its own beyond the last status
//! it read — the daemon owns everything; this asks, displays, and forwards
//! clicks. It is a separate binary in a separate crate on purpose: the daemon
//! must never link the GUI stack, so if the tray dies, the filter lives.
//!
//! Speaks StatusNotifierItem (SNI) over D-Bus via `ksni`, which XFCE's own
//! systray already hosts — no panel plugin for the user to install.

mod daemon;

use std::time::Duration;

use ksni::blocking::TrayMethods;
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{MenuItem, ToolTip};

use crate::daemon::{Client, Status};

/// How often to re-read the daemon's status. The temperature only moves once a
/// minute, so a few seconds keeps the display fresh without busy-polling.
const REFRESH: Duration = Duration::from_secs(5);

/// The tray icon. Owns the daemon connection and the last status it read; all
/// daemon access happens through here, on ksni's own thread.
struct NightLight {
    client: Client,
    status: Option<Status>,
}

impl NightLight {
    /// Re-reads the daemon and stores the result (`None` when unreachable).
    fn refresh(&mut self) {
        self.status = self.client.status();
    }

    /// Toggles the filter, then refreshes so the icon and tooltip update at
    /// once instead of on the next poll.
    fn toggle(&mut self) {
        self.client.toggle();
        self.refresh();
    }

    /// Returns the daemon to following the sun, then refreshes.
    fn follow_the_sun(&mut self) {
        self.client.follow_the_sun();
        self.refresh();
    }

    /// Freezes the screen at the temperature it shows now, leaving the sun.
    /// Does nothing when the daemon is unreachable (no temperature to hold).
    fn hold(&mut self) {
        if let Some(kelvin) = self.status.as_ref().map(|status| status.temperature) {
            self.client.hold(kelvin);
            self.refresh();
        }
    }

    /// Flips sun-tracking for the "Automatic" checkbox: if it is currently
    /// following, freeze where it is; otherwise resume following the sun.
    /// Refreshes first — the cached status can be 5 s stale, and acting on it
    /// could pin an outdated temperature or re-enable a filter turned off from
    /// another surface moments ago.
    fn toggle_follow(&mut self) {
        self.refresh();
        if self.status.as_ref().is_some_and(|status| status.following) {
            self.hold();
        } else {
            self.follow_the_sun();
        }
    }

    /// Applies the direction the menu label advertised, then refreshes.
    fn set_enabled(&mut self, enabled: bool) {
        self.client.set_enabled(enabled);
        self.refresh();
    }
}

impl ksni::Tray for NightLight {
    fn id(&self) -> String {
        env!("CARGO_PKG_NAME").into()
    }

    fn title(&self) -> String {
        "nightlightd".into()
    }

    /// A themed icon name rather than a bundled image, so the panel draws it in
    /// its own style. Shows the disabled variant when the filter is off (or the
    /// daemon is unreachable), so a left click visibly changes the icon. Both
    /// names are Adwaita's, which the mainstream themes inherit from.
    fn icon_name(&self) -> String {
        let on = self.status.as_ref().is_some_and(|status| status.enabled);
        if on {
            "night-light-symbolic".into()
        } else {
            "night-light-disabled-symbolic".into()
        }
    }

    /// Left click toggles the filter — the one action people want most.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.toggle();
    }

    /// The hover text: the tray's version of `--status`.
    fn tool_tip(&self) -> ToolTip {
        let description = match &self.status {
            Some(status) => status.describe(),
            None => "daemon not running".into(),
        };
        ToolTip {
            title: "nightlightd".into(),
            description,
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
        }
    }

    /// Right click: toggle, return to the sun, and quit. The toggle label
    /// reflects the current state so it reads as an action, not a question.
    fn menu(&self) -> Vec<MenuItem<Self>> {
        let on = self.status.as_ref().is_some_and(|status| status.enabled);
        // The item promises a direction, so send that direction — a blind
        // Toggle against status gone stale would do the opposite of the label.
        let turn_on = !on;
        vec![
            StandardItem {
                label: if turn_on { "Turn on" } else { "Turn off" }.into(),
                activate: Box::new(move |this: &mut Self| this.set_enabled(turn_on)),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Automatic".into(),
                checked: self.status.as_ref().is_some_and(|status| status.following),
                activate: Box::new(|this: &mut Self| this.toggle_follow()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Settings…".into(),
                activate: Box::new(|_| open_panel()),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|_| std::process::exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

/// Launches the settings panel. Looks for `nightlight-panel` next to this
/// binary first (they install together, so this survives an autostart PATH that
/// lacks `~/.cargo/bin`), then falls back to a plain PATH lookup. Errors are
/// swallowed — a failed launch must not take the tray down.
fn open_panel() {
    let beside_us = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("nightlight-panel")));
    let panel = beside_us.unwrap_or_else(|| std::path::PathBuf::from("nightlight-panel"));
    let _ = std::process::Command::new(panel).spawn();
}

fn main() {
    // The bus connection is what must exist; the daemon may come and go, and
    // each read reports that. If even the session bus is absent there is no
    // desktop to draw into, so there is nothing useful to do.
    let client = match Client::connect() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("nightlight-tray: cannot reach the session bus: {error}");
            std::process::exit(1);
        }
    };

    let status = client.status();
    // `assume_sni_available(true)`: at login the tray autostarts before the
    // panel's StatusNotifierWatcher exists, so a plain spawn() fails on the
    // missing watcher and the icon never appears (confirmed in
    // ~/.xsession-errors). With this, ksni treats the absent watcher as a soft
    // error and registers the icon once the panel's tray comes online.
    let handle = match (NightLight { client, status })
        .assume_sni_available(true)
        .spawn()
    {
        Ok(handle) => handle,
        Err(error) => {
            eprintln!("nightlight-tray: cannot show the tray icon: {error}");
            std::process::exit(1);
        }
    };

    // Keep the displayed status fresh. ksni serves the icon from its own
    // thread; the read happens inside `update`, on that thread, so the daemon
    // connection has a single owner.
    loop {
        std::thread::sleep(REFRESH);
        handle.update(NightLight::refresh);
    }
}
