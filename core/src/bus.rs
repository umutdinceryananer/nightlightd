//! The well-known bus names, in one place (#55).
//!
//! Three programs here own a name: the daemon, which owns its name as the
//! single-instance lock (#19), and the tray and the panel, which own theirs
//! for the same reason (GitHub #1). Natively the names live under
//! `org.nightlightd`, as they always have. Inside a Flatpak a sandbox may own
//! only names below its own app ID, and owning anything else takes a
//! permission Flathub's reviewers are asked to refuse, so there the names
//! move under the app ID. Interface names and object paths do not move:
//! they are not owned, and no sandbox rule touches them.
//!
//! Every program asks here, so a client and the daemon can never disagree
//! about where to find each other within one install. Across installs they
//! do disagree on purpose: a native client has no business talking to a
//! Flatpak daemon, and #56 keeps the two daemons from running together.

use crate::sandbox;

/// A program that owns a well-known name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Daemon,
    Tray,
    Panel,
}

impl Service {
    fn suffix(self) -> &'static str {
        match self {
            Service::Daemon => "Daemon",
            Service::Tray => "Tray",
            Service::Panel => "Panel",
        }
    }
}

/// What a native install's names start with.
const NATIVE: &str = "org.nightlightd";

/// `service`'s name for this process: below the sandbox's app ID inside a
/// Flatpak, below `org.nightlightd` outside one.
pub fn name(service: Service) -> String {
    name_for(sandbox::app_id().as_deref(), service)
}

/// The same, told the app ID rather than reading it: `None` is a native
/// install. Pure, so it can be tested, and so #56 can name the other
/// install's daemon.
pub fn name_for(app_id: Option<&str>, service: Service) -> String {
    format!("{}.{}", app_id.unwrap_or(NATIVE), service.suffix())
}

/// `service`'s name in the other kind of install: the Flatpak's, seen from
/// a native process, and the native one, seen from inside a Flatpak (#56).
/// After #55 a native daemon and a Flatpak one no longer meet on one name,
/// and two daemons write one gamma ramp: the flicker this project exists
/// to remove. Each looks for the other before it starts.
pub fn other_install_name(service: Service) -> String {
    other_name_for(sandbox::is_flatpak(), service)
}

fn other_name_for(in_flatpak: bool, service: Service) -> String {
    if in_flatpak {
        name_for(None, service)
    } else {
        name_for(Some(sandbox::APP_ID), service)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::APP_ID;

    /// The names every native install has used since 0.1. Changing one is a
    /// break between a daemon and its own clients, so they are pinned.
    #[test]
    fn a_native_install_keeps_its_names() {
        assert_eq!(name_for(None, Service::Daemon), "org.nightlightd.Daemon");
        assert_eq!(name_for(None, Service::Tray), "org.nightlightd.Tray");
        assert_eq!(name_for(None, Service::Panel), "org.nightlightd.Panel");
    }

    /// Each kind of install looks for the other one's daemon, never for its
    /// own: its own name is the single-instance lock, and it already asks
    /// for that.
    #[test]
    fn each_install_looks_for_the_other_ones_daemon() {
        assert_eq!(
            other_name_for(false, Service::Daemon),
            name_for(Some(APP_ID), Service::Daemon)
        );
        assert_eq!(
            other_name_for(true, Service::Daemon),
            "org.nightlightd.Daemon"
        );
    }

    /// Inside the Flatpak every name sits below the app ID, which is what
    /// lets the sandbox own it with no permission at all.
    #[test]
    fn a_flatpak_names_everything_below_its_app_id() {
        for service in [Service::Daemon, Service::Tray, Service::Panel] {
            let name = name_for(Some(APP_ID), service);
            assert!(name.starts_with(&format!("{APP_ID}.")), "{name}");
        }
        assert_eq!(
            name_for(Some(APP_ID), Service::Daemon),
            "io.github.umutdinceryananer.nightlightd.Daemon"
        );
    }
}
