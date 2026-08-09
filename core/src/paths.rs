//! Where this program's files live.
//!
//! One derivation, in the crate everything else depends on. It was written
//! out five times before this — the daemon's config, the gammastep import,
//! the dashboard's settings line, the panel's, and nearly a sixth for the
//! remembered theme — and five copies of "XDG_CONFIG_HOME, else ~/.config"
//! is five chances to disagree about where a user's settings are.

use std::ffi::OsStr;
use std::path::PathBuf;

/// The directory user configuration lives under: `$XDG_CONFIG_HOME`, or
/// `~/.config` when that is unset.
///
/// [`None`] when neither variable is set, which is a session broken well past
/// this program's business — every caller degrades to doing without rather
/// than guessing at `/`.
pub fn config_home() -> Option<PathBuf> {
    home_from(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )
}

/// A named file in this program's own directory —
/// `<config home>/nightlightd/<name>`.
pub fn config_file(name: &str) -> Option<PathBuf> {
    Some(config_home()?.join("nightlightd").join(name))
}

/// The choice, separated from the environment so it can be tested against
/// strings rather than against whichever account happens to run the tests.
/// An empty variable counts as unset: an exported-but-blank `XDG_CONFIG_HOME`
/// would otherwise resolve every path to a bare relative name.
fn home_from(xdg: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    let set = |value: Option<&OsStr>| value.filter(|value| !value.is_empty()).map(PathBuf::from);
    set(xdg).or_else(|| set(home).map(|home| home.join(".config")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_wins_and_home_is_the_fallback() {
        let xdg = OsStr::new("/somewhere/cfg");
        let home = OsStr::new("/home/someone");
        assert_eq!(
            home_from(Some(xdg), Some(home)),
            Some(PathBuf::from("/somewhere/cfg"))
        );
        assert_eq!(
            home_from(None, Some(home)),
            Some(PathBuf::from("/home/someone/.config"))
        );
        assert_eq!(home_from(None, None), None);
        // Exported but blank is not a setting. Left to `or_else` alone it
        // would win over HOME and land every file in the current directory.
        assert_eq!(
            home_from(Some(OsStr::new("")), Some(home)),
            Some(PathBuf::from("/home/someone/.config"))
        );
        assert_eq!(home_from(Some(OsStr::new("")), Some(OsStr::new(""))), None);
    }
}
