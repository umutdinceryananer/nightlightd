//! The X11 / XRandR backend (issue #10 onward).
//!
//! `core` never touches the screen; everything that talks to the X server
//! lives here. It discovers CRTCs and their gamma-ramp sizes (which differ
//! between outputs — 256, 1024, 2048 — and must be read, not assumed), writes
//! ramps to them, and keeps the ramp applied:
//!
//! * RandR events (hotplug, mode/resolution change) are corrected immediately.
//! * A periodic tick re-applies the ramp, which both follows the sun (in the
//!   daemon) and overwrites silent wipes that emit no event (a bare gamma
//!   write, some fullscreen games, DPMS wakeups).

use std::error::Error;
use std::os::fd::{AsFd, BorrowedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nightlightd_core::color::{build_ramp, temperature_to_rgb};
use nightlightd_core::mode::{Mode, resolve_temperature};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::randr::{ConnectionExt as _, GetScreenResourcesReply, NotifyMask};

/// How often the watch loops wake to re-apply: the daemon recomputes the sun on
/// this tick, and it doubles as the safety net that heals silent wipes. When
/// the config lands (#17) this stays a minute; for now it is fixed.
const TICK_INTERVAL: Duration = Duration::from_secs(60);

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

/// Holds a fixed `kelvin` until `terminate` is set: RandR changes trigger an
/// immediate re-apply, and a periodic tick reads the gamma back and rewrites it
/// if a silent wipe drifted it.
pub fn hold_and_watch(kelvin: u32, terminate: &AtomicBool) -> Result<(), Box<dyn Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;

    conn.randr_select_input(root, NotifyMask::SCREEN_CHANGE | NotifyMask::CRTC_CHANGE)?
        .check()?;
    reapply(&conn, root, kelvin)?;

    let mut last_verify = Instant::now();
    while !terminate.load(Ordering::Relaxed) {
        if !wait_for_change(
            conn.stream().as_fd(),
            TICK_INTERVAL.saturating_sub(last_verify.elapsed()),
        )? {
            continue;
        }

        if drain_screen_changes(&conn)? {
            reapply(&conn, root, kelvin)?;
        }
        if last_verify.elapsed() >= TICK_INTERVAL {
            verify(&conn, root, kelvin)?;
            last_verify = Instant::now();
        }
    }
    Ok(())
}

/// Runs the daemon: follows the sun. On each tick it recomputes the target from
/// the timezone location and the current time, then applies it — which also
/// overwrites any silent wipe. RandR changes re-apply the current target at
/// once. Runs until `terminate` is set.
pub fn daemon_loop(
    mode: Mode,
    day_temp: u32,
    night_temp: u32,
    terminate: &AtomicBool,
) -> Result<(), Box<dyn Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;

    conn.randr_select_input(root, NotifyMask::SCREEN_CHANGE | NotifyMask::CRTC_CHANGE)?
        .check()?;

    let mut target = sun_target(mode, day_temp, night_temp);
    reapply(&conn, root, target)?;
    println!("nightlightd: target {target} K");

    let mut last_tick = Instant::now();
    while !terminate.load(Ordering::Relaxed) {
        if !wait_for_change(
            conn.stream().as_fd(),
            TICK_INTERVAL.saturating_sub(last_tick.elapsed()),
        )? {
            continue;
        }

        if drain_screen_changes(&conn)? {
            reapply(&conn, root, target)?;
        }
        if last_tick.elapsed() >= TICK_INTERVAL {
            target = sun_target(mode, day_temp, night_temp);
            reapply(&conn, root, target)?;
            println!("nightlightd: target {target} K");
            last_tick = Instant::now();
        }
    }
    Ok(())
}

/// Blocks on the X fd until an event, `timeout`, or a signal. Returns `false`
/// if a signal interrupted the wait (the caller should re-check `terminate`);
/// `poll`/`ppoll` return EINTR on a signal even under SA_RESTART, so Ctrl+C
/// wakes us at once and idle CPU stays ~0%.
fn wait_for_change(fd: BorrowedFd<'_>, timeout: Duration) -> Result<bool, Box<dyn Error>> {
    let timeout = duration_to_timespec(timeout);
    let mut fds = [PollFd::new(&fd, PollFlags::IN)];
    match poll(&mut fds, Some(&timeout)) {
        Ok(_) => Ok(true),
        Err(error) if error == rustix::io::Errno::INTR => Ok(false),
        Err(error) => Err(Box::new(error)),
    }
}

/// Drains all pending X events and reports whether any was a RandR screen or
/// CRTC change worth re-applying for.
fn drain_screen_changes<C: Connection>(conn: &C) -> Result<bool, Box<dyn Error>> {
    let mut changed = false;
    while let Some(event) = conn.poll_for_event()? {
        if matches!(
            event,
            Event::RandrScreenChangeNotify(_) | Event::RandrNotify(_)
        ) {
            changed = true;
        }
    }
    Ok(changed)
}

/// The target temperature for right now, given the mode (automatic or a manual
/// location) and the day/night bounds.
fn sun_target(mode: Mode, day_temp: u32, night_temp: u32) -> u32 {
    resolve_temperature(mode, unix_now(), day_temp, night_temp)
}

/// Seconds since the Unix epoch, as an `f64` for the solar maths. Degrades to
/// `0.0` rather than panicking if the clock is somehow before the epoch.
fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
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
