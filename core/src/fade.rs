//! Blending two ramp targets, for the fade walk (issue #38).
//!
//! A target change used to hit the screen in one write. The fade walks there
//! instead: the daemon asks, a few times a second, "where between the old
//! target and the new one am I now", and these two functions answer. They are
//! deliberately dumb — a position, not a schedule. Timing lives with the
//! caller; the maths here must hold for any alpha it is handed.
//!
//! The shaping factors walk linearly; the temperature walks in mired space
//! so it *looks* linear (see [`blend_temperature`]). Easing is applied by
//! the caller through [`smoothstep`], so the endpoints stay exact by
//! construction: alpha 0 is bit-for-bit the old target, alpha 1 the new.

/// A point between two temperatures, walked in mired (1,000,000 / kelvin),
/// the photographer's scale on which colour steps look evenly sized. Equal
/// kelvin steps are perceptually tiny near neutral and huge near candle
/// light; a linear kelvin walk therefore looks like it stalls and then
/// lurches. Walked in mired, the same fade reads as one steady glide.
///
/// Alpha outside 0..=1 clamps to the nearer endpoint; a non-finite alpha
/// lands on `to` — mid-fade is the wrong place to be stranded by a broken
/// clock, and the new target is where the fade was going anyway.
pub fn blend_temperature(from: u32, to: u32, alpha: f64) -> u32 {
    let alpha = sane_alpha(alpha);
    // `max(1)` guards the division; real temperatures are four digits.
    let from_mired = 1_000_000.0 / f64::from(from.max(1));
    let to_mired = 1_000_000.0 / f64::from(to.max(1));
    let mired = from_mired + alpha * (to_mired - from_mired);
    (1_000_000.0 / mired).round() as u32
}

/// A point on the straight line between two shaping factors (gamma or
/// brightness). Same alpha rules as [`blend_temperature`].
pub fn blend_factor(from: f64, to: f64, alpha: f64) -> f64 {
    let alpha = sane_alpha(alpha);
    from + alpha * (to - from)
}

/// The classic smoothstep ease, `3a² - 2a³`: starts slow, ends slow, hits
/// 0 and 1 exactly. Feed it the fade's linear progress and pass the result
/// to the blends, and a hard start/stop becomes a settle.
pub fn smoothstep(alpha: f64) -> f64 {
    let alpha = sane_alpha(alpha);
    alpha * alpha * (3.0 - 2.0 * alpha)
}

/// Clamps alpha into 0..=1 and turns non-finite into 1.0 (arrive, don't
/// strand).
fn sane_alpha(alpha: f64) -> f64 {
    if alpha.is_finite() {
        alpha.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn endpoints_are_exact() {
        assert_eq!(blend_temperature(6500, 1500, 0.0), 6500);
        assert_eq!(blend_temperature(6500, 1500, 1.0), 1500);
        assert!(approx(blend_factor(1.0, 0.9, 0.0), 1.0));
        assert!(approx(blend_factor(1.0, 0.9, 1.0), 0.9));
    }

    #[test]
    fn factor_midpoint_is_halfway() {
        assert!(approx(blend_factor(0.8, 1.0, 0.5), 0.9));
    }

    /// The temperature midpoint sits halfway in mired, not in kelvin: for
    /// 6500 to 1500 that is 2437.5 (warmer than the kelvin mean of 4000),
    /// because a kelvin step near candle light looks far bigger than the
    /// same step near neutral.
    #[test]
    fn temperature_midpoint_is_halfway_in_mired() {
        let mid = blend_temperature(6500, 1500, 0.5);
        assert!(mid == 2437 || mid == 2438, "got {mid}");
        let mired = |k: u32| 1_000_000.0 / f64::from(k);
        let want = (mired(6500) + mired(1500)) / 2.0;
        assert!((mired(mid) - want).abs() < 0.2);
    }

    #[test]
    fn walks_monotonically_in_both_directions() {
        let mut down = u32::MAX;
        let mut up = 0;
        let mut alpha = 0.0;
        while alpha <= 1.0 {
            let cooling = blend_temperature(6500, 1500, alpha);
            let warming = blend_temperature(1500, 6500, alpha);
            assert!(cooling <= down, "cooling walk rose at {alpha}");
            assert!(warming >= up, "warming walk fell at {alpha}");
            down = cooling;
            up = warming;
            alpha += 0.01;
        }
    }

    #[test]
    fn alpha_outside_the_window_clamps() {
        assert_eq!(blend_temperature(6500, 1500, -0.5), 6500);
        assert_eq!(blend_temperature(6500, 1500, 1.5), 1500);
        assert!(approx(blend_factor(1.0, 0.9, -1.0), 1.0));
        assert!(approx(blend_factor(1.0, 0.9, 2.0), 0.9));
    }

    /// A broken clock mid-fade must land on the target, never freeze between.
    #[test]
    fn silly_alpha_arrives_instead_of_stranding() {
        assert_eq!(blend_temperature(6500, 1500, f64::NAN), 1500);
        assert_eq!(blend_temperature(6500, 1500, f64::INFINITY), 1500);
        assert!(approx(blend_factor(1.0, 0.9, f64::NAN), 0.9));
    }

    #[test]
    fn smoothstep_hits_the_endpoints_and_eases_symmetrically() {
        assert!(approx(smoothstep(0.0), 0.0));
        assert!(approx(smoothstep(1.0), 1.0));
        assert!(approx(smoothstep(0.5), 0.5));
        // Symmetric: as much gained by 0.25 as remains after 0.75.
        assert!(approx(smoothstep(0.25) + smoothstep(0.75), 1.0));
        // Eased: the first quarter covers less ground than a linear walk.
        assert!(smoothstep(0.25) < 0.25);
    }

    #[test]
    fn smoothstep_rises_monotonically() {
        let mut previous = -1.0;
        let mut alpha = 0.0;
        while alpha <= 1.0 {
            let eased = smoothstep(alpha);
            assert!(eased >= previous, "smoothstep fell at {alpha}");
            previous = eased;
            alpha += 0.01;
        }
    }

    /// The composition the daemon will actually run: eased blending still
    /// lands exactly on the endpoints.
    #[test]
    fn eased_blend_keeps_exact_endpoints() {
        assert_eq!(blend_temperature(6500, 1500, smoothstep(0.0)), 6500);
        assert_eq!(blend_temperature(6500, 1500, smoothstep(1.0)), 1500);
    }
}
