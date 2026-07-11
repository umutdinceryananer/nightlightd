//! The X11 / XRandR backend (issue #10 onward).
//!
//! `core` never touches the screen; everything that talks to the X server
//! lives here. It discovers CRTCs and their gamma-ramp sizes (which differ
//! between outputs — 256, 1024, 2048 — and must be read, not assumed), writes
//! ramps to them, and keeps the ramp applied:
//!
//! * RandR events (hotplug, mode/resolution change) are corrected immediately.
//! * Silent wipes that emit no event (a bare gamma write, some fullscreen
//!   games, DPMS wakeups) are caught by reading the gamma back on a periodic
//!   verification tick and rewriting it if it drifted.

use std::error::Error;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nightlightd_core::color::{build_ramp, temperature_to_rgb};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::randr::{ConnectionExt as _, GetScreenResourcesReply, NotifyMask};

/// How often the watch loop reads the gamma back and re-applies it if a silent
/// wipe drifted it. When the daemon's minute timer lands (#15) this rides on
/// that same tick; here it is a standalone timer.
const VERIFY_INTERVAL: Duration = Duration::from_secs(60);

/// One active CRTC (a "screen" in XRandR terms) and the size of its gamma ramp.
#[derive(Debug, Clone, Copy)]
pub struct CrtcInfo {
    /// The XRandR CRTC identifier.
    pub crtc: u32,
    /// Number of entries in this CRTC's gamma ramp, per channel.
    pub gamma_size: u16,
}

/// Connects to the X server and returns every active CRTC with its gamma-ramp
/// size.
///
/// Returns an error rather than panicking when the X server is unreachable, so
/// the caller can fail quietly.
pub fn discover() -> Result<Vec<CrtcInfo>, Box<dyn Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;
    let resources = conn.randr_get_screen_resources(root)?.reply()?;
    active_crtcs(&conn, &resources)
}

/// Writes the gamma ramp for `kelvin` to every active CRTC once, and returns
/// how many were updated. 6500 K produces the identity ramp (a normal screen).
pub fn apply_temperature(kelvin: u32) -> Result<usize, Box<dyn Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;
    let resources = conn.randr_get_screen_resources(root)?.reply()?;
    let crtcs = active_crtcs(&conn, &resources)?;
    write_ramps(&conn, &crtcs, kelvin)?;
    conn.flush()?;
    Ok(crtcs.len())
}

/// Applies `kelvin`, then keeps it applied until `terminate` is set: RandR
/// screen/CRTC changes trigger an immediate re-apply, and a periodic tick reads
/// the gamma back and rewrites it if a silent wipe drifted it.
pub fn hold_and_watch(kelvin: u32, terminate: &AtomicBool) -> Result<(), Box<dyn Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;

    // Subscribe before the first apply so no change slips through the gap.
    conn.randr_select_input(root, NotifyMask::SCREEN_CHANGE | NotifyMask::CRTC_CHANGE)?
        .check()?;
    reapply(&conn, root, kelvin)?;

    let mut last_verify = Instant::now();
    while !terminate.load(Ordering::Relaxed) {
        // Block until a RandR event, the next verification tick, or a signal.
        // `poll`/`ppoll` return EINTR on a signal even under SA_RESTART, so
        // Ctrl+C wakes us at once; idle CPU is ~0%.
        let timeout = duration_to_timespec(VERIFY_INTERVAL.saturating_sub(last_verify.elapsed()));
        let mut fds = [PollFd::new(conn.stream(), PollFlags::IN)];
        match poll(&mut fds, Some(&timeout)) {
            Ok(_) => {}
            Err(error) if error == rustix::io::Errno::INTR => continue,
            Err(error) => return Err(Box::new(error)),
        }

        // Event-driven path: correct immediately on any screen/CRTC change.
        let mut changed = false;
        while let Some(event) = conn.poll_for_event()? {
            if matches!(
                event,
                Event::RandrScreenChangeNotify(_) | Event::RandrNotify(_)
            ) {
                changed = true;
            }
        }
        if changed {
            reapply(&conn, root, kelvin)?;
        }

        // Verification path: catch wipes that emitted no event.
        if last_verify.elapsed() >= VERIFY_INTERVAL {
            verify(&conn, root, kelvin)?;
            last_verify = Instant::now();
        }
    }
    Ok(())
}

/// Re-reads the current CRTCs and writes the `kelvin` ramp to each. Re-reading
/// means a newly-attached monitor is covered too (issue #14).
fn reapply<C: Connection>(conn: &C, root: u32, kelvin: u32) -> Result<(), Box<dyn Error>> {
    let resources = conn.randr_get_screen_resources(root)?.reply()?;
    let crtcs = active_crtcs(conn, &resources)?;
    write_ramps(conn, &crtcs, kelvin)?;
    conn.flush()?;
    Ok(())
}

/// Reads each CRTC's current gamma and rewrites only those that have drifted
/// from the expected ramp — the cheap safety net for silent wipes.
fn verify<C: Connection>(conn: &C, root: u32, kelvin: u32) -> Result<(), Box<dyn Error>> {
    let resources = conn.randr_get_screen_resources(root)?.reply()?;
    let crtcs = active_crtcs(conn, &resources)?;
    let gains = temperature_to_rgb(kelvin);
    for c in &crtcs {
        let expected = build_ramp(c.gamma_size, gains);
        let current = conn.randr_get_crtc_gamma(c.crtc)?.reply()?;
        if current.red != expected.red
            || current.green != expected.green
            || current.blue != expected.blue
        {
            conn.randr_set_crtc_gamma(c.crtc, &expected.red, &expected.green, &expected.blue)?
                .check()?;
        }
    }
    Ok(())
}

/// Collects the active CRTCs (those driving an output) and their gamma sizes
/// from an already-fetched screen-resources reply.
fn active_crtcs<C: Connection>(
    conn: &C,
    resources: &GetScreenResourcesReply,
) -> Result<Vec<CrtcInfo>, Box<dyn Error>> {
    let mut crtcs = Vec::new();
    for &crtc in &resources.crtcs {
        let info = conn
            .randr_get_crtc_info(crtc, resources.config_timestamp)?
            .reply()?;

        // A CRTC with no mode is not driving an output and has no gamma ramp.
        if info.mode == 0 {
            continue;
        }

        let gamma_size = conn.randr_get_crtc_gamma_size(crtc)?.reply()?.size;
        crtcs.push(CrtcInfo { crtc, gamma_size });
    }
    Ok(crtcs)
}

/// Builds and writes the `kelvin` ramp to each CRTC.
fn write_ramps<C: Connection>(
    conn: &C,
    crtcs: &[CrtcInfo],
    kelvin: u32,
) -> Result<(), Box<dyn Error>> {
    let gains = temperature_to_rgb(kelvin);
    for c in crtcs {
        let ramp = build_ramp(c.gamma_size, gains);
        conn.randr_set_crtc_gamma(c.crtc, &ramp.red, &ramp.green, &ramp.blue)?
            .check()?;
    }
    Ok(())
}

/// Converts a [`Duration`] into a rustix [`Timespec`] for `poll`.
fn duration_to_timespec(duration: Duration) -> Timespec {
    Timespec {
        tv_sec: duration.as_secs() as i64,
        tv_nsec: i64::from(duration.subsec_nanos()),
    }
}
