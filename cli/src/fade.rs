//! The fade walk (issue #38): a target change eases onto the screen over a
//! couple of seconds instead of landing in one write.
//!
//! This is only the bookkeeping — where between the old target and the new
//! one the screen should be *right now*. The event loop asks, writes what it
//! is told, and asks again a beat later; nothing here touches X11 or sleeps.
//! Time comes in through every method rather than being read here, so the
//! whole walk is testable in zero wall-clock seconds.
//!
//! Position is computed from elapsed time, not counted in steps: a late wake
//! (a busy machine, a slow X round trip) lands further along the same curve
//! instead of stretching the fade.

use std::time::{Duration, Instant};

use nightlightd_core::fade::{blend_factor, blend_temperature, smoothstep};

use crate::x11::Target;

/// How long a fade takes, start to settle.
pub(crate) const FADE_DURATION: Duration = Duration::from_millis(2000);

/// How often the loop should wake to advance an active fade. 50 ms keeps
/// the biggest step of a full 6500-to-1500 walk under the eye's notice;
/// 100 ms was visibly stepped on exactly that span.
pub(crate) const FADE_TICK: Duration = Duration::from_millis(50);

/// One wake's worth of fade bookkeeping: keeps the walk pointed at what the
/// state wants. Starts one when the desire moves off the applied target,
/// retargets a walk in flight from the exact point reached, drops it once
/// arrived — and drops it flat when the switch (#44) is off, so the next
/// write lands on the destination in one step.
pub(crate) fn advance(
    current: Option<Fade>,
    enabled: bool,
    applied: Target,
    desired: Target,
    now: Instant,
) -> Option<Fade> {
    if !enabled {
        return None;
    }
    match current {
        Some(active) if active.destination() == desired => {
            if active.done(now) {
                None
            } else {
                Some(active)
            }
        }
        Some(active) => active.retarget(desired, now),
        None => Fade::toward(applied, desired, now),
    }
}

/// One fade in flight: from where, to where, since when.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Fade {
    from: Target,
    to: Target,
    started: Instant,
}

impl Fade {
    /// A fade from `from` to `to`, or `None` when there is nothing to walk —
    /// equal targets must not schedule two seconds of pointless wakes.
    pub(crate) fn toward(from: Target, to: Target, now: Instant) -> Option<Self> {
        if from == to {
            return None;
        }
        Some(Self {
            from,
            to,
            started: now,
        })
    }

    /// Where the screen should be at `now`: the eased blend of the two
    /// targets. Exactly `from` at the start, exactly `to` from the end of
    /// the walk onward.
    pub(crate) fn at(&self, now: Instant) -> Target {
        let alpha = smoothstep(self.progress(now));
        Target {
            kelvin: blend_temperature(self.from.kelvin, self.to.kelvin, alpha),
            gamma: blend_factor(self.from.gamma, self.to.gamma, alpha),
            brightness: blend_factor(self.from.brightness, self.to.brightness, alpha),
        }
    }

    /// Whether the walk has arrived. The `at` for a done fade is `to`.
    pub(crate) fn done(&self, now: Instant) -> bool {
        self.progress(now) >= 1.0
    }

    /// The fade's destination, for deciding whether a new desired target
    /// actually changes anything.
    pub(crate) fn destination(&self) -> Target {
        self.to
    }

    /// A new destination mid-walk: the fade restarts from wherever the
    /// screen is right now, so there is no jump and no queue. `None` when
    /// the walk is already headed exactly there.
    pub(crate) fn retarget(&self, to: Target, now: Instant) -> Option<Self> {
        Self::toward(self.at(now), to, now)
    }

    /// Linear progress through the walk, 0.0 to 1.0. A `now` before
    /// `started` (impossible with a monotonic clock, cheap to be safe about)
    /// counts as not started.
    fn progress(&self, now: Instant) -> f64 {
        let elapsed = now.saturating_duration_since(self.started);
        (elapsed.as_secs_f64() / FADE_DURATION.as_secs_f64()).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WARM: Target = Target {
        kelvin: 1500,
        gamma: 0.95,
        brightness: 0.9,
    };
    const NEUTRAL: Target = Target {
        kelvin: 6500,
        gamma: 1.0,
        brightness: 1.0,
    };

    #[test]
    fn equal_targets_are_no_fade() {
        let now = Instant::now();
        assert!(Fade::toward(WARM, WARM, now).is_none());
    }

    #[test]
    fn starts_exactly_at_from() {
        let now = Instant::now();
        let fade = Fade::toward(NEUTRAL, WARM, now).unwrap();
        assert_eq!(fade.at(now), NEUTRAL);
        assert!(!fade.done(now));
    }

    #[test]
    fn arrives_exactly_at_to_and_stays_there() {
        let now = Instant::now();
        let fade = Fade::toward(NEUTRAL, WARM, now).unwrap();
        let end = now + FADE_DURATION;
        assert_eq!(fade.at(end), WARM);
        assert!(fade.done(end));
        // Long past the end, still the destination, still done.
        let late = now + FADE_DURATION * 10;
        assert_eq!(fade.at(late), WARM);
        assert!(fade.done(late));
    }

    #[test]
    fn walks_monotonically_warmer() {
        let now = Instant::now();
        let fade = Fade::toward(NEUTRAL, WARM, now).unwrap();
        let mut previous = u32::MAX;
        for tenth in 0..=20 {
            let at = fade.at(now + FADE_DURATION * tenth / 20);
            assert!(at.kelvin <= previous, "kelvin rose at step {tenth}");
            assert!(at.kelvin >= WARM.kelvin && at.kelvin <= NEUTRAL.kelvin);
            previous = at.kelvin;
        }
    }

    #[test]
    fn midwalk_is_strictly_between() {
        let now = Instant::now();
        let fade = Fade::toward(NEUTRAL, WARM, now).unwrap();
        let mid = fade.at(now + FADE_DURATION / 2);
        assert!(mid.kelvin > WARM.kelvin && mid.kelvin < NEUTRAL.kelvin);
        assert!(mid.gamma > 0.95 && mid.gamma < 1.0);
        assert!(mid.brightness > 0.9 && mid.brightness < 1.0);
    }

    /// The no-jump guarantee: retargeting mid-walk starts the new fade at
    /// the exact point the old one had reached.
    #[test]
    fn retarget_continues_from_the_current_point() {
        let now = Instant::now();
        let fade = Fade::toward(NEUTRAL, WARM, now).unwrap();
        let halfway = now + FADE_DURATION / 2;
        let reached = fade.at(halfway);
        let back = fade.retarget(NEUTRAL, halfway).unwrap();
        assert_eq!(back.at(halfway), reached);
        assert_eq!(back.destination(), NEUTRAL);
        assert_eq!(back.at(halfway + FADE_DURATION), NEUTRAL);
    }

    #[test]
    fn retarget_to_the_same_destination_mid_walk_still_walks() {
        let now = Instant::now();
        let fade = Fade::toward(NEUTRAL, WARM, now).unwrap();
        let halfway = now + FADE_DURATION / 2;
        // Same destination: a fresh walk from the current point, not None —
        // the screen is not there yet.
        let again = fade.retarget(WARM, halfway).unwrap();
        assert_eq!(again.at(halfway), fade.at(halfway));
    }

    #[test]
    fn retarget_at_arrival_is_no_fade() {
        let now = Instant::now();
        let fade = Fade::toward(NEUTRAL, WARM, now).unwrap();
        let end = now + FADE_DURATION;
        assert!(fade.retarget(WARM, end).is_none());
    }

    /// A wake before the start (a clock that cannot happen, cheaply survived)
    /// reads as the starting point.
    #[test]
    fn a_now_before_started_is_the_start() {
        let now = Instant::now();
        let fade = Fade::toward(NEUTRAL, WARM, now + Duration::from_secs(5)).unwrap();
        assert_eq!(fade.at(now), NEUTRAL);
    }

    /// The switch off (#44) means no walk ever starts and a walk in flight
    /// dies on the next wake, so that write lands on the destination whole.
    #[test]
    fn the_switch_off_kills_walks_present_and_future() {
        let now = Instant::now();
        assert!(advance(None, false, NEUTRAL, WARM, now).is_none());
        let inflight = Fade::toward(NEUTRAL, WARM, now);
        let mid = now + FADE_DURATION / 2;
        assert!(advance(inflight, false, NEUTRAL, WARM, mid).is_none());
    }

    /// With the switch on, advance is the same walk lifecycle the loop ran
    /// before the switch existed: start, hold, retarget, arrive, drop.
    #[test]
    fn the_switch_on_runs_the_walk_lifecycle() {
        let now = Instant::now();
        let started = advance(None, true, NEUTRAL, WARM, now);
        assert!(started.is_some());
        let mid = now + FADE_DURATION / 2;
        let held = advance(started, true, NEUTRAL, WARM, mid);
        assert_eq!(held.unwrap().destination(), WARM);
        let held = advance(held, true, NEUTRAL, WARM, mid);
        let turned = advance(held, true, NEUTRAL, NEUTRAL, mid);
        assert_eq!(turned.unwrap().destination(), NEUTRAL);
        let arrived = advance(turned, true, NEUTRAL, NEUTRAL, mid + FADE_DURATION);
        assert!(arrived.is_none());
        // Applied equals desired and nothing in flight: nothing to walk.
        assert!(advance(None, true, WARM, WARM, now).is_none());
    }
}
