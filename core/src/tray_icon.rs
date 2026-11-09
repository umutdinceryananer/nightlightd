//! Which icon the tray wears (GitHub #4): a list three interfaces share.
//!
//! The tray paints them; the panel and the dashboard are where one gets
//! chosen, in their settings. All that passes between them is one small file
//! naming the choice, which the tray reads again on its usual refresh. A
//! D-Bus method would reach the tray sooner, at the price of one more
//! interface for something that changes about once a lifetime, and the daemon
//! has no business knowing what a client looks like.

use crate::paths;

/// The file the choice is kept in, beside the config.
pub const FILE: &str = "tray-icon";

/// One of the tray's icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrayIcon {
    /// The icon theme's own monochrome symbol, and the default: nobody's
    /// panel changes unless they ask it to.
    #[default]
    Plain,
    /// A navy disc holding a crescent in the screen's colour.
    NightBadge,
    /// A sun that sinks and a sky that darkens as the screen warms.
    Horizon,
    /// A monitor whose display takes the screen's colour.
    Screen,
}

impl TrayIcon {
    /// Every icon, in the order the settings list them.
    pub const ALL: [TrayIcon; 4] = [
        TrayIcon::Plain,
        TrayIcon::NightBadge,
        TrayIcon::Horizon,
        TrayIcon::Screen,
    ];

    /// What the settings call it.
    pub fn label(self) -> &'static str {
        match self {
            TrayIcon::Plain => "plain",
            TrayIcon::NightBadge => "night badge",
            TrayIcon::Horizon => "horizon",
            TrayIcon::Screen => "screen",
        }
    }

    /// What the choice is written down as. Separate from the label so that
    /// rewording the settings cannot forget anybody's choice.
    pub fn name(self) -> &'static str {
        match self {
            TrayIcon::Plain => "plain",
            TrayIcon::NightBadge => "night-badge",
            TrayIcon::Horizon => "horizon",
            TrayIcon::Screen => "screen",
        }
    }

    pub fn from_name(name: &str) -> Option<TrayIcon> {
        TrayIcon::ALL.into_iter().find(|icon| icon.name() == name)
    }

    /// The neighbour in the list, going round at either end.
    pub fn stepped(self, forward: bool) -> TrayIcon {
        let count = TrayIcon::ALL.len();
        let here = TrayIcon::ALL
            .iter()
            .position(|icon| *icon == self)
            .unwrap_or(0);
        TrayIcon::ALL[if forward {
            (here + 1) % count
        } else {
            (here + count - 1) % count
        }]
    }
}

/// What a file's contents choose. Anything empty or unknown is the default
/// without complaint: the file is a convenience, and somebody may have typed
/// into it.
fn chosen_by(contents: &str) -> TrayIcon {
    TrayIcon::from_name(contents.trim()).unwrap_or_default()
}

/// The icon last chosen, or the plain one when nothing has been.
pub fn remembered() -> TrayIcon {
    paths::config_file(FILE)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|contents| chosen_by(&contents))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_choice_can_be_read_back_from_its_name() {
        for icon in TrayIcon::ALL {
            assert_eq!(TrayIcon::from_name(icon.name()), Some(icon));
        }
    }

    #[test]
    fn a_file_chooses_by_what_it_says_and_nothing_else() {
        assert_eq!(chosen_by("horizon"), TrayIcon::Horizon);
        // A newline from an editor is not a different choice.
        assert_eq!(chosen_by("night-badge\n"), TrayIcon::NightBadge);
        assert_eq!(chosen_by(""), TrayIcon::Plain);
        assert_eq!(chosen_by("sparkles"), TrayIcon::Plain);
    }

    #[test]
    fn stepping_goes_round_at_both_ends() {
        assert_eq!(TrayIcon::Plain.stepped(true), TrayIcon::NightBadge);
        assert_eq!(TrayIcon::Screen.stepped(true), TrayIcon::Plain);
        assert_eq!(TrayIcon::Plain.stepped(false), TrayIcon::Screen);
        for icon in TrayIcon::ALL {
            assert_eq!(icon.stepped(true).stepped(false), icon);
        }
    }
}
