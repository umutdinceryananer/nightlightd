//! Whether this process runs inside a Flatpak sandbox, and as which app
//! (#54).
//!
//! Several behaviours switch on it once the Flatpak lands (#55 to #59):
//! which bus names to own, how a tray icon registers, how a daemon gets
//! started and how start-at-login is set. One definition here, std only.
//! The parsing is a pure function over the text of `/.flatpak-info`, and the
//! reading is a thin shell around it, the same shape as `core::location`, so
//! everything worth testing is testable without a sandbox.

use std::path::Path;

/// The app ID nightlightd is published under on Flathub. A constant rather
/// than read at run time because a native install needs it too: the daemon
/// there has to recognise a Flatpak daemon's bus name (#56).
pub const APP_ID: &str = "io.github.umutdinceryananer.nightlightd";

/// The file Flatpak puts at the root of every sandbox. Its presence is the
/// test the Flatpak documentation itself gives.
const INFO_FILE: &str = "/.flatpak-info";

/// Whether this process is inside a Flatpak sandbox.
pub fn is_flatpak() -> bool {
    Path::new(INFO_FILE).exists()
}

/// The ID of the app this sandbox belongs to, or `None` outside one. Read
/// from `/.flatpak-info` first, because the sandbox writes it and the app
/// cannot change it; `FLATPAK_ID` is the fallback for a file that exists
/// but cannot be read. Outside a sandbox the variable is not trusted at
/// all: anybody can set it in a shell.
pub fn app_id() -> Option<String> {
    if !is_flatpak() {
        return None;
    }
    std::fs::read_to_string(INFO_FILE)
        .ok()
        .and_then(|info| application_name(&info).map(str::to_owned))
        .or_else(|| std::env::var("FLATPAK_ID").ok().filter(|id| !id.is_empty()))
}

/// The `name` key of the `[Application]` group in the text of a
/// `/.flatpak-info`, which is a GLib key file: groups in brackets, `key=value`
/// lines with whitespace allowed around the `=`, and `#` comments. A `name`
/// in any other group is not the app's, and an empty one is no name.
fn application_name(info: &str) -> Option<&str> {
    let mut in_application = false;
    for line in info.lines().map(str::trim) {
        if line.starts_with('[') {
            in_application = line == "[Application]";
            continue;
        }
        if !in_application || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=')
            && key.trim() == "name"
        {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape Flatpak writes, trimmed to the groups that matter.
    const INFO: &str = "\
[Application]
name=io.github.umutdinceryananer.nightlightd
runtime=runtime/org.freedesktop.Platform/x86_64/25.08

[Instance]
instance-id=1234567890
app-path=/var/lib/flatpak/app/io.github.umutdinceryananer.nightlightd/x86_64/master/active/files
";

    #[test]
    fn the_app_is_named_in_its_own_group() {
        assert_eq!(application_name(INFO), Some(APP_ID));
    }

    /// A key file allows spaces around the `=` and comments between keys.
    #[test]
    fn spacing_and_comments_do_not_hide_the_name() {
        let info = "[Application]\n# written by flatpak\nname = org.example.App \n";
        assert_eq!(application_name(info), Some("org.example.App"));
    }

    /// Only the `[Application]` group names the app; a `name` anywhere else,
    /// or an empty one, is no answer.
    #[test]
    fn a_name_elsewhere_or_an_empty_one_is_not_the_app() {
        assert_eq!(application_name("[Instance]\nname=not.this\n"), None);
        assert_eq!(application_name("[Application]\nname=\n"), None);
        assert_eq!(application_name(""), None);
        // The group ends where the next begins.
        let info = "[Application]\nruntime=x\n[Context]\nname=not.this\n";
        assert_eq!(application_name(info), None);
    }

    /// The constant is a valid app ID the way Flatpak counts one: reverse
    /// DNS, at least three parts, and a bus name below it stays a bus name.
    #[test]
    fn the_app_id_is_shaped_like_one() {
        let parts: Vec<&str> = APP_ID.split('.').collect();
        assert!(parts.len() >= 3);
        assert!(
            parts.iter().all(|part| !part.is_empty()
                && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        );
        assert!(format!("{APP_ID}.Daemon").len() <= 255);
    }
}
