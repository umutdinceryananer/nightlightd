//! The D-Bus client: a short-lived invocation that messages a running daemon
//! (#20). No `--daemon` flag means the binary acts as a client.

use zbus::blocking::Connection;
use zbus::proxy;

use nightlightd_core::bus::{self, Service};
use nightlightd_core::solar::sun_is_rising;
use nightlightd_core::transition::{Band, Bands, phase};

use crate::status::Status;

/// A blocking proxy for the daemon's interface. The `proxy` macro generates
/// `DaemonProxyBlocking` from these method signatures.
///
/// No `default_service`: the daemon's bus name depends on whether this is a
/// Flatpak (#55), so every proxy is handed it from `core::bus` and none can
/// fall back to a name written here.
#[proxy(
    interface = "org.nightlightd.Daemon",
    default_path = "/org/nightlightd/Daemon"
)]
trait Daemon {
    fn set_temperature(&self, kelvin: u32) -> zbus::Result<()>;
    fn set_enabled(&self, enabled: bool) -> zbus::Result<()>;
    fn toggle(&self) -> zbus::Result<()>;
    fn set_mode(&self, mode: &str) -> zbus::Result<()>;
    fn set_gamma(&self, gamma: f64) -> zbus::Result<()>;
    fn set_fade(&self, fade: bool) -> zbus::Result<()>;
    fn set_transition_band(&self, day_elevation: f64, night_elevation: f64) -> zbus::Result<()>;
    fn set_transition_bands(
        &self,
        dawn_day_elevation: f64,
        dawn_night_elevation: f64,
        dusk_day_elevation: f64,
        dusk_night_elevation: f64,
    ) -> zbus::Result<()>;
    fn set_brightness(&self, day: f64, night: f64) -> zbus::Result<()>;
    fn get_status(&self) -> zbus::Result<Status>;
}

/// What the client was asked to do.
pub enum Request {
    /// Pin a fixed temperature (kelvin).
    SetTemperature(u32),
    /// Flip the filter on/off.
    Toggle,
    /// Turn the filter on or off.
    SetEnabled(bool),
    /// Return to following the sun.
    Auto,
    /// Set the gamma exponent (GitHub #2).
    SetGamma(f64),
    /// Set the day and night brightness bounds (GitHub #2).
    SetBrightness(f64, f64),
    /// Turn the fade walk on or off (#44).
    SetFade(bool),
    /// Set the transition band's elevation bounds (#39), morning and
    /// evening alike.
    SetBand(f64, f64),
    /// Set the morning's band alone (#47).
    SetDawn(f64, f64),
    /// Set the evening's band alone (#47).
    SetDusk(f64, f64),
    /// Print the daemon's status.
    Status,
}

/// Whether something owns the daemon's bus name right now. Asked only after
/// a failed request (#42): owned but failing means this binary and the
/// daemon are different versions, which deserves a different error than
/// "not running".
pub fn daemon_on_bus() -> bool {
    Connection::session()
        .ok()
        .and_then(|connection| zbus::blocking::fdo::DBusProxy::new(&connection).ok())
        .and_then(|fdo| {
            let name = zbus::names::BusName::try_from(bus::name(Service::Daemon)).ok()?;
            fdo.name_has_owner(name).ok()
        })
        .unwrap_or(false)
}

/// Sends `request` to the running daemon over the session bus.
pub fn send(request: Request) -> zbus::Result<()> {
    let connection = Connection::session()?;
    let proxy = DaemonProxyBlocking::new(&connection, bus::name(Service::Daemon))?;
    match request {
        Request::SetTemperature(kelvin) => proxy.set_temperature(kelvin),
        Request::Toggle => proxy.toggle(),
        Request::SetEnabled(enabled) => proxy.set_enabled(enabled),
        Request::Auto => proxy.set_mode("auto"),
        Request::SetGamma(gamma) => proxy.set_gamma(gamma),
        Request::SetBrightness(day, night) => proxy.set_brightness(day, night),
        Request::SetFade(fade) => proxy.set_fade(fade),
        Request::SetBand(day, night) => proxy.set_transition_band(day, night),
        // The other side is sent as NaN, which the daemon leaves alone: one
        // request rather than a read and a write with a gap between them.
        Request::SetDawn(day, night) => proxy.set_transition_bands(day, night, f64::NAN, f64::NAN),
        Request::SetDusk(day, night) => proxy.set_transition_bands(f64::NAN, f64::NAN, day, night),
        Request::Status => {
            print_status(&proxy.get_status()?, crate::x11::unix_now());
            Ok(())
        }
    }
}

/// The daemon snapshot as `--status` prints it: the headline on the first
/// line, then the details worth eyeballing indented under it.
///
/// Built as a string rather than printed line by line so it can be checked
/// without a daemon, and so the whole readout reaches the terminal in one
/// write.
fn status_text(status: &Status, now: f64) -> String {
    let mut out = String::new();
    macro_rules! line {
        ($($arg:tt)*) => {{
            out.push_str(&format!($($arg)*));
            out.push('\n');
        }};
    }
    let onoff = if status.enabled { "on" } else { "off" };
    line!("nightlightd: {onoff}, {} K", status.temperature);
    line!("  source: {}", status.source);
    // The shaping factors earn a line only when they do something.
    if (status.gamma - 1.0).abs() > 1e-9 || (status.brightness - 1.0).abs() > 1e-9 {
        line!(
            "  ramp:   gamma {:.2}, brightness {:.2} (day {:.2} / night {:.2})",
            status.gamma,
            status.brightness,
            status.day_brightness,
            status.night_brightness
        );
    }
    // Same rule as the ramp line: the default earns no ink.
    if !status.fade {
        line!("  fade:   off");
    }
    // Verbatim, as configured: the readout shows what the file says, and
    // core repairs what it cannot use where it is spent.
    let bands = Bands {
        dawn: Band {
            day_elevation: status.dawn_day_elevation,
            night_elevation: status.dawn_night_elevation,
        },
        dusk: Band {
            day_elevation: status.day_elevation,
            night_elevation: status.night_elevation,
        },
    };
    let band_line = |band: Band| {
        format!(
            "day above {:+.1}°, night below {:+.1}°",
            band.day_elevation, band.night_elevation
        )
    };
    if bands.split() {
        // Set apart (#47), each side earns its own line.
        line!("  dawn:   {}", band_line(bands.dawn));
        line!("  dusk:   {}", band_line(bands.dusk));
    } else if (status.day_elevation - 3.0).abs() > 1e-9
        || (status.night_elevation + 6.0).abs() > 1e-9
    {
        line!("  band:   {}", band_line(bands.dusk));
    }
    if status.has_location {
        // Named by the band this daemon runs now, morning's or evening's,
        // not by the one it shipped with.
        let rising = sun_is_rising(status.latitude, status.longitude, now);
        line!(
            "  sun:    {:+.1}° ({})",
            status.elevation,
            phase(status.elevation, bands.at(rising))
        );
        line!(
            "  place:  {:.2}, {:.2} (resolved)",
            status.latitude,
            status.longitude
        );
    }
    out
}

/// Prints it.
///
/// Written rather than `println!`ed because `println!` panics when the write
/// fails, and `nightlightd --status | head -1` fails it every time: `head`
/// closes the pipe as soon as it has its line. Panicking there is absurd —
/// the readout has already been delivered, and the reader asked for less of
/// it, not for a backtrace.
fn print_status(status: &Status, now: f64) {
    let _ = std::io::Write::write_all(&mut std::io::stdout(), status_text(status, now).as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2021-06-21 12:00 in Istanbul (+03), as unix time: a moment whose
    /// morning and evening are a known number of hours either side.
    const NOON: f64 = 1_624_266_000.0;

    fn following() -> Status {
        Status {
            enabled: true,
            temperature: 4200,
            source: "auto (following the sun)".into(),
            elevation: -2.5,
            has_location: true,
            latitude: 41.02,
            longitude: 28.97,
            following: true,
            day_temp: 6500,
            night_temp: 2700,
            gamma: 1.0,
            brightness: 1.0,
            day_brightness: 1.0,
            night_brightness: 1.0,
            fade: true,
            day_elevation: 3.0,
            night_elevation: -6.0,
            dawn_day_elevation: 3.0,
            dawn_night_elevation: -6.0,
        }
    }

    /// The readout's rule: a setting that is doing nothing earns no line.
    /// The alternative is a wall of defaults with the one changed number
    /// hidden in it.
    #[test]
    fn the_defaults_earn_no_ink() {
        let text = status_text(&following(), NOON);
        assert!(text.starts_with("nightlightd: on, 4200 K\n"));
        assert!(text.contains("  source: auto"));
        for absent in ["ramp:", "fade:", "band:"] {
            assert!(!text.contains(absent), "{absent} printed at its default");
        }
        // A resolved location is worth two lines; nothing else is.
        assert!(text.contains("  sun:"));
        assert!(text.contains("  place:  41.02, 28.97 (resolved)"));
        assert_eq!(text.lines().count(), 4);
    }

    /// And a setting that is doing something earns exactly one.
    #[test]
    fn a_changed_setting_earns_a_line() {
        let mut status = following();
        status.gamma = 0.9;
        status.fade = false;
        status.day_elevation = 1.5;
        status.night_elevation = -12.0;
        status.dawn_day_elevation = 1.5;
        status.dawn_night_elevation = -12.0;
        let text = status_text(&status, NOON);
        assert!(text.contains("  ramp:   gamma 0.90"));
        assert!(text.contains("  fade:   off"));
        assert!(text.contains("  band:   day above +1.5°, night below -12.0°"));
    }

    /// A morning set apart from the evening (#47) earns a line per side
    /// instead of the one band line, and the phase word comes from the side
    /// the sun is on.
    #[test]
    fn a_split_band_reads_as_dawn_and_dusk() {
        let mut status = following();
        status.dawn_day_elevation = 1.0;
        status.dawn_night_elevation = -2.0;
        status.night_elevation = -12.0;
        // Six in the evening, local, in Istanbul in late June: the sun has
        // gone down to about +6, still above both day bounds.
        let text = status_text(&status, NOON + 6.0 * 3600.0);
        assert!(!text.contains("band:"));
        assert!(text.contains("  dawn:   day above +1.0°, night below -2.0°"));
        assert!(text.contains("  dusk:   day above +3.0°, night below -12.0°"));

        // At -4 the morning's band says night and the evening's does not.
        status.elevation = -4.0;
        let morning = status_text(&status, NOON - 7.0 * 3600.0);
        let evening = status_text(&status, NOON + 7.0 * 3600.0);
        assert!(morning.contains("(night)"), "{morning}");
        assert!(evening.contains("(transition)"), "{evening}");
    }

    /// Nothing here reads the location fields when there is no location, so
    /// their placeholder values never reach the terminal.
    #[test]
    fn a_placeless_daemon_prints_no_coordinates() {
        let mut status = following();
        status.has_location = false;
        let text = status_text(&status, NOON);
        assert!(!text.contains("sun:"));
        assert!(!text.contains("place:"));
        assert_eq!(text.lines().count(), 2);
    }
}
