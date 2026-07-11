//! `nightlightd` — a screen colour temperature daemon for X11.
//!
//! One binary, two modes: `--daemon` runs the daemon, a bare invocation such
//! as `--temp 2800` acts as a client and messages the daemon over D-Bus.
//!
//! Neither the daemon nor D-Bus exists yet. For now the binary applies a colour
//! temperature directly: `--temp 2800` warms every screen and holds it until
//! Ctrl+C, which restores the screen (#12); `--no-reset` applies it and exits,
//! leaving the ramp in place. With no arguments it reports the CRTCs it found
//! (#10). Full argument handling arrives with the CLI in #20.

mod x11;

use std::error::Error;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use signal_hook::consts::{SIGINT, SIGTERM};

/// The neutral temperature: its ramp is the identity, which restores a screen.
const NEUTRAL_KELVIN: u32 = 6500;

/// What the user asked the binary to do.
#[derive(Debug, PartialEq)]
enum Command {
    /// Print the discovered CRTCs (no arguments).
    Discover,
    /// Apply a colour temperature in kelvin to every screen.
    SetTemp {
        kelvin: u32,
        /// Restore the screen on a clean exit (the default); `--no-reset`
        /// turns this off so the ramp persists.
        reset_on_exit: bool,
    },
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args) {
        Ok(Command::Discover) => run_discover(),
        Ok(Command::SetTemp {
            kelvin,
            reset_on_exit,
        }) => run_set_temp(kelvin, reset_on_exit),
        Err(message) => {
            eprintln!("nightlightd: {message}");
            eprintln!("usage: nightlightd [--temp <kelvin>] [--no-reset]");
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
        None => Err("nothing to do (try --temp <kelvin>)".to_owned()),
    }
}

/// Applies `kelvin` to every screen. With `--no-reset` it applies once and
/// exits; otherwise it holds the ramp — re-applying whenever the screen
/// configuration changes — until Ctrl+C, then restores the screen.
fn run_set_temp(kelvin: u32, reset_on_exit: bool) {
    if !reset_on_exit {
        match x11::apply_temperature(kelvin) {
            Ok(count) => println!("applied {kelvin} K to {count} CRTC(s)"),
            Err(error) => {
                eprintln!("nightlightd: cannot set temperature: {error}");
                std::process::exit(1);
            }
        }
        return;
    }

    let terminate = Arc::new(AtomicBool::new(false));
    if let Err(error) = register_termination(&terminate) {
        eprintln!("nightlightd: cannot install signal handlers: {error}");
        std::process::exit(1);
    }

    println!("holding {kelvin} K — press Ctrl+C to restore (re-applies on screen changes)");
    if let Err(error) = x11::hold_and_watch(kelvin, &terminate) {
        eprintln!("nightlightd: {error}");
        std::process::exit(1);
    }

    match x11::apply_temperature(NEUTRAL_KELVIN) {
        Ok(_) => println!("restored"),
        Err(error) => {
            eprintln!("nightlightd: cannot restore the screen: {error}");
            std::process::exit(1);
        }
    }
}

/// Registers SIGINT (Ctrl+C) and SIGTERM to set `flag`, so the watch loop can
/// notice a termination request and exit cleanly.
fn register_termination(flag: &Arc<AtomicBool>) -> Result<(), Box<dyn Error>> {
    signal_hook::flag::register(SIGINT, Arc::clone(flag))?;
    signal_hook::flag::register(SIGTERM, Arc::clone(flag))?;
    Ok(())
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
        Err(error) => {
            eprintln!("nightlightd: cannot query the X server: {error}");
            std::process::exit(1);
        }
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
