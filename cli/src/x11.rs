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
use nightlightd_core::location::location_from_timezone;
use nightlightd_core::mode::{Mode, resolve_temperature};
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

/// After a screen change or a resume, the layout is "settling": another actor
/// (a display/colour helper) may re-assert its own gamma ramp a beat later,
/// with no RandR event to wake us. For a short window we poll every
/// [`SETTLE_INTERVAL`] instead of waiting out the full tick, so such a silent
/// reset is overwritten within about a second rather than up to a minute (#13).
/// Steady state is untouched: once the window passes, the loop is back to the
/// 60 s tick and idle CPU stays ~0%.
const SETTLE_WINDOW: Duration = Duration::from_secs(15);
const SETTLE_INTERVAL: Duration = Duration::from_secs(1);

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
    resumed: &AtomicBool,
    terminate: &AtomicBool,
) -> Result<(), Box<dyn Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;

    conn.randr_select_input(root, NotifyMask::SCREEN_CHANGE | NotifyMask::CRTC_CHANGE)?
        .check()?;

    try_apply(&conn, root, state)?;

    let mut last_tick = Instant::now();
    // While `Some(deadline)` and not yet past it, poll fast to overwrite a
    // silent gamma reset that emits no event (see [`SETTLE_WINDOW`]).
    let mut settle_until: Option<Instant> = None;
    while !terminate.load(Ordering::Relaxed) {
        let tick_remaining = TICK_INTERVAL.saturating_sub(last_tick.elapsed());
        let settling = settle_until.is_some_and(|deadline| Instant::now() < deadline);
        if !wait_for_change(
            &[conn.stream().as_fd(), waker.as_fd()],
            poll_timeout(tick_remaining, settling),
        )? {
            continue;
        }

        // A D-Bus request, a screen change, or the tick all mean the same
        // thing: drain both wake sources and re-apply what the state now wants.
        waker.drain();
        // A resume emits no RandR event (the waker fires instead), so record it
        // before it is lost; it arms settling just like a screen change does.
        let woke_on_resume = resumed.swap(false, Ordering::Relaxed);
        let mut layout_changed = drain_screen_changes(&conn)?;
        try_apply(&conn, root, state)?;
        // Events that raced in during our own round trips would otherwise wake
        // the loop again at once for a full extra pass; absorb them with one
        // bounded re-apply instead (never a loop — a storm settles on the tick).
        if drain_screen_changes(&conn)? {
            layout_changed = true;
            try_apply(&conn, root, state)?;
        }
        // Arm (or re-arm) the settling window on any layout change or resume, so
        // a gamma reset landing seconds later is healed within ~1 s, not ~60 s.
        if layout_changed || woke_on_resume {
            settle_until = Some(Instant::now() + SETTLE_WINDOW);
        }
        if last_tick.elapsed() >= TICK_INTERVAL {
            last_tick = Instant::now();
        }
    }
    Ok(())
}

/// The poll timeout: the time left until the next tick, but capped at
/// [`SETTLE_INTERVAL`] while the layout is settling so a silent, eventless
/// gamma reset is overwritten within a second.
fn poll_timeout(tick_remaining: Duration, settling: bool) -> Duration {
    if settling {
        tick_remaining.min(SETTLE_INTERVAL)
    } else {
        tick_remaining
    }
}

/// Applies, degrading quietly on per-request X errors: a CRTC can vanish
/// between fetching the screen resources and the per-CRTC round trips (a
/// monitor unplugged mid-apply returns BadCrtc), and that must not kill the
/// daemon — the next tick retries against fresh resources. Only the loss of
/// the X connection itself is fatal, since nothing can be applied or restored
/// without it.
fn try_apply<C: Connection>(conn: &C, root: u32, state: &Shared) -> Result<(), Box<dyn Error>> {
    match apply_desired(conn, root, state) {
        Ok(()) => Ok(()),
        Err(error) if is_connection_error(error.as_ref()) => Err(error),
        Err(error) => {
            tracing::warn!("could not apply (will retry on the next tick): {error}");
            Ok(())
        }
    }
}

/// Whether `error` means the X connection itself is gone, as opposed to a
/// single request failing against hardware that changed under us.
fn is_connection_error(error: &(dyn Error + 'static)) -> bool {
    use x11rb::errors::{ConnectionError, ReplyError, ReplyOrIdError};
    error.downcast_ref::<ConnectionError>().is_some()
        || matches!(
            error.downcast_ref::<ReplyError>(),
            Some(ReplyError::ConnectionError(_))
        )
        || matches!(
            error.downcast_ref::<ReplyOrIdError>(),
            Some(ReplyOrIdError::ConnectionError(_))
        )
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

/// Applies the temperature the current state calls for, and records it — plus
/// the outputs it landed on — back into the state (without holding the lock
/// across the X writes).
fn apply_desired<C: Connection>(conn: &C, root: u32, state: &Shared) -> Result<(), Box<dyn Error>> {
    let (target, previous) = {
        let mut state = lock(state);
        let target = desired_temp(&mut state);
        (target, state.current_temp)
    };
    let crtcs = reapply(conn, root, target, target != previous)?;
    let mut state = lock(state);
    state.current_temp = target;
    state.outputs = crtcs.iter().map(|c| (c.crtc, c.gamma_size)).collect();
    Ok(())
}

/// The temperature the state calls for: neutral when disabled, the manual
/// override when set, otherwise the sun-based target for right now.
///
/// In automatic mode the timezone lookup is refreshed into the state's
/// location cache when it succeeds and reused from there when it transiently
/// fails, so a momentary failure never resets the screen to neutral
/// (`day_temp`). Only a location that has never resolved falls back to
/// `day_temp`. `GetStatus` reads the same cache.
fn desired_temp(state: &mut State) -> u32 {
    if !state.enabled {
        return NEUTRAL_KELVIN;
    }
    if let Some(kelvin) = state.override_temp {
        return kelvin;
    }
    if !matches!(state.mode, Mode::Automatic) {
        return resolve_temperature(state.mode, unix_now(), state.day_temp, state.night_temp);
    }
    if let Some(resolved) = location_from_timezone() {
        state.location = Some(resolved);
    }
    match state.location {
        Some((lat, lon)) => resolve_temperature(
            Mode::ManualLocation { lat, lon },
            unix_now(),
            state.day_temp,
            state.night_temp,
        ),
        None => state.day_temp,
    }
}

/// Seconds since the Unix epoch, as an `f64` for the solar maths. Degrades to
/// `0.0` rather than panicking if the clock is somehow before the epoch.
pub(crate) fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

/// Re-reads the current CRTCs, writes the `kelvin` ramp to each, and returns
/// what it wrote to. Re-reading means a newly-attached monitor is covered too
/// (issue #14).
fn reapply<C: Connection>(
    conn: &C,
    root: u32,
    kelvin: u32,
    changed: bool,
) -> Result<Vec<CrtcInfo>, Box<dyn Error>> {
    let resources = conn.randr_get_screen_resources(root)?.reply()?;
    let crtcs = active_crtcs(conn, &resources)?;
    write_ramps(conn, &crtcs, kelvin, changed)?;
    conn.flush()?;
    Ok(crtcs)
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

#[cfg(test)]
mod tests {
    use super::*;
    use x11rb::errors::{ConnectionError, ReplyError};
    use x11rb::protocol::ErrorKind;
    use x11rb::x11_utils::X11Error;

    #[test]
    fn connection_loss_is_fatal() {
        assert!(is_connection_error(&ConnectionError::UnknownError));
        assert!(is_connection_error(&ReplyError::ConnectionError(
            ConnectionError::UnknownError
        )));
    }

    #[test]
    fn settling_caps_the_poll_timeout_but_never_extends_it() {
        let tick_remaining = Duration::from_secs(42);
        // Not settling: wait the full time left until the tick.
        assert_eq!(poll_timeout(tick_remaining, false), tick_remaining);
        // Settling: poll fast, capped at the settle interval.
        assert_eq!(poll_timeout(tick_remaining, true), SETTLE_INTERVAL);
        // Settling never lengthens a wait already shorter than the interval
        // (e.g. the tick is about to fire).
        let almost_due = Duration::from_millis(200);
        assert_eq!(poll_timeout(almost_due, true), almost_due);
    }

    #[test]
    fn a_protocol_error_is_not_fatal() {
        // The BadCrtc case: a monitor vanished mid-apply. RandR extension
        // errors carry extension-specific codes; any X11Error goes down the
        // retry path, not the fatal one.
        let bad_request = X11Error {
            error_kind: ErrorKind::Request,
            error_code: 1,
            sequence: 0,
            bad_value: 0,
            minor_opcode: 0,
            major_opcode: 0,
            extension_name: None,
            request_name: None,
        };
        assert!(!is_connection_error(&ReplyError::X11Error(bad_request)));
        let io = std::io::Error::other("unrelated");
        assert!(!is_connection_error(&io));
    }
}
