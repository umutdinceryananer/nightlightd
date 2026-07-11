//! `nightlightd` — a screen colour temperature daemon for X11.
//!
//! One binary, two modes: `--daemon` runs the daemon (follow the sun); any
//! other invocation acts as a client and messages the running daemon over
//! D-Bus. With no arguments it reports the CRTCs it found (a diagnostic).

mod client;
mod config;
mod dbus;
mod state;
mod waker;
mod x11;

use std::error::Error;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use clap::{ArgGroup, Parser};
use signal_hook::consts::{SIGINT, SIGTERM};

/// Screen colour temperature daemon for X11.
#[derive(Parser)]
#[command(name = "nightlightd", version, about)]
#[command(group(ArgGroup::new("action").args(["temp", "toggle", "on", "off", "auto", "status"])))]
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
    } else if cli.status {
        Some(client::Request::Status)
    } else {
        None
    }
}

/// Sends a request to the daemon, with a clear error when it is not running.
fn run_client(request: client::Request) {
    if let Err(error) = client::send(request) {
        eprintln!(
            "nightlightd: cannot reach the daemon (is it running? start it with --daemon): {error}"
        );
        std::process::exit(1);
    }
}

/// Runs the daemon: serve D-Bus and follow the config until Ctrl+C. Restores
/// the screen on exit unless `no_reset` is set.
fn run_daemon(no_reset: bool) {
    let config = config::load();

    let waker = match waker::waker() {
        Ok(waker) => waker,
        Err(error) => fail("cannot create the wakeup channel", Box::new(error)),
    };
    let shared: state::Shared = Arc::new(Mutex::new(state::State {
        enabled: true,
        override_temp: None,
        mode: config.mode(),
        day_temp: config.day_temp,
        night_temp: config.night_temp,
        current_temp: x11::NEUTRAL_KELVIN,
    }));

    // Claim the D-Bus name — this is the single-instance lock (#19). Keep the
    // connection alive for the daemon's lifetime; dropping it stops serving.
    let _connection = match dbus::serve(Arc::clone(&shared), waker.clone()) {
        Ok(Some(connection)) => connection,
        Ok(None) => {
            println!("nightlightd: already running");
            return;
        }
        Err(error) => fail("cannot serve D-Bus", Box::new(error)),
    };

    let terminate = install_termination();
    println!(
        "nightlightd: daemon started (day {} K / night {} K)",
        config.day_temp, config.night_temp
    );
    if let Err(error) = x11::daemon_loop(&shared, &waker, &terminate) {
        fail("daemon failed", error);
    }
    if !no_reset {
        restore();
    }
}

/// Writes the neutral ramp back to every screen on a clean exit.
fn restore() {
    match x11::apply_temperature(x11::NEUTRAL_KELVIN) {
        Ok(_) => println!("restored"),
        Err(error) => fail("cannot restore the screen", error),
    }
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
}
