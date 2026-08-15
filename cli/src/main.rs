//! `nightlightd` — a screen colour temperature daemon for X11.
//!
//! One binary, two modes: `--daemon` runs the daemon (follow the sun); any
//! other invocation acts as a client and messages the running daemon over
//! D-Bus. With no arguments it reports the CRTCs it found (a diagnostic).

mod client;
mod config;
mod dbus;
mod fade;
mod import;
mod state;
mod status;
mod suspend;
mod waker;
mod x11;

use std::error::Error;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use clap::{ArgGroup, Parser};
use signal_hook::consts::{SIGINT, SIGTERM};
use tracing_subscriber::EnvFilter;

/// Screen colour temperature daemon for X11.
#[derive(Parser)]
#[command(name = "nightlightd", version, about)]
#[command(group(ArgGroup::new("action").args(["temp", "toggle", "on", "off", "auto", "gamma", "brightness", "fade", "band", "status"])))]
struct Cli {
    /// Run the daemon: follow the sun continuously.
    #[arg(long, conflicts_with = "action")]
    daemon: bool,
    /// (daemon) Leave the ramp in place on exit instead of restoring the screen.
    #[arg(long, requires = "daemon")]
    no_reset: bool,
    /// Set a fixed temperature in kelvin (client).
    #[arg(long, value_name = "KELVIN")]
    temp: Option<u32>,
    /// Toggle the filter on or off (client).
    #[arg(long)]
    toggle: bool,
    /// Turn the filter on (client).
    #[arg(long)]
    on: bool,
    /// Turn the filter off (client).
    #[arg(long)]
    off: bool,
    /// Return to following the sun (client).
    #[arg(long)]
    auto: bool,
    /// Set the gamma exponent, 0.1 to 10, constant across the day (client).
    #[arg(long, value_name = "GAMMA")]
    gamma: Option<f64>,
    /// Set the brightness bounds, 0.1 to 1: one value for all day, or
    /// DAY:NIGHT to dim with the sun (client).
    #[arg(long, value_name = "DAY:NIGHT")]
    brightness: Option<String>,
    /// Ease target changes over a couple of seconds: on or off (client).
    #[arg(long, value_name = "ON|OFF")]
    fade: Option<String>,
    /// Set the transition band's solar elevations as DAY:NIGHT degrees,
    /// like 3:-6; lower NIGHT lands full night deeper into dusk (client).
    #[arg(long, value_name = "DAY:NIGHT", allow_hyphen_values = true)]
    band: Option<String>,
    /// Print the daemon's status (client).
    #[arg(long)]
    status: bool,
}

fn main() {
    let cli = Cli::parse();
    if cli.daemon {
        run_daemon(cli.no_reset);
    } else if let Some(request) = client_request(&cli) {
        run_client(request);
    } else {
        run_discover();
    }
}

/// Maps the parsed flags to a client request, or `None` when none was given.
/// clap's `action` group guarantees at most one is set.
fn client_request(cli: &Cli) -> Option<client::Request> {
    if let Some(kelvin) = cli.temp {
        Some(client::Request::SetTemperature(kelvin))
    } else if cli.toggle {
        Some(client::Request::Toggle)
    } else if cli.on {
        Some(client::Request::SetEnabled(true))
    } else if cli.off {
        Some(client::Request::SetEnabled(false))
    } else if cli.auto {
        Some(client::Request::Auto)
    } else if let Some(gamma) = cli.gamma {
        Some(client::Request::SetGamma(gamma))
    } else if let Some(brightness) = cli.brightness.as_deref() {
        match parse_brightness(brightness) {
            Some(request) => Some(request),
            None => {
                eprintln!(
                    "nightlightd: --brightness wants a number or DAY:NIGHT, like 0.9 or 1.0:0.85"
                );
                std::process::exit(2);
            }
        }
    } else if let Some(fade) = cli.fade.as_deref() {
        match parse_fade(fade) {
            Some(fade) => Some(client::Request::SetFade(fade)),
            None => {
                eprintln!("nightlightd: --fade wants on or off");
                std::process::exit(2);
            }
        }
    } else if let Some(band) = cli.band.as_deref() {
        match parse_band(band) {
            Some((day, night)) => Some(client::Request::SetBand(day, night)),
            None => {
                eprintln!("nightlightd: --band wants DAY:NIGHT degrees, like 3:-6 or 1.5:-12");
                std::process::exit(2);
            }
        }
    } else if cli.status {
        Some(client::Request::Status)
    } else {
        None
    }
}

/// Parses the `--band` value: two elevations split by a colon. Returns
/// `None` on nonsense so the caller can print a usage hint. An inverted
/// pair is passed through — the daemon carries it verbatim and core
/// degrades it where it is spent.
fn parse_band(text: &str) -> Option<(f64, f64)> {
    let (day, night) = text.split_once(':')?;
    let day: f64 = day.trim().parse().ok()?;
    let night: f64 = night.trim().parse().ok()?;
    (day.is_finite() && night.is_finite()).then_some((day, night))
}

/// Parses the `--fade` value. Returns `None` on nonsense so the caller can
/// print a usage hint instead of sending garbage to the daemon.
fn parse_fade(text: &str) -> Option<bool> {
    match text.trim().to_ascii_lowercase().as_str() {
        "on" => Some(true),
        "off" => Some(false),
        _ => None,
    }
}

/// Parses the `--brightness` value: `0.9` means both bounds, `1.0:0.9`
/// means day and night. Returns `None` on nonsense so the caller can print
/// a usage hint instead of sending garbage to the daemon.
fn parse_brightness(text: &str) -> Option<client::Request> {
    let (day, night) = match text.split_once(':') {
        Some((day, night)) => (day.trim().parse().ok()?, night.trim().parse().ok()?),
        None => {
            let both: f64 = text.trim().parse().ok()?;
            (both, both)
        }
    };
    Some(client::Request::SetBrightness(day, night))
}

/// Sends a request to the daemon, with a clear error when it is not running.
fn run_client(request: client::Request) {
    if let Err(error) = client::send(request) {
        // Two different absences (#42): nobody on the bus, or a daemon that
        // is there but speaks another version of the interface.
        if client::daemon_on_bus() {
            eprintln!(
                "nightlightd: update needed, this binary and the daemon are different versions: {error}"
            );
        } else {
            eprintln!(
                "nightlightd: cannot reach the daemon (is it running? start it with --daemon): {error}"
            );
        }
        std::process::exit(1);
    }
}

/// Runs the daemon: serve D-Bus and follow the config until Ctrl+C. Restores
/// the screen on exit unless `no_reset` is set.
fn run_daemon(no_reset: bool) {
    init_logging();
    let loaded = config::load();
    let config = loaded.config;

    // What the daemon will run, which is not always what the file says: a
    // typo'd bound is held to the renderable table and a crossed pair drops
    // to the defaults. The file itself is left as its author wrote it; the
    // one trace is the warning below, so a screen that ignores a config line
    // is at least explicable from the log.
    let (day_temp, night_temp) = state::sane_temperatures(config.day_temp, config.night_temp);
    if (day_temp, night_temp) != (config.day_temp, config.night_temp) {
        tracing::warn!(
            "config temperatures (day {} K / night {} K) cannot be run as written; using day {} K / night {} K",
            config.day_temp,
            config.night_temp,
            day_temp,
            night_temp
        );
    }

    let waker = match waker::waker() {
        Ok(waker) => waker,
        Err(error) => fail("cannot create the wakeup channel", Box::new(error)),
    };
    let shared: state::Shared = Arc::new(Mutex::new(state::State {
        enabled: true,
        override_temp: None,
        mode: config.mode(),
        configured_mode: config.mode(),
        config_damaged: loaded.damaged,
        day_temp,
        night_temp,
        gamma: config.gamma,
        day_brightness: config.day_brightness,
        night_brightness: config.night_brightness,
        current_temp: x11::NEUTRAL_KELVIN,
        current_brightness: 1.0,
        current_gamma: 1.0,
        fade: config.fade,
        band: nightlightd_core::transition::Band {
            day_elevation: config.day_elevation,
            night_elevation: config.night_elevation,
        },
        location: None,
        outputs: Vec::new(),
    }));

    // Claim the D-Bus name — this is the single-instance lock (#19). Keep the
    // connection alive for the daemon's lifetime; dropping it stops serving.
    let _connection = match dbus::serve(Arc::clone(&shared), waker.clone()) {
        Ok(Some(connection)) => connection,
        Ok(None) => {
            tracing::info!("already running");
            return;
        }
        Err(error) => fail("cannot serve D-Bus", Box::new(error)),
    };

    let terminate = install_termination();

    // Shared with the suspend watcher: set on resume so the loop can tell a
    // resume (no RandR event) from an ordinary D-Bus wake and heal fast (#13).
    let resumed = Arc::new(AtomicBool::new(false));

    // Watch for resume from suspend on the system bus; wake the loop so the ramp
    // is re-applied at once instead of on the next tick (#16).
    let sleep_waker = waker.clone();
    let sleep_resumed = Arc::clone(&resumed);
    std::thread::spawn(move || {
        if let Err(error) = suspend::watch(sleep_waker, sleep_resumed) {
            tracing::warn!("suspend watcher unavailable: {error}");
        }
    });

    tracing::info!("daemon started (day {day_temp} K / night {night_temp} K)");
    if let Err(error) = x11::daemon_loop(&shared, &waker, &resumed, &terminate) {
        fail("daemon failed", error);
    }
    if !no_reset {
        restore();
    }
}

/// Writes the neutral ramp back to every screen on a clean exit.
fn restore() {
    match x11::apply_temperature(x11::NEUTRAL_KELVIN) {
        Ok(_) => tracing::info!("restored"),
        Err(error) => fail("cannot restore the screen", error),
    }
}

/// Sets up daemon logging: quiet by default (state changes only), verbose under
/// `RUST_LOG=debug` (every tick and every per-CRTC write).
fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// Creates the termination flag and wires SIGINT/SIGTERM to it, exiting on
/// failure to install the handlers.
fn install_termination() -> Arc<AtomicBool> {
    let terminate = Arc::new(AtomicBool::new(false));
    if let Err(error) = register_termination(&terminate) {
        fail("cannot install signal handlers", error);
    }
    terminate
}

/// Registers SIGINT (Ctrl+C) and SIGTERM to set `flag`, so the loop can notice
/// a termination request and exit cleanly.
fn register_termination(flag: &Arc<AtomicBool>) -> Result<(), Box<dyn Error>> {
    signal_hook::flag::register(SIGINT, Arc::clone(flag))?;
    signal_hook::flag::register(SIGTERM, Arc::clone(flag))?;
    Ok(())
}

/// Prints an error to stderr and exits non-zero.
fn fail(context: &str, error: Box<dyn Error>) -> ! {
    eprintln!("nightlightd: {context}: {error}");
    std::process::exit(1);
}

/// Prints the discovered CRTCs and their gamma-ramp sizes (a diagnostic).
fn run_discover() {
    match x11::discover() {
        Ok(crtcs) => {
            println!("found {} active CRTC(s)", crtcs.len());
            for c in &crtcs {
                println!("  CRTC {}: ramp size {}", c.crtc, c.gamma_size);
            }
        }
        Err(error) => fail("cannot query the X server", error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_flag_parses() {
        let cli = Cli::try_parse_from(["nightlightd", "--temp", "2800"]).unwrap();
        assert_eq!(cli.temp, Some(2800));
        assert!(!cli.daemon);
    }

    #[test]
    fn fade_flag_parses_and_excludes_other_actions() {
        let cli = Cli::try_parse_from(["nightlightd", "--fade", "off"]).unwrap();
        assert_eq!(cli.fade.as_deref(), Some("off"));
        assert!(Cli::try_parse_from(["nightlightd", "--fade", "on", "--temp", "2800"]).is_err());
    }

    #[test]
    fn band_values_parse_with_their_minus_signs() {
        assert_eq!(parse_band("3:-6"), Some((3.0, -6.0)));
        assert_eq!(parse_band(" 1.5 : -12 "), Some((1.5, -12.0)));
        // Inverted rides through; core judges it at spend time.
        assert_eq!(parse_band("-9:4"), Some((-9.0, 4.0)));
        assert_eq!(parse_band("3"), None);
        assert_eq!(parse_band("3:warm"), None);
        assert_eq!(parse_band("nan:-6"), None);
        let cli = Cli::try_parse_from(["nightlightd", "--band", "3:-9"]).unwrap();
        assert_eq!(cli.band.as_deref(), Some("3:-9"));
    }

    #[test]
    fn fade_values_parse_generously_but_not_infinitely() {
        assert_eq!(parse_fade("on"), Some(true));
        assert_eq!(parse_fade(" OFF "), Some(false));
        assert_eq!(parse_fade("On"), Some(true));
        assert_eq!(parse_fade("maybe"), None);
        assert_eq!(parse_fade(""), None);
    }

    #[test]
    fn daemon_conflicts_with_client_actions() {
        assert!(Cli::try_parse_from(["nightlightd", "--daemon", "--temp", "2800"]).is_err());
    }

    #[test]
    fn client_actions_are_mutually_exclusive() {
        assert!(Cli::try_parse_from(["nightlightd", "--toggle", "--status"]).is_err());
    }

    #[test]
    fn no_reset_requires_daemon() {
        assert!(Cli::try_parse_from(["nightlightd", "--no-reset"]).is_err());
        assert!(Cli::try_parse_from(["nightlightd", "--daemon", "--no-reset"]).is_ok());
    }

    #[test]
    fn brightness_accepts_one_value_or_a_pair() {
        assert!(matches!(
            parse_brightness("0.9"),
            Some(client::Request::SetBrightness(d, n)) if d == 0.9 && n == 0.9
        ));
        assert!(matches!(
            parse_brightness("1.0:0.85"),
            Some(client::Request::SetBrightness(d, n)) if d == 1.0 && n == 0.85
        ));
        assert!(parse_brightness("warm").is_none());
        assert!(parse_brightness("1.0:").is_none());
    }

    #[test]
    fn shaping_flags_join_the_exclusive_action_group() {
        assert!(Cli::try_parse_from(["nightlightd", "--gamma", "0.9"]).is_ok());
        assert!(Cli::try_parse_from(["nightlightd", "--gamma", "0.9", "--status"]).is_err());
    }
}
