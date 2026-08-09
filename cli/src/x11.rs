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
//!   write, some fullscreen games, DPMS wakeups). Since #40 the tick reads
//!   each ramp before writing it, so a screen already carrying ours costs
//!   nothing and a screen that has lost it says so.

use std::error::Error;
use std::os::fd::{AsFd, BorrowedFd};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nightlightd_core::color::{build_ramp, temperature_to_rgb};
use nightlightd_core::location::location_from_timezone;
use nightlightd_core::mode::Mode;
use nightlightd_core::solar::solar_elevation;
use nightlightd_core::transition::{target_brightness, target_temperature};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::randr::{ConnectionExt as _, GetScreenResourcesReply, NotifyMask};

use crate::fade::{FADE_TICK, Fade};
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

/// Everything one ramp write needs (GitHub #2): the temperature plus the two
/// shaping factors, derived together in one place so every path that touches
/// the screen agrees on what it should show.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Target {
    pub(crate) kelvin: u32,
    pub(crate) gamma: f64,
    pub(crate) brightness: f64,
}

/// The screen a disabled filter leaves behind: fully neutral, factors and
/// all, the behaviour gammastep users already expect from a kill.
const NEUTRAL_TARGET: Target = Target {
    kelvin: NEUTRAL_KELVIN,
    gamma: 1.0,
    brightness: 1.0,
};

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
    // Identity factors on purpose: this is the restore/oneshot path, and a
    // restore must hand back a fully neutral screen.
    let target = Target {
        kelvin,
        gamma: 1.0,
        brightness: 1.0,
    };
    // A restore has no history to compare against — this process has written
    // nothing — so it declares itself moving and unsettled, and any
    // difference it finds reads as its own rather than as a wipe. A screen
    // that already carries the ramp needs no restoring.
    let intent = Intent {
        moving: true,
        changed: true,
        settled: false,
    };
    write_ramps(&conn, &crtcs, target, intent)?;
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

    // The fade in flight, if any (#38). Owned by the loop: the daemon starts
    // believing the screen neutral, so a night-time start eases in instead of
    // snapping.
    let mut fade: Option<Fade> = None;
    try_apply(&conn, root, state, &mut fade)?;

    let mut last_tick = Instant::now();
    // While `Some(deadline)` and not yet past it, poll fast to overwrite a
    // silent gamma reset that emits no event (see [`SETTLE_WINDOW`]).
    let mut settle_until: Option<Instant> = None;
    while !terminate.load(Ordering::Relaxed) {
        let tick_remaining = TICK_INTERVAL.saturating_sub(last_tick.elapsed());
        let settling = settle_until.is_some_and(|deadline| Instant::now() < deadline);
        if !wait_for_change(
            &[conn.stream().as_fd(), waker.as_fd()],
            poll_timeout(tick_remaining, settling, fade.is_some()),
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
        try_apply(&conn, root, state, &mut fade)?;
        // Events that raced in during our own round trips would otherwise wake
        // the loop again at once for a full extra pass; absorb them with one
        // bounded re-apply instead (never a loop — a storm settles on the tick).
        if drain_screen_changes(&conn)? {
            layout_changed = true;
            try_apply(&conn, root, state, &mut fade)?;
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

/// The poll timeout: the time left until the next tick, capped at
/// [`SETTLE_INTERVAL`] while the layout is settling so a silent, eventless
/// gamma reset is overwritten within a second, and capped harder at
/// [`FADE_TICK`] while a fade is walking so it advances a few times a
/// second (#38). Idle steady state stays the full tick, 0% CPU.
fn poll_timeout(tick_remaining: Duration, settling: bool, fading: bool) -> Duration {
    let mut timeout = tick_remaining;
    if settling {
        timeout = timeout.min(SETTLE_INTERVAL);
    }
    if fading {
        timeout = timeout.min(FADE_TICK);
    }
    timeout
}

/// Applies, degrading quietly on per-request X errors: a CRTC can vanish
/// between fetching the screen resources and the per-CRTC round trips (a
/// monitor unplugged mid-apply returns BadCrtc), and that must not kill the
/// daemon — the next tick retries against fresh resources. Only the loss of
/// the X connection itself is fatal, since nothing can be applied or restored
/// without it.
fn try_apply<C: Connection>(
    conn: &C,
    root: u32,
    state: &Shared,
    fade: &mut Option<Fade>,
) -> Result<(), Box<dyn Error>> {
    match apply_desired(conn, root, state, fade) {
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
fn apply_desired<C: Connection>(
    conn: &C,
    root: u32,
    state: &Shared,
    fade: &mut Option<Fade>,
) -> Result<(), Box<dyn Error>> {
    let now = Instant::now();
    let (desired, applied, fade_enabled, settled) = {
        let mut state = lock(state);
        let desired = desired_target(&mut state);
        let applied = Target {
            kelvin: state.current_temp,
            gamma: state.current_gamma,
            brightness: state.current_brightness,
        };
        // The outputs cache is empty until the first apply, which makes it
        // exactly the "have we written here yet" flag #40 needs: before that,
        // whatever is on the screen belongs to whoever ran last, not to us.
        (desired, applied, state.fade, !state.outputs.is_empty())
    };

    // Keep the walk pointed at what the state wants (#38); with the switch
    // off (#44) there is never a walk and every change lands whole.
    *fade = crate::fade::advance(fade.take(), fade_enabled, applied, desired, now);

    let target = fade.as_ref().map_or(desired, |active| active.at(now));
    // One info line per transition, at arrival; the intermediate steps of a
    // walk and the unchanged ticks stay at debug.
    let moving = target != applied;
    let intent = Intent {
        moving,
        changed: moving && fade.is_none(),
        settled,
    };
    let crtcs = reapply(conn, root, target, intent)?;
    let mut state = lock(state);
    state.current_temp = target.kelvin;
    state.current_gamma = target.gamma;
    state.current_brightness = target.brightness;
    state.outputs = crtcs.iter().map(|c| (c.crtc, c.gamma_size)).collect();
    Ok(())
}

/// The target the state calls for: fully neutral when disabled; otherwise
/// the sun's temperature and brightness plus the constant gamma. A manual
/// temperature override pins the kelvin but leaves the calibration gamma and
/// the sun-driven brightness alone, so holding a temperature never strips a
/// dim evening screen back to full blast.
fn desired_target(state: &mut State) -> Target {
    if !state.enabled {
        return NEUTRAL_TARGET;
    }
    let elevation = current_elevation(state);
    let brightness = match elevation {
        Some(elevation) => target_brightness(
            elevation,
            state.band,
            state.day_brightness,
            state.night_brightness,
        ),
        None => state.day_brightness,
    };
    let kelvin = if let Some(kelvin) = state.override_temp {
        kelvin
    } else if let Mode::Fixed(kelvin) = state.mode {
        kelvin
    } else {
        match elevation {
            Some(elevation) => {
                target_temperature(elevation, state.band, state.day_temp, state.night_temp)
            }
            None => state.day_temp,
        }
    };
    Target {
        kelvin,
        gamma: state.gamma,
        brightness,
    }
}

/// The sun's elevation right now, when a location can be had: a manual
/// mode's pinned coordinates, or the timezone lookup refreshed into the
/// state's cache on success and reused from there on a transient failure, so
/// a momentary failure never resets the screen. `None` only for a location
/// that has never resolved (or a fixed mode, which has none to offer).
fn current_elevation(state: &mut State) -> Option<f64> {
    let (lat, lon) = match state.mode {
        Mode::ManualLocation { lat, lon } => (lat, lon),
        Mode::Fixed(_) => return None,
        Mode::Automatic => {
            if let Some(resolved) = location_from_timezone() {
                state.location = Some(resolved);
            }
            state.location?
        }
    };
    Some(solar_elevation(lat, lon, unix_now()))
}

/// Seconds since the Unix epoch, as an `f64` for the solar maths. Degrades to
/// `0.0` rather than panicking if the clock is somehow before the epoch.
pub(crate) fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

/// Re-reads the current CRTCs, writes the target's ramp to each, and returns
/// what it wrote to. Re-reading means a newly-attached monitor is covered too
/// (issue #14).
fn reapply<C: Connection>(
    conn: &C,
    root: u32,
    target: Target,
    intent: Intent,
) -> Result<Vec<CrtcInfo>, Box<dyn Error>> {
    let resources = conn.randr_get_screen_resources(root)?.reply()?;
    let crtcs = active_crtcs(conn, &resources)?;
    write_ramps(conn, &crtcs, target, intent)?;
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

/// What the caller knows about a write, which is what lets a difference found
/// on the screen be read as ours or as somebody else's.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Intent {
    /// The ramp we are about to write differs from the one we last wrote, so
    /// a difference on the screen is ours. Not the same question as
    /// `changed`: a fade moves the ramp every step while deliberately staying
    /// quiet in the log, and every one of those steps is still ours.
    moving: bool,
    /// Worth an info line rather than a debug one: an arrival, not a step.
    changed: bool,
    /// We have written to this screen before. Until we have, whatever is on
    /// it is the state we found rather than something that was taken from us.
    settled: bool,
}

/// What a tick found on a CRTC.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Found {
    /// The screen already carries this exact ramp. Nothing to write, which is
    /// the common case once the sun stops moving.
    Unchanged,
    /// A difference we asked for.
    Applying,
    /// A difference nobody asked for, on a screen we had already written to:
    /// something else took the ramp between ticks. This is precisely the case
    /// the RandR events (#13) can miss, because a bare gamma write emits none.
    Wiped,
}

/// Reads what was found on the screen against what we meant to put there.
///
/// The whole of #40 is this table. A ramp that already matches is a write we
/// can skip; a ramp that differs is either our own doing or a wipe, and the
/// difference between those two is whether we asked for it — never whether it
/// is merely different, which is what a naive delta write would have said on
/// every fade step and on the very first apply.
fn classify(matches: bool, intent: Intent) -> Found {
    if matches {
        Found::Unchanged
    } else if intent.moving || !intent.settled {
        Found::Applying
    } else {
        Found::Wiped
    }
}

/// Builds the target's ramp for each CRTC and writes it — unless the screen
/// already carries it (#40). The read costs one round trip per CRTC per tick
/// and buys the cheap half of the recovery story: a wipe that fires no event
/// is corrected on the next tick instead of lasting until something else
/// happens to wake the loop.
fn write_ramps<C: Connection>(
    conn: &C,
    crtcs: &[CrtcInfo],
    target: Target,
    intent: Intent,
) -> Result<(), Box<dyn Error>> {
    let gains = temperature_to_rgb(target.kelvin);
    let kelvin = target.kelvin;
    let changed = intent.changed;
    // Shaping factors appear in the log line only when they do something,
    // so the common log stays as short as it always was.
    let shaped = if (target.gamma - 1.0).abs() > 1e-9 || (target.brightness - 1.0).abs() > 1e-9 {
        format!(
            " (gamma {:.2}, brightness {:.2})",
            target.gamma, target.brightness
        )
    } else {
        String::new()
    };
    for c in crtcs {
        let ramp = build_ramp(c.gamma_size, gains, target.gamma, target.brightness);
        // A read that fails is not evidence of a match, so it falls through to
        // the write. Degrading toward writing is the safe direction: the worst
        // an unnecessary write costs is a round trip, and the worst a skipped
        // one costs is a screen left wrong.
        let live = conn
            .randr_get_crtc_gamma(c.crtc)
            .ok()
            .and_then(|cookie| cookie.reply().ok());
        let matches = live.is_some_and(|live| {
            live.red == ramp.red && live.green == ramp.green && live.blue == ramp.blue
        });
        match classify(matches, intent) {
            Found::Unchanged => {
                tracing::debug!("CRTC {} already at {kelvin} K{shaped}", c.crtc);
                continue;
            }
            // Rare and worth saying out loud whatever else the tick was
            // doing: it is the one line that tells you the safety net caught
            // something.
            Found::Wiped => {
                tracing::info!(
                    "CRTC {} lost its ramp to something else; rewrote {kelvin} K{shaped}",
                    c.crtc
                );
            }
            // A change (sun moved, a client request) is logged by default; a
            // fade's intermediate steps stay at debug.
            Found::Applying if changed => {
                tracing::info!("applied {kelvin} K{shaped} to CRTC {}", c.crtc);
            }
            Found::Applying => {
                tracing::debug!("applied {kelvin} K{shaped} to CRTC {}", c.crtc);
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use x11rb::errors::{ConnectionError, ReplyError};
    use x11rb::protocol::ErrorKind;
    use x11rb::x11_utils::X11Error;

    fn intent(moving: bool, settled: bool) -> Intent {
        Intent {
            moving,
            changed: moving,
            settled,
        }
    }

    /// The whole of #40 in one table. Two of these four rows are the ones a
    /// naive "write only on difference" would have got wrong, and both would
    /// have been wrong loudly: a fade would have reported itself wiped on
    /// every step, and so would the first apply of a daemon taking over a
    /// screen somebody else had left coloured.
    #[test]
    fn a_difference_is_only_a_wipe_when_nobody_asked_for_it() {
        // A screen already carrying the ramp is a write we can skip, whatever
        // else the tick believed about itself.
        for moving in [true, false] {
            for settled in [true, false] {
                assert_eq!(
                    classify(true, intent(moving, settled)),
                    Found::Unchanged,
                    "moving={moving} settled={settled}"
                );
            }
        }
        // Moving: the difference is ours, fade step or arrival alike.
        assert_eq!(classify(false, intent(true, true)), Found::Applying);
        // Not moving and never written here: this is the state we found, not
        // something taken from us.
        assert_eq!(classify(false, intent(false, false)), Found::Applying);
        // Not moving, and we had written here — the one case the RandR events
        // can miss, and the only one worth a line in the log.
        assert_eq!(classify(false, intent(false, true)), Found::Wiped);
    }

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
        assert_eq!(poll_timeout(tick_remaining, false, false), tick_remaining);
        // Settling: poll fast, capped at the settle interval.
        assert_eq!(poll_timeout(tick_remaining, true, false), SETTLE_INTERVAL);
        // Settling never lengthens a wait already shorter than the interval
        // (e.g. the tick is about to fire).
        let almost_due = Duration::from_millis(200);
        assert_eq!(poll_timeout(almost_due, true, false), almost_due);
    }

    /// A walking fade wakes the loop a few times a second, beats the settle
    /// cap, and never extends a wait that is already shorter.
    #[test]
    fn fading_caps_the_poll_timeout_hardest() {
        let tick_remaining = Duration::from_secs(42);
        assert_eq!(poll_timeout(tick_remaining, false, true), FADE_TICK);
        assert_eq!(poll_timeout(tick_remaining, true, true), FADE_TICK);
        // Already due sooner than a fade step: keep the shorter wait.
        let almost_due = Duration::from_millis(20);
        assert_eq!(poll_timeout(almost_due, true, true), almost_due);
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
