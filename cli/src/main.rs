//! `nightlightd` — a screen colour temperature daemon for X11.
//!
//! One binary, two modes: `--daemon` runs the daemon, a bare invocation such
//! as `--temp 2800` acts as a client and messages the daemon over D-Bus.
//!
//! Neither mode exists yet; this is the M0 skeleton. Argument parsing, the
//! daemon, and the client arrive from M2 onward.

fn main() {
    // Intentionally does nothing yet.
}
