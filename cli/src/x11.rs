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
use nightlightd_core::mode::resolve_temperature;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::randr::{ConnectionExt as _, GetScreenResourcesReply, NotifyMask};

use crate::state::{Shared, State, lock};
use crate::waker::Waker;

/// How often the watch loops wake to re-apply: the daemon recomputes the sun on
/// this tick, and it doubles as the safety net that heals silent wipes. When
/// the config lands (#17) this stays a minute; for now it is fixed.
const TICK_INTERVAL: Duration = Duration::from_secs(60);

/// The neutral temperature whose ramp is the identity — a normal screen.
pub const NEUTRAL_KELVIN: u32 = 6500;

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
    write_ramps(&conn, &crtcs, kelvin, true)?;
    conn.flush()?;
    Ok(crtcs.len())
}

/// Runs the daemon: applies whatever the shared state calls for and keeps it
/// applied. Wakes on a D-Bus request (the waker eventfd), a RandR screen change,
/// or the minute tick, then re-derives the target and applies it. Runs until
/// `terminate` is set.
pub fn daemon_loop(
    state: &Shared,
    waker: &Waker,
    terminate: &AtomicBool,
) -> Result<(), Box<dyn Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;

    conn.randr_select_input(root, NotifyMask::SCREEN_CHANGE | NotifyMask::CRTC_CHANGE)?
        .check()?;
    apply_desired(&conn, root, state)?;

    let mut last_tick = Instant::now();
    while !terminate.load(Ordering::Relaxed) {
        if !wait_for_change(
            &[conn.stream().as_fd(), waker.as_fd()],
            TICK_INTERVAL.saturating_sub(last_tick.elapsed()),
        )? {
            continue;
        }

        // A D-Bus request, a screen change, or the tick all mean the same
        // thing: drain both wake sources and re-apply what the state now wants.
        waker.drain();
        drain_screen_changes(&conn)?;
        apply_desired(&conn, root, state)?;
        if last_tick.elapsed() >= TICK_INTERVAL {
            last_tick = Instant::now();
        }
    }
    Ok(())
}

/// Blocks on the X fd until an event, `timeout`, or a signal. Returns `false`
/// if a signal interrupted the wait (the caller should re-check `terminate`);
/// `poll`/`ppoll` return EINTR on a signal even under SA_RESTART, so Ctrl+C
/// wakes us at once and idle CPU stays ~0%.
fn wait_for_change(fds: &[BorrowedFd<'_>], timeout: Duration) -> Result<bool, Box<dyn Error>> {
    let timeout = duration_to_timespec(timeout);
    let mut poll_fds: Vec<PollFd<'_>> = fds
        .iter()
        .map(|fd| PollFd::new(fd, PollFlags::IN))
        .collect();
    match poll(&mut poll_fds, Some(&timeout)) {
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

/// Applies the temperature the current state calls for, and records it as the
/// current temperature (without holding the lock across the X writes).
fn apply_desired<C: Connection>(conn: &C, root: u32, state: &Shared) -> Result<(), Box<dyn Error>> {
    let (target, previous) = {
        let state = lock(state);
        (desired_temp(&state), state.current_temp)
    };
    reapply(conn, root, target, target != previous)?;
    lock(state).current_temp = target;
    Ok(())
}

/// The temperature the state calls for: neutral when disabled, the manual
/// override when set, otherwise the sun-based target for right now.
fn desired_temp(state: &State) -> u32 {
    if !state.enabled {
        NEUTRAL_KELVIN
    } else if let Some(kelvin) = state.override_temp {
        kelvin
    } else {
        resolve_temperature(state.mode, unix_now(), state.day_temp, state.night_temp)
    }
}

/// Seconds since the Unix epoch, as an `f64` for the solar maths. Degrades to
/// `0.0` rather than panicking if the clock is somehow before the epoch.
pub(crate) fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

/// Re-reads the current CRTCs and writes the `kelvin` ramp to each. Re-reading
/// means a newly-attached monitor is covered too (issue #14).
fn reapply<C: Connection>(
    conn: &C,
    root: u32,
    kelvin: u32,
    changed: bool,
) -> Result<(), Box<dyn Error>> {
    let resources = conn.randr_get_screen_resources(root)?.reply()?;
    let crtcs = active_crtcs(conn, &resources)?;
    write_ramps(conn, &crtcs, kelvin, changed)?;
    conn.flush()?;
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
    changed: bool,
) -> Result<(), Box<dyn Error>> {
    let gains = temperature_to_rgb(kelvin);
    for c in crtcs {
        let ramp = build_ramp(c.gamma_size, gains);
        conn.randr_set_crtc_gamma(c.crtc, &ramp.red, &ramp.green, &ramp.blue)?
            .check()?;
        // A change (sun moved, a client request) is logged by default; an
        // unchanged periodic tick is only logged at debug.
        if changed {
            tracing::info!("applied {kelvin} K to CRTC {}", c.crtc);
        } else {
            tracing::debug!("applied {kelvin} K to CRTC {}", c.crtc);
        }
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
