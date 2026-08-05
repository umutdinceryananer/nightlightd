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
    /// Whether the fade walk (#44) is on; `None` against a daemon that is
    /// unreachable or predates `GetFade`, and then no menu item shows.
    fade: Option<bool>,
    /// Set when the status is unreadable but the daemon's name is owned
    /// (#42): the two are different versions, and saying "not running"
    /// would be the lie the first stale tray told.
    mismatch: bool,
}

impl NightLight {
    /// Re-reads the daemon and stores the result (`None` when unreachable).
    fn refresh(&mut self) {
        self.status = self.client.status();
        self.fade = self.client.fade();
        self.mismatch = self.status.is_none() && self.client.daemon_on_bus();
        if self.mismatch {
            // Maybe this process is simply older than the file it came from;
            // one silent relaunch answers that. If it comes back still
            // mismatched, the tooltip and the menu take over.
            relaunch_once();
        }
    }

    /// Asks systemd for a fresh daemon — the explicit, user-clicked recovery
    /// for the one mismatch a client cannot heal itself (#42): a daemon
    /// still running its pre-update binary. Harmless when the disk is just
    /// as old; the same daemon comes back and the message stays.
    fn restart_daemon(&mut self) {
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "restart", "nightlightd"])
            .spawn();
        self.refresh();
    }

    /// Brings a stopped daemon up (#43). systemd first, so the usual case
    /// lands in the supervised unit; when that fails (no unit installed, a
    /// tarball setup), the daemon binary next to this one is spawned
    /// directly. A thin client cannot be the daemon, but it can ask for one.
    fn start_daemon(&mut self) {
        let unit = std::process::Command::new("systemctl")
            .args(["--user", "start", "nightlightd"])
            .status();
        if !unit.is_ok_and(|status| status.success()) {
            let _ = std::process::Command::new(sibling("nightlightd"))
                .arg("--daemon")
                .spawn();
        }
        self.refresh();
    }

    /// Flips the fade walk, optimistically so the checkmark answers the
    /// click at once; the refresh confirms.
    fn set_fade(&mut self, fade: bool) {
        self.client.set_fade(fade);
        self.fade = Some(fade);
        self.refresh();
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
        // A version mismatch (#42) shows the plain icon: the daemon *is*
        // running, and the disabled variant here is exactly the lie that
        // motivated the issue.
        let on = self.status.as_ref().is_some_and(|status| status.enabled) || self.mismatch;
        if on {
            "night-light-symbolic".into()
        } else {
            "night-light-disabled-symbolic".into()
        }
    }

    /// Left click toggles the filter — the one action people want most.
    /// With no daemon to toggle, the click starts one instead (#43).
    fn activate(&mut self, _x: i32, _y: i32) {
        if self.status.is_none() && !self.mismatch {
            self.start_daemon();
        } else {
            self.toggle();
        }
    }

    /// The hover text: the tray's version of `--status`.
    fn tool_tip(&self) -> ToolTip {
        let description = match &self.status {
            Some(status) => status.describe(),
            None if self.mismatch => "update needed\n\
                 tray and daemon are different versions"
                .into(),
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
        // No daemon on the bus at all (#43): every filter action would be a
        // call to nobody, so the menu offers the one thing that helps.
        if self.status.is_none() && !self.mismatch {
            return vec![
                StandardItem {
                    label: "Start the daemon".into(),
                    activate: Box::new(|this: &mut Self| this.start_daemon()),
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
            ];
        }

        let on = self.status.as_ref().is_some_and(|status| status.enabled);
        // The item promises a direction, so send that direction — a blind
        // Toggle against status gone stale would do the opposite of the label.
        let turn_on = !on;
        let mut items: Vec<MenuItem<Self>> = vec![
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
        ];
        // The user-clicked half of the #42 recovery: a daemon still running
        // its pre-update binary is fixed by a restart, and clicking is the
        // user's decision in a way an automatic kill never could be.
        if self.mismatch {
            items.push(
                StandardItem {
                    label: "Restart the daemon".into(),
                    activate: Box::new(|this: &mut Self| this.restart_daemon()),
                    ..Default::default()
                }
                .into(),
            );
        }
        // The fade switch (#44) earns an item only when the daemon can
        // answer for it; a checkbox nobody reads behind would lie.
        if let Some(fade) = self.fade {
            items.push(
                CheckmarkItem {
                    label: "Fade transitions".into(),
                    checked: fade,
                    activate: Box::new(move |this: &mut Self| this.set_fade(!fade)),
                    ..Default::default()
                }
                .into(),
            );
        }
        items.extend([
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
        ]);
        items
    }
}

/// The one self-repair a stale client can do (#42): replace this process
/// with whatever its own path holds on disk now. After an update the
/// running copy is old while the file is new, and this heals that with
/// nobody watching. Guarded to a single attempt — when the disk copy is
/// just as old, exec would loop forever otherwise. `exec` only returns on
/// failure, and every failure path falls through to the visible message.
fn relaunch_once() {
    use std::os::unix::process::CommandExt;
    use std::sync::atomic::{AtomicBool, Ordering};
    static TRIED: AtomicBool = AtomicBool::new(false);
    if TRIED.swap(true, Ordering::SeqCst) || std::env::var_os("NIGHTLIGHT_RELAUNCHED").is_some() {
        return;
    }
    let mut args = std::env::args_os();
    let Some(argv0) = args.next() else {
        return;
    };
    let _ = std::process::Command::new(argv0)
        .args(args)
        .env("NIGHTLIGHT_RELAUNCHED", "1")
        .exec();
}

/// The named sibling binary when it exists next to this one (the four
/// install together, which survives an autostart PATH that lacks
/// `~/.cargo/bin`), otherwise the bare name so the PATH lookup gets a real
/// chance instead of being dead code.
fn sibling(name: &str) -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .filter(|path| path.exists())
        .unwrap_or_else(|| std::path::PathBuf::from(name))
}

/// Launches the settings panel. Errors are swallowed — a failed launch must
/// not take the tray down.
fn open_panel() {
    let _ = std::process::Command::new(sibling("nightlight-panel")).spawn();
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

    // One tray per user bus (GitHub #1): claim a well-known name before
    // showing anything; if it is taken, a tray from this or an earlier login
    // is already alive and this one leaves quietly.
    match client.claim_tray_name() {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("nightlight-tray: already running");
            return;
        }
        // Carry on unlocked: a duplicate icon beats no icon at all.
        Err(error) => {
            eprintln!("nightlight-tray: cannot check for another tray: {error}");
        }
    }

    let status = client.status();
    let fade = client.fade();
    let mismatch = status.is_none() && client.daemon_on_bus();
    if mismatch {
        // Heal a stale process before the icon even registers.
        relaunch_once();
    }
    // `assume_sni_available(true)`: at login the tray autostarts before the
    // panel's StatusNotifierWatcher exists, so a plain spawn() fails on the
    // missing watcher and the icon never appears (confirmed in
    // ~/.xsession-errors). With this, ksni treats the absent watcher as a soft
    // error and registers the icon once the panel's tray comes online.
    let handle = match (NightLight {
        client,
        status,
        fade,
        mismatch,
    })
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
