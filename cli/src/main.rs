//! `nightlightd` — a screen colour temperature daemon for X11.
//!
//! Modes:
//! * `--daemon` — follow the sun continuously, updating each minute and
//!   surviving screen changes (#15).
//! * `--temp <kelvin>` — apply a fixed temperature and hold it until Ctrl+C,
//!   which restores the screen; `--no-reset` applies it and exits (#11, #12).
//! * no arguments — report the CRTCs found (#10).
//!
//! A separate thin client that talks to the daemon over D-Bus arrives in M4.

mod x11;

use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use signal_hook::consts::{SIGINT, SIGTERM};

/// The neutral temperature: its ramp is the identity, which restores a screen.
const NEUTRAL_KELVIN: u32 = 6500;
/// Default daytime temperature (neutral white) until the config file (#17).
const DEFAULT_DAY_KELVIN: u32 = 6500;
/// Default night temperature. 4500 K matches redshift/gammastep — a gentle
/// warmth for a first run; a stronger value belongs in the user's config (#17).
const DEFAULT_NIGHT_KELVIN: u32 = 4500;

/// What the user asked the binary to do.
#[derive(Debug, PartialEq)]
enum Command {
    /// Print the discovered CRTCs (no arguments).
    Discover,
    /// Apply a fixed colour temperature in kelvin to every screen.
    SetTemp {
        kelvin: u32,
        /// Restore the screen on a clean exit (the default); `--no-reset`
        /// turns this off so the ramp persists.
        reset_on_exit: bool,
    },
    /// Run the daemon: follow the sun continuously.
    Daemon,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args) {
        Ok(Command::Discover) => run_discover(),
        Ok(Command::SetTemp {
            kelvin,
            reset_on_exit,
        }) => run_set_temp(kelvin, reset_on_exit),
        Ok(Command::Daemon) => run_daemon(),
        Err(message) => {
            eprintln!("nightlightd: {message}");
            eprintln!("usage: nightlightd [--daemon | --temp <kelvin> [--no-reset]]");
            std::process::exit(2);
        }
    }
}

/// Parses the command-line arguments into a [`Command`]. Deliberately tiny —
/// `clap` and the full CLI land in #20.
fn parse_args(args: &[String]) -> Result<Command, String> {
    if args.is_empty() {
        return Ok(Command::Discover);
    }

    // `--daemon` is exclusive: it takes no other arguments.
    if args.iter().any(|arg| arg == "--daemon") {
        return if args.len() == 1 {
            Ok(Command::Daemon)
        } else {
            Err("--daemon takes no other arguments".to_owned())
        };
    }

    let mut kelvin: Option<u32> = None;
    let mut reset_on_exit = true;

    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--temp" => {
                let value = rest.next().ok_or("--temp needs a value")?;
                let parsed = value
                    .parse::<u32>()
                    .map_err(|_| format!("invalid temperature: {value}"))?;
                kelvin = Some(parsed);
            }
            "--no-reset" => reset_on_exit = false,
            other => return Err(format!("unrecognised argument: {other}")),
        }
    }

    match kelvin {
        Some(kelvin) => Ok(Command::SetTemp {
            kelvin,
            reset_on_exit,
        }),
        None => Err("nothing to do (try --temp <kelvin> or --daemon)".to_owned()),
    }
}

/// Applies a fixed `kelvin`. With `--no-reset` it applies once and exits;
/// otherwise it holds the ramp until Ctrl+C, then restores the screen.
fn run_set_temp(kelvin: u32, reset_on_exit: bool) {
    if !reset_on_exit {
        match x11::apply_temperature(kelvin) {
            Ok(count) => println!("applied {kelvin} K to {count} CRTC(s)"),
            Err(error) => fail("cannot set temperature", error),
        }
        return;
    }

    let terminate = install_termination();
    println!("holding {kelvin} K — press Ctrl+C to restore (re-applies on screen changes)");
    if let Err(error) = x11::hold_and_watch(kelvin, &terminate) {
        fail("watch loop failed", error);
    }
    restore();
}

/// Runs the daemon: follow the sun until Ctrl+C, then restore the screen.
fn run_daemon() {
    let terminate = install_termination();
    println!(
        "nightlightd: daemon started (day {DEFAULT_DAY_KELVIN} K / night {DEFAULT_NIGHT_KELVIN} K)"
    );
    if let Err(error) = x11::daemon_loop(DEFAULT_DAY_KELVIN, DEFAULT_NIGHT_KELVIN, &terminate) {
        fail("daemon failed", error);
    }
    restore();
}

/// Writes the neutral ramp back to every screen on a clean exit.
fn restore() {
    match x11::apply_temperature(NEUTRAL_KELVIN) {
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

/// Registers SIGINT (Ctrl+C) and SIGTERM to set `flag`, so the loops can notice
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

/// Prints the discovered CRTCs and their gamma-ramp sizes.
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

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn no_arguments_means_discover() {
        assert_eq!(parse_args(&args(&[])), Ok(Command::Discover));
    }

    #[test]
    fn daemon_flag_is_recognised() {
        assert_eq!(parse_args(&args(&["--daemon"])), Ok(Command::Daemon));
    }

    #[test]
    fn daemon_is_exclusive() {
        assert!(parse_args(&args(&["--daemon", "--temp", "2800"])).is_err());
    }

    #[test]
    fn temp_flag_resets_by_default() {
        assert_eq!(
            parse_args(&args(&["--temp", "2800"])),
            Ok(Command::SetTemp {
                kelvin: 2800,
                reset_on_exit: true,
            })
        );
    }

    #[test]
    fn no_reset_flag_is_honoured_in_any_order() {
        let expected = Command::SetTemp {
            kelvin: 2800,
            reset_on_exit: false,
        };
        assert_eq!(
            parse_args(&args(&["--temp", "2800", "--no-reset"])),
            Ok(expected)
        );

        let expected = Command::SetTemp {
            kelvin: 2800,
            reset_on_exit: false,
        };
        assert_eq!(
            parse_args(&args(&["--no-reset", "--temp", "2800"])),
            Ok(expected)
        );
    }

    #[test]
    fn malformed_arguments_are_rejected() {
        assert!(parse_args(&args(&["--temp", "warm"])).is_err()); // not a number
        assert!(parse_args(&args(&["--temp"])).is_err()); // missing value
        assert!(parse_args(&args(&["--bogus"])).is_err()); // unknown flag
        assert!(parse_args(&args(&["--no-reset"])).is_err()); // nothing to apply
    }
}
