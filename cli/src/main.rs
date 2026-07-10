//! `nightlightd` — a screen colour temperature daemon for X11.
//!
//! One binary, two modes: `--daemon` runs the daemon, a bare invocation such
//! as `--temp 2800` acts as a client and messages the daemon over D-Bus.
//!
//! Neither mode exists yet. For now (issue #10) the binary probes the X server
//! and reports the CRTCs it found; argument parsing and writing ramps arrive
//! from #11 onward.

mod x11;

fn main() {
    match x11::discover() {
        Ok(crtcs) => report(&crtcs),
        Err(error) => {
            eprintln!("nightlightd: cannot query the X server: {error}");
            std::process::exit(1);
        }
    }
}

/// Prints the discovered CRTCs and their gamma-ramp sizes.
fn report(crtcs: &[x11::CrtcInfo]) {
    println!("found {} active CRTC(s)", crtcs.len());
    for c in crtcs {
        println!("  CRTC {}: ramp size {}", c.crtc, c.gamma_size);
    }
}
