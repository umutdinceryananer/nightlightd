//! The X11 / XRandR backend (issue #10 onward).
//!
//! `core` never touches the screen; everything that talks to the X server
//! lives here. This module starts with discovery — connecting and enumerating
//! the CRTCs and their gamma-ramp sizes — because those sizes differ between
//! outputs (256, 1024, 2048) and must be read, not assumed.

use std::error::Error;

use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;

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

    // `get_screen_resources` (rather than `..._current`) re-polls the outputs,
    // so a monitor attached after startup is actually seen. Using the "current"
    // variant is why gammastep misses hotplugged displays (issue #13).
    let resources = conn.randr_get_screen_resources(root)?.reply()?;

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
