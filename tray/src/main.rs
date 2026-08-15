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
use ksni::menu::Disposition;
use ksni::menu::Disposition::{Alert, Informative, Warning};
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{MenuItem, ToolTip};
use nightlightd_core::transition::{Band, phase};

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
    /// The transition band (#39) the daemon runs, so the tooltip names the
    /// sun's phase the way the screen behaves rather than the way the
    /// defaults used to.
    band: Band,
}

impl NightLight {
    /// Re-reads the daemon and stores the result (`None` when unreachable).
    fn refresh(&mut self) {
        self.status = self.client.status();
        self.fade = self.client.fade();
        self.band = self.client.band();
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
            Some(status) => status.describe(self.band),
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

    /// Right click. The toggle label reflects the current state so it reads
    /// as an action rather than a question, and the whole menu keeps one
    /// shape whatever the daemon is doing (#46).
    ///
    /// The ceiling, stated where someone will next be tempted to raise it:
    /// this menu is drawn by the desktop's own panel. Fonts, colours,
    /// spacing and how a disabled item looks all belong to the GTK theme and
    /// cannot be reached from here. What we do control is which items exist,
    /// what order they come in, where the separators fall, which icon names
    /// they carry, and each item's `Disposition` — the one hint the protocol
    /// gives about how a line should feel. Everything below is those levers.
    fn menu(&self) -> Vec<MenuItem<Self>> {
        // Three blocks, always in this order: what is happening, what you can
        // do about it, and the application itself (#46). The menu had grown
        // by accretion — #42, #43 and #44 each pushed an item onto one flat
        // pile — and a pile that changes length every time it opens is a menu
        // nobody builds muscle memory for.
        let mut items: Vec<MenuItem<Self>> = vec![self.readout()];
        if let Some(band) = self.band_line() {
            items.push(band);
        }
        items.push(MenuItem::Separator);

        // The filter's own controls. Unreachable during a version mismatch
        // (#42), and disabled rather than hidden, because the line above says
        // exactly why — hiding is for a control whose absence cannot be
        // explained, like the fade switch against a daemon that has never
        // heard of it.
        let reachable = self.status.is_some();
        let on = self.status.as_ref().is_some_and(|status| status.enabled);
        // The item promises a direction, so send that direction — a blind
        // Toggle against status gone stale would do the opposite of the label.
        let turn_on = !on;
        items.push(
            StandardItem {
                label: if turn_on { "Turn on" } else { "Turn off" }.into(),
                enabled: reachable,
                activate: Box::new(move |this: &mut Self| this.set_enabled(turn_on)),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            CheckmarkItem {
                label: "Automatic".into(),
                enabled: reachable,
                checked: self.status.as_ref().is_some_and(|status| status.following),
                activate: Box::new(|this: &mut Self| this.toggle_follow()),
                ..Default::default()
            }
            .into(),
        );
        // The fade switch (#44) earns an item only when the daemon can
        // answer for it; a checkbox nobody reads behind would lie, and a
        // greyed one would invite a click that can never work.
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
        items.push(MenuItem::Separator);

        // The daemon's own lifecycle: whichever of the two repairs applies.
        // Starting a daemon that is not there (#43), or restarting one still
        // running its pre-update binary (#42) — the user-clicked half of that
        // recovery, a decision an automatic kill could never make for them.
        if self.status.is_none() {
            let (label, icon, restart) = if self.mismatch {
                ("Restart the daemon", "view-refresh", true)
            } else {
                ("Start the daemon", "media-playback-start", false)
            };
            items.push(
                StandardItem {
                    label: label.into(),
                    icon_name: icon.into(),
                    activate: Box::new(move |this: &mut Self| {
                        if restart {
                            this.restart_daemon()
                        } else {
                            this.start_daemon()
                        }
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }
        // Only when there is a panel to open. It ships as its own package
        // (#50) because it is the one binary of the four that needs a
        // graphics stack, so a complete and sensible install — daemon, this
        // tray, the dashboard — can be without it, and a "Settings…" that
        // silently does nothing would be worse than no "Settings…" at all.
        //
        // Asked here rather than once at startup, which costs a `stat` every
        // REFRESH and means installing the panel later is picked up within
        // five seconds instead of needing the tray restarted: ksni's
        // `update` re-runs `update_menu`, which calls this method again.
        if located(PANEL).is_some() {
            items.push(
                StandardItem {
                    label: "Settings…".into(),
                    icon_name: "preferences-system".into(),
                    activate: Box::new(|_| open_panel()),
                    ..Default::default()
                }
                .into(),
            );
        }
        items.extend([
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

impl NightLight {
    /// The top line of the menu: what the screen is doing, as something you
    /// cannot click. The tooltip has carried this since the beginning, but a
    /// tooltip needs a hover held over a 22-pixel icon, and several desktops
    /// show it late or not at all. Opening the menu is the deliberate act;
    /// the answer should be waiting there.
    fn readout(&self) -> MenuItem<Self> {
        let (label, disposition) = readout_label(self.status.as_ref(), self.mismatch, self.band);
        StandardItem {
            label,
            enabled: false,
            disposition,
            ..Default::default()
        }
        .into()
    }

    /// The band under the readout, by the rule the status line uses: the
    /// default earns no ink. Someone who has never touched it never learns
    /// the word; someone who has can check it without opening anything.
    fn band_line(&self) -> Option<MenuItem<Self>> {
        if self.status.is_none() || self.band == Band::default() {
            return None;
        }
        Some(
            StandardItem {
                label: format!(
                    "Band · day above {:+.1}°, night below {:+.1}°",
                    self.band.day_elevation, self.band.night_elevation
                ),
                enabled: false,
                disposition: Informative,
                ..Default::default()
            }
            .into(),
        )
    }
}

/// The readout's words, kept out of the menu item so they can be tested:
/// a `MenuItem` carries closures and cannot be read back. Order matters —
/// "off" outranks everything, because a filter that is off is the answer to
/// the question no matter where the sun is, and "manual" outranks the phase,
/// because a pinned temperature is not following anything.
fn readout_label(status: Option<&Status>, mismatch: bool, band: Band) -> (String, Disposition) {
    match status {
        Some(status) if !status.enabled => (format!("{} K · off", status.temperature), Informative),
        Some(status) if !status.following => {
            (format!("{} K · manual", status.temperature), Informative)
        }
        Some(status) if status.has_location => (
            format!(
                "{} K · {}",
                status.temperature,
                phase(status.elevation, band)
            ),
            Informative,
        ),
        Some(status) => (
            format!("{} K · no location", status.temperature),
            Informative,
        ),
        None if mismatch => ("Update needed · versions differ".to_string(), Alert),
        None => ("Daemon not running".to_string(), Warning),
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

/// The first of `dirs` holding a file called `name`.
///
/// Split out from the two lookups below because it is the part worth
/// testing: everything else is `current_exe` and `PATH`, which a test can
/// only assert about the machine it runs on.
fn first_holding(
    dirs: impl Iterator<Item = std::path::PathBuf>,
    name: &str,
) -> Option<std::path::PathBuf> {
    dirs.map(|dir| dir.join(name)).find(|path| path.is_file())
}

/// Where a companion binary actually is: next to this one first (they
/// install together, which survives an autostart `PATH` that lacks
/// `~/.cargo/bin`), otherwise wherever `PATH` has it. `None` means it is
/// not installed at all, which since #50 is an ordinary state rather than a
/// broken one — the panel is its own package now.
fn located(name: &str) -> Option<std::path::PathBuf> {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(std::path::Path::to_path_buf));
    first_holding(beside.into_iter(), name).or_else(|| {
        let path = std::env::var_os("PATH")?;
        first_holding(std::env::split_paths(&path), name)
    })
}

/// The same, for a caller that is going to spawn regardless: the bare name
/// is a fair last try, since the exec itself does one more `PATH` lookup.
fn sibling(name: &str) -> std::path::PathBuf {
    located(name).unwrap_or_else(|| std::path::PathBuf::from(name))
}

/// The settings panel's binary name. It is one package (#50) and this tray
/// is in another, so it may simply not be here.
const PANEL: &str = "nightlight-panel";

/// Launches the settings panel. Errors are swallowed — a failed launch must
/// not take the tray down.
fn open_panel() {
    if let Some(panel) = located(PANEL) {
        let _ = std::process::Command::new(panel).spawn();
    }
}

const USAGE: &str = "usage: nightlight-tray [--version] [--help]

Puts the filter's state and its controls in the notification area.
Takes no options; everything else is set from the menu.";

/// Answers `--version` and `--help` before anything is drawn, and refuses
/// anything else.
///
/// The daemon has answered `--version` since it had a `main`, because clap
/// gives it away; the three clients parsed nothing and so quietly *started* on
/// any flag they were handed. That is the worst of the three possible
/// answers — a packager or a bug report runs `--version` first, and a typo
/// deserves a complaint rather than a window.
///
/// Returns true when the argument was the whole of the job.
fn cli_only() -> bool {
    let Some(argument) = std::env::args().nth(1) else {
        return false;
    };
    match argument.as_str() {
        "--version" | "-V" => println!("nightlight-tray {}", env!("CARGO_PKG_VERSION")),
        "--help" | "-h" => println!("{USAGE}"),
        other => {
            eprintln!("nightlight-tray: unknown option {other:?}\n\n{USAGE}");
            std::process::exit(2);
        }
    }
    true
}

fn main() {
    if cli_only() {
        return;
    }
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
    let band = client.band();
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
        band,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The panel is a separate package (#50), so "is it installed" is a
    /// real question with a real "no". Getting that wrong in either
    /// direction is a visible defect: a menu item that opens nothing, or a
    /// missing one on a machine that has the panel.
    #[test]
    fn a_companion_binary_is_found_only_where_it_actually_is() {
        let root = std::env::temp_dir().join("nightlightd-located-test");
        let (present, absent) = (root.join("bin"), root.join("empty"));
        std::fs::create_dir_all(&present).expect("temp dir");
        std::fs::create_dir_all(&absent).expect("temp dir");
        let panel = present.join(PANEL);
        std::fs::write(&panel, b"not really a binary").expect("temp file");

        assert_eq!(
            first_holding([present.clone()].into_iter(), PANEL),
            Some(panel)
        );
        assert_eq!(first_holding([absent.clone()].into_iter(), PANEL), None);
        // Nothing anywhere is the answer for a daemon-only install.
        assert_eq!(first_holding(std::iter::empty(), PANEL), None);
        // A directory of that name is not a binary of that name.
        std::fs::create_dir_all(absent.join(PANEL)).expect("temp dir");
        assert_eq!(first_holding([absent].into_iter(), PANEL), None);
        // Earlier directories win, which is what puts a sibling ahead of PATH.
        let dirs = [root.join("nowhere"), present, root.join("later")];
        assert!(first_holding(dirs.into_iter(), PANEL).is_some_and(|p| p.ends_with(PANEL)));

        std::fs::remove_dir_all(&root).ok();
    }

    fn following_at(elevation: f64) -> Status {
        Status {
            enabled: true,
            temperature: 4200,
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

    /// The line has to answer "what is my screen doing" in every state the
    /// tray can be in, and never answer it with the sun when the sun is not
    /// what is driving the screen.
    #[test]
    fn the_readout_answers_for_every_state() {
        let sunny = following_at(-6.5);
        assert_eq!(
            readout_label(Some(&sunny), false, Band::default()).0,
            "4200 K · night"
        );
        // The same instant under a band that reaches deeper into dusk.
        let deep = Band {
            day_elevation: 3.0,
            night_elevation: -14.0,
        };
        assert_eq!(
            readout_label(Some(&sunny), false, deep).0,
            "4200 K · transition"
        );

        let mut off = sunny.clone();
        off.enabled = false;
        assert_eq!(
            readout_label(Some(&off), false, Band::default()).0,
            "4200 K · off"
        );

        let mut manual = sunny.clone();
        manual.following = false;
        assert_eq!(
            readout_label(Some(&manual), false, Band::default()).0,
            "4200 K · manual"
        );

        let mut placeless = sunny.clone();
        placeless.has_location = false;
        assert_eq!(
            readout_label(Some(&placeless), false, Band::default()).0,
            "4200 K · no location"
        );
    }

    /// The two daemon-shaped absences must not read alike: one is fixed by
    /// starting something, the other by installing something (#42).
    #[test]
    fn an_absent_daemon_and_a_stale_one_read_differently() {
        let (stopped, stopped_tone) = readout_label(None, false, Band::default());
        let (stale, stale_tone) = readout_label(None, true, Band::default());
        assert_ne!(stopped, stale);
        assert_eq!(stopped_tone, Warning);
        assert_eq!(stale_tone, Alert);
    }

    /// An off filter is off whatever the sun is doing: the phase word must
    /// not leak into a line about a screen nothing is filtering.
    #[test]
    fn a_disabled_filter_does_not_report_the_sun() {
        let mut off = following_at(45.0);
        off.enabled = false;
        let (label, _) = readout_label(Some(&off), false, Band::default());
        assert!(!label.contains("day"));
        assert!(label.ends_with("off"));
    }
}
