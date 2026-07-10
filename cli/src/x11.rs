//! The X11 / XRandR backend (issue #10 onward).
//!
//! `core` never touches the screen; everything that talks to the X server
//! lives here. It discovers the CRTCs and their gamma-ramp sizes (which differ
//! between outputs — 256, 1024, 2048 — and must be read, not assumed) and
//! writes ramps to them.

use std::error::Error;

use nightlightd_core::color::{build_ramp, temperature_to_rgb};
use x11rb::connection::Connection;
use x11rb::protocol::randr::{ConnectionExt as _, GetScreenResourcesReply};

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

/// Writes the gamma ramp for `kelvin` to every active CRTC, and returns how
/// many were updated.
///
/// 6500 K produces the identity ramp, which restores the screen to normal.
pub fn apply_temperature(kelvin: u32) -> Result<usize, Box<dyn Error>> {
    let (conn, screen_num) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen_num].root;
    let resources = conn.randr_get_screen_resources(root)?.reply()?;
    let crtcs = active_crtcs(&conn, &resources)?;

    let gains = temperature_to_rgb(kelvin);
    for c in &crtcs {
        let ramp = build_ramp(c.gamma_size, gains);
        conn.randr_set_crtc_gamma(c.crtc, &ramp.red, &ramp.green, &ramp.blue)?
            .check()?;
    }

    Ok(crtcs.len())
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
