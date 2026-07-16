//! The "Start at login" toggle: the daemon's systemd user service (#25).
//!
//! Ticking the box enables the unit so the daemon starts on the next login;
//! unticking disables it. The panel shells out to `systemctl --user` rather
//! than linking anything — this is exactly what a user would type by hand.

use std::process::Command;

/// The daemon's systemd user unit name.
const UNIT: &str = "nightlightd";

/// Whether the daemon's user service is enabled (starts at login). Anything
/// other than a clean `enabled` — disabled, not installed, systemd absent — is
/// reported as `false`.
pub fn enabled() -> bool {
    Command::new("systemctl")
        .args(["--user", "is-enabled", UNIT])
        .output()
        .map(|out| out.stdout.trim_ascii() == b"enabled")
        .unwrap_or(false)
}

/// Enables or disables the daemon's user service. Errors (no systemd, unit not
/// installed) are swallowed — the checkbox must not crash the panel.
pub fn set(enable: bool) {
    let verb = if enable { "enable" } else { "disable" };
    let _ = Command::new("systemctl")
        .args(["--user", verb, UNIT])
        .status();
}
