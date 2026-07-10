//! `nightlightd` — a screen colour temperature daemon for X11.
//!
//! One binary, two modes: `--daemon` runs the daemon, a bare invocation such
//! as `--temp 2800` acts as a client and messages the daemon over D-Bus.
//!
//! Neither the daemon nor D-Bus exists yet. For now the binary applies a
//! colour temperature directly (issue #11): `--temp 2800` warms every screen,
//! `--temp 6500` restores it. With no arguments it just reports the CRTCs it
//! found (issue #10). Full argument handling arrives with the CLI in #20.

mod x11;

/// What the user asked the binary to do.
#[derive(Debug, PartialEq)]
enum Command {
    /// Print the discovered CRTCs (no arguments).
    Discover,
    /// Apply a colour temperature in kelvin to every screen.
    SetTemp(u32),
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_args(&args) {
        Ok(Command::Discover) => run_discover(),
        Ok(Command::SetTemp(kelvin)) => run_set_temp(kelvin),
        Err(message) => {
            eprintln!("nightlightd: {message}");
            eprintln!("usage: nightlightd [--temp <kelvin>]");
            std::process::exit(2);
        }
    }
}

/// Parses the command-line arguments into a [`Command`]. Deliberately tiny —
/// `clap` and the full CLI land in #20.
fn parse_args(args: &[String]) -> Result<Command, String> {
    match args {
        [] => Ok(Command::Discover),
        [flag, value] if flag == "--temp" => value
            .parse::<u32>()
            .map(Command::SetTemp)
            .map_err(|_| format!("invalid temperature: {value}")),
        _ => Err("unrecognised arguments".to_owned()),
    }
}

/// Applies `kelvin` to every active CRTC.
fn run_set_temp(kelvin: u32) {
    match x11::apply_temperature(kelvin) {
        Ok(count) => println!("applied {kelvin} K to {count} CRTC(s)"),
        Err(error) => {
            eprintln!("nightlightd: cannot set temperature: {error}");
            std::process::exit(1);
        }
    }
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
    fn temp_flag_parses_the_kelvin_value() {
        assert_eq!(
            parse_args(&args(&["--temp", "2800"])),
            Ok(Command::SetTemp(2800))
        );
    }

    #[test]
    fn non_numeric_temperature_is_rejected() {
        assert!(parse_args(&args(&["--temp", "warm"])).is_err());
    }

    #[test]
    fn unknown_arguments_are_rejected() {
        assert!(parse_args(&args(&["--temp"])).is_err());
        assert!(parse_args(&args(&["--bogus"])).is_err());
        assert!(parse_args(&args(&["--temp", "2800", "extra"])).is_err());
    }
}
