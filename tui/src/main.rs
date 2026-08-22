//! The TUI client (#35): a one-screen ratatui dashboard.
//!
//! A thin client like the tray and panel — no state beyond the last snapshot;
//! the daemon owns everything. One glanceable screen, designed per
//! docs/TUI-DESIGN.md: everything is derived from a single accent colour, and
//! in the default `live` theme that accent is the actual tint the screen is
//! filtered to — the interface warms with the screen at night. `T` cycles the
//! fixed themes; `--theme` picks one at launch. Deliberately no tabs, views,
//! or config editing beyond the night bound: a remote control, not an
//! application.

mod autostart;
mod daemon;
mod theme;

use std::io;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nightlightd_core::color::{UI_TEMPERATURE_RANGE, temperature_to_rgb};
use nightlightd_core::location::nearest_zone;
use nightlightd_core::schedule::{Milestone, milestones};
use nightlightd_core::solar::solar_elevation;
use nightlightd_core::transition::{Band, phase, target_temperature};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Map, MapResolution};
use ratatui::widgets::{
    Axis, Block, Cell, Chart, Clear, Dataset, GraphType, Padding, Paragraph, RatatuiLogo, Row,
    Table, Wrap,
};
use ratatui::{DefaultTerminal, Frame};
use ratatui_braille_bar::BrailleBar;
use tui_big_text::{BigText, PixelSize};
use tui_slider::Slider;

use crate::daemon::{Client, Status};
use crate::theme::{Palette, THEMES};

/// Bounds and step for the temperature keys, mirroring the panel. Both ends
/// come from core so the two interfaces cannot drift apart about what is
/// settable; the day end runs past neutral into a bluish daytime (#41).
const NIGHT_MIN: u32 = UI_TEMPERATURE_RANGE.0;
const DAY_MAX: u32 = UI_TEMPERATURE_RANGE.1;
const NIGHT_STEP: u32 = 100;

/// What the chart has to say about the screen, or `None` when the schedule
/// drawn on it is the thing the screen is following (#52).
///
/// Two states take the schedule out of force and they are not the same: a
/// hold is a temperature someone chose, off is the screen left neutral. Off
/// outranks a hold, the precedence every readout here already uses — a
/// filter that is off is off whether or not a hold sits remembered under it.
fn out_of_force(enabled: bool, following: bool, temperature: u32) -> Option<String> {
    match (enabled, following) {
        (true, true) => None,
        (true, false) => Some(format!("held at {temperature} K")),
        (false, _) => Some(format!("off · {temperature} K")),
    }
}

/// The scale both temperature rails are drawn against: exactly what the
/// arrow keys can reach, and fixed.
///
/// It has to be fixed. A ceiling that followed the day bound put that bound
/// permanently at the end of its own rail — a full bar at 6500 K, a full bar
/// at 10000 K, no way to tell them apart — because the value being drawn was
/// also the scale drawing it. The panel's curve survives that rule (a
/// chart's information is the shape between its plateaus, so a plateau at
/// the top still reads); a single bar does not, since its fill *is* the
/// whole message. The two need different rules and briefly shared one.
///
/// The cost, paid knowingly: at the default 6500 K day the bar is a little
/// over half rather than hard against the end. What it buys is a bar that
/// means something at every value, and the same window the panel's own
/// slider offers, so the two clients describe one setting one way.
const RAIL_RANGE: (f64, f64) = (NIGHT_MIN as f64, DAY_MAX as f64);

/// One full day in the `--demo` compressed clock, in real seconds (#30).
/// Grown from 28 when the band editor and the day summary joined the tour:
/// the reel had no room left, and a tour that hurries reads as a list of
/// features rather than as a program being used. `scripts/demo-gif.sh`
/// records exactly this long, so the two move together.
const DEMO_DAY_SECONDS: f64 = 34.0;

/// The demo's scripted tour (#30): (second, key, chip label). Starts at noon
/// on the now tab; dwells through sunset while the interface warms, walks the
/// tabs, rolls `T` through every theme back to `live` (sunrise lands during
/// the roll, so the return to `live` opens on morning gold), then jumps home.
/// One pass is exactly one compressed day, so a recording loops seamlessly.
const DEMO_SCRIPT: &[(f64, KeyCode, &str)] = &[
    // Nothing at all until the sun is down. The day warming through sunset is
    // the one thing this program is for, and a tour walking over it is a tour
    // talking during the film.
    //
    // Then the band (#39, #45): open it, take the night bound down three
    // degrees a half at a time, and watch the ramp stretch on the same frame
    // as the press. It leaves by escape rather than enter, which shows the
    // unsaved-work guard and — the reason it matters here — applies nothing,
    // so recording the reel never writes to the recorder's own daemon.
    (12.5, KeyCode::Char('b'), "b"),
    (13.0, KeyCode::Down, "↓"),
    (13.4, KeyCode::Left, "←"),
    (13.8, KeyCode::Left, "←"),
    (14.2, KeyCode::Left, "←"),
    (14.6, KeyCode::Left, "←"),
    (15.0, KeyCode::Left, "←"),
    (15.4, KeyCode::Left, "←"),
    (16.4, KeyCode::Esc, "esc"),
    (17.4, KeyCode::Char('n'), "n"),
    // The day, said in numbers: how long it is, and how much of it we have
    // gained since yesterday.
    (18.2, KeyCode::Char('s'), "s"),
    (20.2, KeyCode::Esc, "esc"),
    // Then the tabs, and the palettes on the way out.
    (21.0, KeyCode::Tab, "⇥"),
    (22.4, KeyCode::Tab, "⇥"),
    (23.6, KeyCode::Tab, "⇥"),
    (24.6, KeyCode::Tab, "⇥"),
    (25.4, KeyCode::Char('T'), "T"),
    (26.1, KeyCode::Char('T'), "T"),
    (26.8, KeyCode::Char('T'), "T"),
    (27.5, KeyCode::Char('T'), "T"),
    (28.2, KeyCode::Char('T'), "T"),
    (28.9, KeyCode::Char('T'), "T"),
    (29.6, KeyCode::Char('T'), "T"),
    (30.3, KeyCode::Char('T'), "T"),
    (31.5, KeyCode::Char('1'), "1"),
];

/// The tab bar, in order. Each holds real content or it does not exist.
const TABS: &[&str] = &["now", "today", "location", "outputs", "settings"];

/// Where the chosen theme is remembered. Its own file, beside the daemon's
/// config but not inside it: which colours a terminal wears is the terminal's
/// business, and the panel keeps its choice under another name so a change
/// made here never reaches into a window that was not open.
const THEME_FILE: &str = "tui-theme";
const LOCATION_TAB: usize = 2;
/// The settings tab's index and its selectable rows: day, night, gamma,
/// night dim, fade, theme, login. The transition band is deliberately not
/// here (#45): it changes the shape of the curve, so it is edited over the
/// curve, with `b`, where the change can be watched.
const SETTINGS_TAB: usize = 4;
const SETTINGS_ITEMS: usize = 7;
/// The tabs that draw the schedule, and so accept `b`.
const NOW_TAB: usize = 0;
const TODAY_TAB: usize = 1;

/// The gamma slider's on-screen band. Core accepts 0.1 to 10, but a slider
/// spanning that would bury the useful calibration range in its first
/// centimetre; hand-written config values outside the band still apply.
const GAMMA_UI_MIN: f64 = 0.5;
const GAMMA_UI_MAX: f64 = 1.5;
/// One arrow press of gamma or brightness.
const FACTOR_STEP: f64 = 0.05;

/// One arrow press of a transition bound (#45), in degrees of elevation.
const BAND_STEP: f64 = 0.5;
/// The band rows' on-screen window. The floor is astronomical twilight,
/// below which there is no more night to find; the ceiling is daylight
/// nobody filters through. A hand-written config value outside this still
/// applies and still draws, the arrows just cannot produce one.
const BAND_UI_MIN: f64 = -18.0;
const BAND_UI_MAX: f64 = 6.0;
/// The narrowest the arrows may pinch the band. A bound may move, the pair
/// may never cross.
const MIN_BAND_WIDTH: f64 = 0.5;

/// A settings slider rail: (value, min, max) for the row underneath.
type Rail = (f64, f64, f64);

/// One line of the help popup: the key, and what it does.
type KeyRow = (&'static str, &'static str);

/// The help popup's sections, folded like an accordion so one is open at a
/// time. The footer keeps three keys and no more, which makes this the only
/// place the rest are written down — and twenty keys in one flat column is a
/// wall, not a reference.
const HELP: [(&str, &[KeyRow]); 4] = [
    (
        "everywhere",
        &[
            ("⇥ · 1-5", "switch tab"),
            ("t", "toggle the filter"),
            ("a", "back to automatic"),
            ("↑↓", "nudge the night temperature"),
            ("T", "cycle the theme"),
            ("s", "sun details"),
            ("r", "start or restart a silent daemon"),
            ("?", "this help"),
            ("q", "quit"),
        ],
    ),
    (
        "now · today",
        &[
            ("b", "the transition band"),
            ("↑↓", "pick a bound"),
            ("‹›", "move it half a degree"),
            ("d", "back to the default band"),
            ("⏎", "apply · esc reverts"),
        ],
    ),
    (
        "location",
        &[
            ("⏎", "pick a spot · pin it"),
            ("m", "the map, full size"),
            ("c", "back to the timezone"),
        ],
    ),
    (
        "settings",
        &[("↑↓", "select a row"), ("‹›", "adjust it"), ("⏎", "toggle")],
    ),
];

/// The band editor's state (#45). The arrows build a draft that the curve
/// draws immediately and the daemon never hears about; the screen changes
/// on apply, and only then. The same bargain the panel's drag makes, for
/// the same reason: a walk to the value you meant crosses a dozen you did
/// not, and each one would be a write to disk and a lurch on the screen.
struct BandEdit {
    /// The band the editor opened with, and what revert goes back to.
    original: Band,
    /// The band the arrows have built so far.
    draft: Band,
    /// Which bound the arrows move: 0 the day bound, 1 the night bound.
    selected: usize,
    /// Set by escaping with an unapplied draft: the panel stops offering
    /// bounds and asks the one question left.
    confirming: bool,
}

impl BandEdit {
    fn touched(&self) -> bool {
        self.draft != self.original
    }
}

/// One arrow press on a transition bound: the next half-degree in the
/// direction pressed, not half a degree from wherever the value happens to
/// sit. A bound dragged on the panel's curve lands on whatever elevation the
/// pointer was over, and stepping from -13.9578 would only ever produce more
/// numbers like it. The first press tidies, the rest walk the grid.
/// One arrow press on the band editor: move `day` or the night bound by a
/// step, then hold the two rules a keypress must never break. The pair may
/// not cross — a bound stops half a degree from its neighbour, which is
/// still a hard switch but at least a well-defined one — and neither bound
/// leaves the window the rails draw, so what the row shows is what the value
/// is. Pure, because these are the rules worth a test rather than an eye.
fn nudged_band(band: Band, day: bool, increase: bool) -> Band {
    let mut next = band;
    if day {
        next.day_elevation = nudge(next.day_elevation, increase)
            .clamp(next.night_elevation + MIN_BAND_WIDTH, BAND_UI_MAX);
    } else {
        next.night_elevation = nudge(next.night_elevation, increase)
            .clamp(BAND_UI_MIN, next.day_elevation - MIN_BAND_WIDTH);
    }
    next
}

fn nudge(value: f64, increase: bool) -> f64 {
    let steps = value / BAND_STEP;
    let next = if increase {
        (steps + 1e-9).floor() + 1.0
    } else {
        (steps - 1e-9).ceil() - 1.0
    };
    next * BAND_STEP
}

/// Picker steps in degrees — coarse on purpose; braille map cells are chunky.
const PICK_LAT_STEP: f64 = 3.0;
const PICK_LON_STEP: f64 = 5.0;

/// The map viewport. Antarctica is cropped away — nobody runs a night light
/// there — which hands its rows to the latitudes people actually live at.
/// The picker clamps to the same bounds so the pin can never leave the map.
const MAP_LAT_MIN: f64 = -55.0;
const MAP_LAT_MAX: f64 = 75.0;

struct App {
    client: Client,
    status: Option<Status>,
    /// The fade switch (#44), read through the additive `GetFade`; `None`
    /// against a daemon that is unreachable or too old to answer.
    fade: Option<bool>,
    /// Status unreadable but the daemon's name owned (#42): different
    /// versions, which deserves a different banner than "not running".
    mismatch: bool,
    /// The transition band (#39) as the daemon last reported it, so every
    /// curve and schedule drawn here matches what the screen actually does;
    /// the default against a daemon that cannot answer.
    band: Band,
    /// Whether that band came from the daemon or is the drawing fallback.
    /// The settings rows (#45) show blanks when it is the fallback: a v0.2.1
    /// daemon answers `GetStatus` but knows nothing of a band, and offering
    /// arrows that change nothing is the lie the fade row already refuses.
    band_known: bool,
    last_poll: Option<Instant>,
    offset_secs: i32,
    theme_index: usize,
    tab: usize,
    settings_selected: usize,
    /// The theme picker popup: `Some(highlighted index)` while open.
    theme_popup: Option<usize>,
    /// The `b` band editor (#45) while it is open.
    band_edit: Option<BandEdit>,
    /// The `?` overlay: every key in one place.
    help_popup: bool,
    /// Which of the help popup's sections is unfolded.
    help_section: usize,
    /// The `s` overlay: the solar facts behind the dashboard's summaries.
    sun_popup: bool,
    /// The `m` overlay: the world map at full size.
    map_popup: bool,
    start_at_login: bool,
    /// The map's location picker: `Some((lat, lon))` cursor while picking.
    picker: Option<(f64, f64)>,
    /// The close-up camera while picking on a small map: the viewport centre,
    /// dragged along when the cursor nears an edge. Cleared with the picker.
    map_cam: Option<(f64, f64)>,
    /// The nearest zone city under the picker cursor, refreshed as it moves.
    picker_place: Option<String>,
    /// The nearest zone city for the pinned location, cached by coordinate so
    /// zone.tab is only re-read when the location actually changes.
    place: Option<(f64, f64, String)>,
    /// The active outputs, polled together with the status.
    outputs: Option<Vec<(u32, u16)>>,
    /// `--demo`: when the compressed day started; `None` in normal use.
    demo: Option<Instant>,
    /// How many scripted demo keys have fired, across loops.
    demo_cursor: usize,
    /// The last scripted key and when it fired, for the on-screen chip.
    demo_key: Option<(&'static str, Instant)>,
}

/// The stand-in snapshot for `--demo` without a daemon: Istanbul, the
/// defaults, following the sun. Honest about itself in the source field.
fn demo_status() -> Status {
    Status {
        enabled: true,
        temperature: 6500,
        source: "demo".into(),
        elevation: 0.0,
        has_location: true,
        latitude: 41.01,
        longitude: 28.98,
        following: true,
        day_temp: 6500,
        night_temp: 2600,
        gamma: 1.0,
        brightness: 1.0,
        day_brightness: 1.0,
        night_brightness: 1.0,
    }
}

/// A human label from a zone name and how close it sits: `Europe/Istanbul`
/// right on the spot becomes "Istanbul"; a nearby match becomes "≈ Istanbul".
fn place_label(zone: &str, exact: bool) -> String {
    let city = zone.rsplit('/').next().unwrap_or(zone).replace('_', " ");
    if exact { city } else { format!("~ {city}") }
}

/// The nearest-city label for a coordinate, if the zone table is readable.
fn place_for(lat: f64, lon: f64) -> Option<String> {
    let (zone, zone_lat, zone_lon) = nearest_zone(lat, lon)?;
    let exact = (zone_lat - lat).abs() < 0.5 && (zone_lon - lon).abs() < 0.5;
    Some(place_label(&zone, exact))
}

fn main() -> io::Result<()> {
    let (theme_index, tab, demo) = match parse_args() {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    let client = match Client::connect() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("nightlight-tui: cannot reach the session bus: {error}");
            std::process::exit(1);
        }
    };
    let mut app = App {
        client,
        status: None,
        fade: None,
        mismatch: false,
        band: Band::default(),
        band_known: false,
        last_poll: None,
        offset_secs: local_offset_seconds(),
        // The flag is a one-off override, like `--tab`: it dresses this run
        // without rewriting what you last chose from inside the dashboard.
        theme_index: theme_index.unwrap_or_else(remembered_theme),
        tab,
        settings_selected: 0,
        theme_popup: None,
        band_edit: None,
        help_popup: false,
        help_section: 0,
        sun_popup: false,
        map_popup: false,
        start_at_login: autostart::enabled(),
        picker: None,
        map_cam: None,
        picker_place: None,
        place: None,
        outputs: None,
        demo: demo.then(Instant::now),
        demo_cursor: 0,
        demo_key: None,
    };

    let mut terminal = ratatui::init();
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
}

/// Minimal argument parsing: `--theme <name>`, `--tab <name|number>` and
/// `--demo`. No clap — three flags do not justify a dependency.
fn parse_args() -> Result<(Option<usize>, usize, bool), String> {
    let theme_names = || {
        THEMES
            .iter()
            .map(|theme| theme.name)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let usage = || {
        format!(
            "usage: nightlight-tui [--version] [--help] [--theme <{}>] [--tab <{}>] [--demo]",
            theme_names(),
            TABS.join(", ")
        )
    };
    // `None` means the flag was not given, which is different from being
    // given `live`: the remembered theme fills the gap, and an explicit
    // `--theme live` overrides it for this run.
    let (mut theme_index, mut tab, mut demo) = (None, 0, false);
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            // Answered here rather than left to fall through to the usage
            // error: `--version` is the first thing a packager or a bug
            // report runs, and until now it got a complaint instead of an
            // answer.
            "--version" | "-V" => {
                println!("nightlight-tui {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!("{}", usage());
                std::process::exit(0);
            }
            "--theme" | "-t" => {
                let name = args.next().ok_or_else(usage)?;
                theme_index = Some(theme::index_of(&name).ok_or_else(|| {
                    format!("unknown theme {name:?} — available: {}", theme_names())
                })?);
            }
            "--tab" => {
                let name = args.next().ok_or_else(usage)?;
                tab = TABS
                    .iter()
                    .position(|title| **title == name)
                    .or_else(|| {
                        name.parse::<usize>()
                            .ok()
                            .filter(|n| (1..=TABS.len()).contains(n))
                            .map(|n| n - 1)
                    })
                    .ok_or_else(|| {
                        format!("unknown tab {name:?} — available: {}", TABS.join(", "))
                    })?;
            }
            "--demo" => demo = true,
            _ => return Err(usage()),
        }
    }
    Ok((theme_index, tab, demo))
}

impl App {
    /// Draw, wait briefly for a key, repeat. The wait doubles as the refresh
    /// pace; the status itself is re-read at most once a second.
    fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        loop {
            let poll_due = self
                .last_poll
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(1));
            if poll_due {
                self.status = self.client.status();
                self.outputs = self.client.outputs();
                self.fade = self.client.fade();
                let reported = self.client.band().map(|(day, night)| {
                    Band {
                        day_elevation: day,
                        night_elevation: night,
                    }
                    .sane()
                });
                // The schedule still needs a band to draw with, so it falls
                // back to the default; the settings rows need to know it was
                // a fallback, or they would offer to edit a band the daemon
                // has never heard of.
                self.band = reported.unwrap_or_default();
                self.band_known = reported.is_some();
                self.mismatch = self.status.is_none() && self.client.daemon_on_bus();
                self.last_poll = Some(Instant::now());
            }
            // The demo clock rewrites the snapshot every frame, so it runs
            // after the poll (which would overwrite it) and before the place
            // lookup (which should see the demo's synthesised location).
            self.apply_demo();
            self.run_demo_script();
            if poll_due {
                // Refresh the place name only when the location itself moved.
                if let Some(status) = self.status.as_ref().filter(|s| s.has_location) {
                    let moved = self.place.as_ref().is_none_or(|(lat, lon, _)| {
                        (lat - status.latitude).abs() > 1e-6
                            || (lon - status.longitude).abs() > 1e-6
                    });
                    if moved {
                        self.place = place_for(status.latitude, status.longitude)
                            .map(|name| (status.latitude, status.longitude, name));
                    }
                } else {
                    self.place = None;
                }
            }
            terminal.draw(|frame| self.draw(frame))?;
            // The demo redraws briskly so the sweep reads as motion; normal
            // use keeps the relaxed pace.
            let frame_wait = if self.demo.is_some() { 100 } else { 250 };
            if event::poll(Duration::from_millis(frame_wait))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
                && self.handle_key(key.code, key.modifiers)
            {
                return Ok(());
            }
        }
    }

    /// Handles one keypress; returns `true` to quit. Every daemon action
    /// invalidates the snapshot so the next frame shows the daemon's answer.
    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        // A modal popup owns the keyboard while it is open.
        if self.theme_popup.is_some() {
            self.popup_key(code);
            return false;
        }
        // The band editor is modal too, but deliberately not a curtain: it
        // sits in a corner of the curve so the shape it edits stays visible.
        if self.band_edit.is_some() {
            self.band_key(code);
            return false;
        }
        // The big map is a working surface, not just a view: the picker runs
        // inside it. `m` always closes; the rest belongs to the pick.
        if self.map_popup {
            if code == KeyCode::Char('m') {
                self.map_popup = false;
                return false;
            }
            if self.picker.is_some() {
                return self.picker_key(code);
            }
            match code {
                KeyCode::Enter => {
                    let start = self
                        .status
                        .as_ref()
                        .filter(|s| s.has_location)
                        .map(|s| (s.latitude, s.longitude))
                        .unwrap_or((0.0, 0.0));
                    self.picker = Some(start);
                }
                KeyCode::Char('c') => {
                    self.client.clear_location();
                    self.last_poll = None;
                }
                KeyCode::Esc | KeyCode::Char('q') => self.map_popup = false,
                _ => {}
            }
            return false;
        }
        // The help popup is a reference you walk through: one section open at
        // a time, the others folded to their titles.
        if self.help_popup {
            match code {
                KeyCode::Up | KeyCode::Left => {
                    self.help_section = (self.help_section + HELP.len() - 1) % HELP.len();
                }
                KeyCode::Down | KeyCode::Right | KeyCode::Tab => {
                    self.help_section = (self.help_section + 1) % HELP.len();
                }
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?') => {
                    self.help_popup = false;
                }
                _ => {}
            }
            return false;
        }
        if self.sun_popup {
            if matches!(
                code,
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('s')
            ) {
                self.sun_popup = false;
            }
            return false;
        }
        // So does the map picker (esc cancels the pick, q still quits).
        if self.picker.is_some() && self.tab == LOCATION_TAB {
            return self.picker_key(code);
        }
        match code {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => return true,
            KeyCode::Char('t') => {
                self.client.toggle();
                self.last_poll = None;
            }
            KeyCode::Char('a') => {
                self.client.follow_the_sun();
                self.last_poll = None;
            }
            KeyCode::Char('T') => {
                self.wear_theme((self.theme_index + 1) % THEMES.len());
            }
            KeyCode::Char('?') => self.help_popup = true,
            KeyCode::Char('s') => self.sun_popup = true,
            KeyCode::Char('r') => self.revive_daemon(),
            // Only where a curve is drawn, and only against a daemon that
            // has a band to edit: arrows that change nothing are worse than
            // no arrows.
            KeyCode::Char('b') if self.band_known && matches!(self.tab, NOW_TAB | TODAY_TAB) => {
                self.band_edit = Some(BandEdit {
                    original: self.band,
                    draft: self.band,
                    // The night bound first: it is the one people come to
                    // move, and the one both reviews complained about.
                    selected: 1,
                    confirming: false,
                });
            }
            KeyCode::Tab => {
                self.tab = (self.tab + 1) % TABS.len();
            }
            KeyCode::Char(digit @ '1'..='9') => {
                let index = (digit as usize) - ('1' as usize);
                if index < TABS.len() {
                    self.tab = index;
                }
            }
            // The settings tab owns the arrows and enter; the location tab
            // owns enter and c; elsewhere the arrows stay the night nudge.
            _ if self.tab == SETTINGS_TAB => self.settings_key(code),
            _ if self.tab == LOCATION_TAB => self.location_key(code),
            KeyCode::Up | KeyCode::Down => {
                if let Some(status) = &self.status {
                    let night = if code == KeyCode::Up {
                        status.night_temp.saturating_add(NIGHT_STEP)
                    } else {
                        status.night_temp.saturating_sub(NIGHT_STEP)
                    }
                    .max(NIGHT_MIN)
                    .min(status.day_temp);
                    self.client.set_night_temp(night);
                    self.last_poll = None;
                }
            }
            _ => {}
        }
        false
    }

    fn settings_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Up => self.settings_selected = self.settings_selected.saturating_sub(1),
            KeyCode::Down => {
                self.settings_selected = (self.settings_selected + 1).min(SETTINGS_ITEMS - 1);
            }
            KeyCode::Left | KeyCode::Right => self.adjust_setting(code == KeyCode::Right),
            KeyCode::Enter | KeyCode::Char(' ') => match self.settings_selected {
                4 => self.toggle_fade(),
                5 => self.theme_popup = Some(self.theme_index),
                6 => self.toggle_login(),
                _ => {}
            },
            _ => {}
        }
    }

    /// Left/right on the selected settings row: nudge the bounds (the daemon
    /// clamps and persists), cycle the theme, or flip the login toggle.
    ///
    /// The bounds are held by a `min` then a `max` rather than a `clamp`,
    /// because `clamp` panics when its two ends cross and a hand-written
    /// config can cross them. The order names the winner: the schedule's
    /// ordering outranks the control's own end.
    fn adjust_setting(&mut self, increase: bool) {
        match self.settings_selected {
            0 => {
                if let Some(status) = &self.status {
                    let day = if increase {
                        status.day_temp.saturating_add(NIGHT_STEP)
                    } else {
                        status.day_temp.saturating_sub(NIGHT_STEP)
                    }
                    .min(DAY_MAX)
                    .max(status.night_temp);
                    self.client.set_day_temp(day);
                    self.last_poll = None;
                }
            }
            1 => {
                if let Some(status) = &self.status {
                    let night = if increase {
                        status.night_temp.saturating_add(NIGHT_STEP)
                    } else {
                        status.night_temp.saturating_sub(NIGHT_STEP)
                    }
                    .max(NIGHT_MIN)
                    .min(status.day_temp);
                    self.client.set_night_temp(night);
                    self.last_poll = None;
                }
            }
            2 => {
                if let Some(status) = &self.status {
                    let step = if increase { FACTOR_STEP } else { -FACTOR_STEP };
                    let gamma = (status.gamma + step).clamp(GAMMA_UI_MIN, GAMMA_UI_MAX);
                    self.client.set_gamma(gamma);
                    self.last_poll = None;
                }
            }
            3 => {
                if let Some(status) = &self.status {
                    let step = if increase { FACTOR_STEP } else { -FACTOR_STEP };
                    let night = (status.night_brightness + step).clamp(0.1, 1.0);
                    // The day bound rides along unchanged; this row is the
                    // night dim, the knob Mumuskeh actually asked for.
                    self.client.set_brightness(status.day_brightness, night);
                    self.last_poll = None;
                }
            }
            4 => self.toggle_fade(),
            5 => {
                let count = THEMES.len();
                self.wear_theme(if increase {
                    (self.theme_index + 1) % count
                } else {
                    (self.theme_index + count - 1) % count
                });
            }
            6 => self.toggle_login(),
            _ => {}
        }
    }

    /// The band editor's keys (#45), over the curve they reshape: up and down
    /// pick a bound, left and right walk it half a degree at a time, enter
    /// applies, escape goes back. The curve redraws on the same frame as the
    /// press, which is the whole reason this is not a settings row.
    fn band_key(&mut self, code: KeyCode) {
        let Some(edit) = self.band_edit.as_mut() else {
            return;
        };
        // Escaping with an unapplied draft asks rather than throws the work
        // away; the question is the whole panel until it is answered.
        if edit.confirming {
            match code {
                KeyCode::Enter | KeyCode::Char('y') => self.apply_band(),
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('q') => self.band_edit = None,
                // Anything else is a change of mind about leaving.
                _ => edit.confirming = false,
            }
            return;
        }
        match code {
            KeyCode::Up => edit.selected = 0,
            KeyCode::Down => edit.selected = 1,
            KeyCode::Left | KeyCode::Right => {
                edit.draft = nudged_band(edit.draft, edit.selected == 0, code == KeyCode::Right);
            }
            // Back to redshift's band (#48). It fills the draft rather than
            // sending, so it arrives the same way every other change does:
            // drawn first, applied on enter, and escapable.
            KeyCode::Char('d') => edit.draft = Band::default(),
            KeyCode::Enter => self.apply_band(),
            KeyCode::Esc | KeyCode::Char('b') | KeyCode::Char('q') => {
                if edit.touched() {
                    edit.confirming = true;
                } else {
                    self.band_edit = None;
                }
            }
            _ => {}
        }
    }

    /// Sends the draft and closes. Adopted locally too, so the schedule
    /// answers before the next poll confirms it.
    fn apply_band(&mut self) {
        let Some(edit) = self.band_edit.take() else {
            return;
        };
        self.client
            .set_band(edit.draft.day_elevation, edit.draft.night_elevation);
        self.band = edit.draft;
        self.last_poll = None;
    }

    /// The band every curve and schedule here is drawn with: the editor's
    /// unapplied draft while it is open, the daemon's band otherwise. One
    /// place, so the picture can never disagree with itself.
    fn shown_band(&self) -> Band {
        self.band_edit.as_ref().map_or(self.band, |edit| edit.draft)
    }

    /// `r` on the no-daemon banner: start a stopped daemon (#43) or restart
    /// a mismatched one (#42). Does nothing while a daemon is answering.
    /// systemd first; a direct spawn covers a missing unit, with its stdio
    /// nulled so daemon logs never paint over this dashboard.
    fn revive_daemon(&mut self) {
        if self.status.is_some() {
            return;
        }
        let verb = if self.mismatch { "restart" } else { "start" };
        let unit = std::process::Command::new("systemctl")
            .args(["--user", verb, "nightlightd"])
            .status();
        if !unit.is_ok_and(|status| status.success()) && !self.mismatch {
            let _ = std::process::Command::new("nightlightd")
                .arg("--daemon")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
        }
        self.last_poll = None;
    }

    /// Flips the fade walk (#44), optimistically so the row answers the key
    /// at once; the next poll confirms. A dash row (daemon unreachable or
    /// pre-0.2.1) has nothing to flip.
    fn toggle_fade(&mut self) {
        if let Some(fade) = self.fade {
            self.client.set_fade(!fade);
            self.fade = Some(!fade);
            self.last_poll = None;
        }
    }

    /// Flips the systemd enablement and re-reads the truth, so a failed
    /// systemctl call shows as unchanged instead of as false success.
    fn toggle_login(&mut self) {
        autostart::set(!self.start_at_login);
        self.start_at_login = autostart::enabled();
    }

    /// Keys on the location tab while not picking: enter starts the picker at
    /// the active location, c returns to the timezone.
    fn location_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Enter => {
                let start = self
                    .status
                    .as_ref()
                    .filter(|s| s.has_location)
                    .map(|s| (s.latitude, s.longitude))
                    .unwrap_or((0.0, 0.0));
                self.picker = Some(start);
            }
            KeyCode::Char('c') => {
                self.client.clear_location();
                self.last_poll = None;
            }
            KeyCode::Char('m') => self.map_popup = true,
            _ => {}
        }
    }

    /// Keys while the map picker is active; returns `true` to quit the app.
    fn picker_key(&mut self, code: KeyCode) -> bool {
        let Some((lat, lon)) = self.picker else {
            return false;
        };
        match code {
            KeyCode::Up => self.picker = Some(((lat + PICK_LAT_STEP).min(MAP_LAT_MAX), lon)),
            KeyCode::Down => self.picker = Some(((lat - PICK_LAT_STEP).max(MAP_LAT_MIN), lon)),
            KeyCode::Right => self.picker = Some((lat, (lon + PICK_LON_STEP).min(179.0))),
            KeyCode::Left => self.picker = Some((lat, (lon - PICK_LON_STEP).max(-179.0))),
            KeyCode::Enter => {
                self.client.set_location(lat, lon);
                self.picker = None;
                self.picker_place = None;
                self.map_cam = None;
                self.last_poll = None;
            }
            KeyCode::Esc => {
                self.picker = None;
                self.picker_place = None;
                self.map_cam = None;
            }
            KeyCode::Char('q') => return true,
            _ => {}
        }
        // Keep the "what am I about to pin" label in step with the cursor.
        if let Some((lat, lon)) = self.picker {
            self.picker_place = place_for(lat, lon);
        }
        false
    }

    fn popup_key(&mut self, code: KeyCode) {
        let Some(selected) = self.theme_popup else {
            return;
        };
        match code {
            KeyCode::Up => self.theme_popup = Some(selected.saturating_sub(1)),
            KeyCode::Down => self.theme_popup = Some((selected + 1).min(THEMES.len() - 1)),
            KeyCode::Enter => {
                self.wear_theme(selected);
                self.theme_popup = None;
            }
            KeyCode::Esc | KeyCode::Char('q') => self.theme_popup = None,
            _ => {}
        }
    }

    /// Wears a theme and remembers it. Three keys reach the theme — `T`, the
    /// settings row's arrows, the picker's enter — and a save left off any
    /// one of them is a choice that survives until it happens to be made the
    /// other way.
    fn wear_theme(&mut self, index: usize) {
        self.theme_index = index;
        // A demo run rolls through every palette on its way past, and a
        // recording is not a session: without this the reel would leave
        // whoever recorded it wearing whatever the tour stopped on.
        if self.demo.is_none() {
            remember_theme(index);
        }
    }

    fn palette(&self) -> Palette {
        theme::palette(
            self.theme_index,
            self.status.as_ref().map(|s| s.temperature),
        )
    }

    fn draw(&mut self, frame: &mut Frame<'_>) {
        let pal = self.palette();
        let area = frame.area();
        // Paint the whole screen in the theme's background and text tones —
        // the palette owns the canvas, not the terminal's default colours.
        frame.render_widget(
            Block::default().style(Style::default().bg(pal.bg).fg(pal.text)),
            area,
        );
        // The full frame degrades piece by piece down to 76 columns (the sky
        // art, endpoint times and long wordings step back on the way); below
        // that, or on very short terminals, the compact view takes over. A
        // stock 80x24 terminal gets the full frame.
        if area.width < 76 || area.height < 22 {
            self.draw_compact(frame, area, &pal);
            return;
        }

        // Cap the frame like a page with a max-width: past 110 columns the
        // cards would just smear across the glass, so centre the app instead
        // and let the painted background own the margins.
        let width = area.width.min(110);
        let area = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y,
            width,
            height: area.height,
        };

        // The app frame: a sidebar owning identity, navigation and the live
        // summary; the content pane to the right of a hairline rule; the key
        // hints along the bottom.
        let [_, main, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(9),
            Constraint::Length(1),
        ])
        .areas(area);
        // The sidebar is a raised panel like the cards; the page-coloured
        // gutter between the patches is the separator — no hairline needed.
        let [_, sidebar, _, content, _] = Layout::horizontal([
            Constraint::Length(1),
            Constraint::Length(20),
            Constraint::Length(2),
            Constraint::Min(40),
            Constraint::Length(1),
        ])
        .areas(main);
        frame.render_widget(
            Block::new().style(Style::default().bg(pal.surface)),
            sidebar,
        );
        self.draw_sidebar(frame, sidebar, &pal);
        match self.tab {
            1 => self.draw_today_tab(frame, content, &pal),
            2 => self.draw_location_tab(frame, content, &pal),
            3 => self.draw_outputs_tab(frame, content, &pal),
            4 => self.draw_settings_tab(frame, content, &pal),
            _ => self.draw_now_tab(frame, content, &pal),
        }
        self.draw_footer(frame, footer, &pal);
        if self.theme_popup.is_some() {
            self.draw_theme_popup(frame, area, &pal);
        }
        if self.help_popup {
            self.draw_help_popup(frame, area, &pal);
        }
        if self.sun_popup {
            self.draw_sun_popup(frame, area, &pal);
        }
        if self.map_popup {
            self.draw_map_popup(frame, area, &pal);
        }

        // The demo's key chip: the scripted keypress, bottom-right for a
        // beat — a viewer must see the cause of every change on screen.
        if self.demo.is_some()
            && let Some((label, at)) = self.demo_key
            && at.elapsed() < Duration::from_millis(1100)
        {
            let text = format!(" {label} ");
            let width = text.chars().count() as u16;
            let chip = Rect {
                x: area.right().saturating_sub(width + 2),
                y: area.bottom().saturating_sub(2),
                width,
                height: 1,
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    text,
                    Style::default().fg(pal.bg).bg(pal.accent).bold(),
                ))),
                chip,
            );
        }
    }

    /// Tab 3: the world map — the resolved location marked on it, and a picker
    /// to pin a manual one. The map is ratatui's own braille world; `z` swaps
    /// between the whole world and a close-up that follows the cursor.
    fn draw_location_tab(&mut self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        // Two framed cards like the other tabs: the position summary on top,
        // the map below it. 10 tall to match the now tab's card row and the
        // today tab's schedule card, so the lower edge never jumps between
        // tabs.
        let [info_area, _, map_zone] = Layout::vertical([
            Constraint::Length(10),
            Constraint::Length(1),
            Constraint::Min(7),
        ])
        .areas(area);
        let info_card = card(" position ", pal).padding(Padding::new(2, 1, 1, 0));
        let info = info_card.inner(info_area);
        frame.render_widget(info_card, info_area);
        let map_card = card(" map ", pal);
        let map_area = map_card.inner(map_zone);
        frame.render_widget(map_card, map_zone);

        // The big city name: half the kelvin readout's size (octant pixels),
        // fed by the picker cursor while picking, the pinned place otherwise.
        let [name_area, text_area] =
            Layout::vertical([Constraint::Length(4), Constraint::Min(2)]).areas(info);
        let big_name = if self.picker.is_some() {
            self.picker_place.clone()
        } else {
            self.place.as_ref().map(|(_, _, name)| name.clone())
        };
        if let Some(name) = big_name {
            // The clock answers the city in the same big type: the place in
            // the accent on the left, its local time in the data hue on the
            // right. On a narrow card the city keeps the room to itself.
            let clock_width = if name_area.width >= 46 { 22 } else { 0 };
            let [name_col, clock_col] =
                Layout::horizontal([Constraint::Min(20), Constraint::Length(clock_width)])
                    .areas(name_area);
            frame.render_widget(
                BigText::builder()
                    .pixel_size(PixelSize::Quadrant)
                    .style(Style::default().fg(pal.accent))
                    .left_aligned()
                    .lines(vec![Line::from(name)])
                    .build(),
                name_col,
            );
            if clock_width > 0 {
                frame.render_widget(
                    BigText::builder()
                        .pixel_size(PixelSize::Quadrant)
                        .style(Style::default().fg(pal.accent2))
                        .right_aligned()
                        .lines(vec![Line::from(self.local_hhmm())])
                        .build(),
                    clock_col,
                );
            }
        }
        let info = text_area;

        let active = self
            .status
            .as_ref()
            .filter(|s| s.has_location)
            .map(|s| (s.latitude, s.longitude));
        let picker = self.picker;
        let accent = pal.accent;
        // Muted, not faint — the coastlines have to read against the dark
        // background rather than fade into it.
        let map_color = pal.muted;
        let text = pal.text;
        // The viewport. A card tall enough to hold the whole −55°..75° range
        // without heavy squashing gets the whole world — the big-screen view,
        // unchanged. A smaller card would smear that world flat, so it gets a
        // close-up around the pin instead, at the same detail density as the
        // big view (~0.75° per braille dot; dots are 2 across and 4 down per
        // cell, hence 1.5°·cols by 3°·rows) and therefore undistorted. While
        // picking, the close-up is a camera: it holds still while the cursor
        // moves through the middle of the view, and drags along once the
        // cursor crosses into the outer quarter, so with enough arrow presses
        // everywhere on earth stays reachable.
        let cols = f64::from(map_area.width.max(1));
        let rows = f64::from(map_area.height);
        let full_range = MAP_LAT_MAX - MAP_LAT_MIN;
        let (x_bounds, y_bounds) = if 720.0 * rows / cols >= full_range {
            ([-180.0, 180.0], [MAP_LAT_MIN, MAP_LAT_MAX])
        } else {
            let lon_span = (1.5 * cols).min(360.0);
            let lat_span = (3.0 * rows).min(full_range);
            let (half_lat, half_lon) = (lat_span / 2.0, lon_span / 2.0);
            // The centre: the camera while picking, else the pin, else the
            // mid-northern latitudes where most of the map's readers live.
            let (mut lat_c, mut lon_c) = match (picker, self.map_cam) {
                (Some(_), Some(camera)) => camera,
                _ => picker.or(active).unwrap_or((30.0, 0.0)),
            };
            if let Some((cursor_lat, cursor_lon)) = picker {
                let (m_lat, m_lon) = (lat_span * 0.25, lon_span * 0.25);
                lat_c = lat_c.clamp(cursor_lat + m_lat - half_lat, cursor_lat - m_lat + half_lat);
                lon_c = lon_c.clamp(cursor_lon + m_lon - half_lon, cursor_lon - m_lon + half_lon);
            }
            lat_c = lat_c.clamp(MAP_LAT_MIN + half_lat, MAP_LAT_MAX - half_lat);
            lon_c = lon_c.clamp(-180.0 + half_lon, 180.0 - half_lon);
            if picker.is_some() {
                self.map_cam = Some((lat_c, lon_c));
            }
            (
                [lon_c - half_lon, lon_c + half_lon],
                [lat_c - half_lat, lat_c + half_lat],
            )
        };
        let canvas = Canvas::default()
            // Paint the canvas in the theme background, or ratatui fills it
            // with the terminal default (a mid grey) and buries the map.
            .background_color(pal.surface)
            .marker(Marker::Braille)
            .x_bounds(x_bounds)
            .y_bounds(y_bounds)
            .paint(move |ctx| {
                ctx.draw(&Map {
                    resolution: MapResolution::High,
                    color: map_color,
                });
                if let Some((lat, lon)) = active {
                    ctx.print(
                        lon,
                        lat,
                        Span::styled("◉", Style::default().fg(accent).bold()),
                    );
                }
                if let Some((lat, lon)) = picker {
                    ctx.layer();
                    ctx.print(
                        lon,
                        lat,
                        Span::styled("✛", Style::default().fg(text).bold()),
                    );
                }
            });
        frame.render_widget(canvas, map_area);

        let lines = match (picker, &self.status) {
            (Some((lat, lon)), _) => vec![
                Line::from(Span::styled(
                    format!(" ✛ picking {}", format_coords(lat, lon)),
                    Style::default().fg(pal.accent).bold(),
                )),
                Line::from(Span::styled(
                    "   arrows move · ⏎ pin it · esc cancel",
                    Style::default().fg(pal.muted),
                )),
            ],
            (None, Some(status)) if status.has_location => vec![
                Line::from(vec![
                    Span::styled(" ◉ ", Style::default().fg(pal.accent)),
                    Span::styled(
                        format_coords(status.latitude, status.longitude),
                        Style::default().fg(pal.accent2),
                    ),
                    Span::styled(
                        format!("  ·  {}", status.source),
                        Style::default().fg(pal.muted),
                    ),
                ]),
                Line::from(Span::styled(
                    "   ⏎ pick a spot on the map · c use the timezone",
                    Style::default().fg(pal.faint),
                )),
            ],
            _ => vec![Line::from(Span::styled(
                " no location — ⏎ to pick one on the map",
                Style::default().fg(pal.muted),
            ))],
        };
        // A blank line between the big name and the details — air, not a wall.
        let mut spaced = vec![Line::default()];
        spaced.extend(lines);
        frame.render_widget(Paragraph::new(spaced), info);
    }

    /// Tab 4: the outputs — every CRTC the daemon is writing gamma ramps to,
    /// with its ramp size and the shared applied temperature. Per-output
    /// temperatures are #34 (v0.2); this tab is their future home.
    fn draw_outputs_tab(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let block = card(" outputs ", pal).padding(Padding::new(2, 1, 1, 0));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let Some(outputs) = self.outputs.as_ref().filter(|list| !list.is_empty()) else {
            let message = if self.status.is_some() {
                "no active outputs reported yet"
            } else {
                "daemon not running"
            };
            frame.render_widget(
                Paragraph::new(message).style(Style::default().fg(pal.muted)),
                inner,
            );
            return;
        };

        let applied = self
            .status
            .as_ref()
            .map(|s| format!("{} K", s.temperature))
            .unwrap_or_else(|| "—".into());
        let rows: Vec<Row<'_>> = outputs
            .iter()
            .map(|(crtc, ramp)| {
                Row::new(vec![
                    Cell::from(format!("CRTC {crtc}")),
                    Cell::from(Span::styled(
                        format!("{ramp} steps"),
                        Style::default().fg(pal.accent2),
                    )),
                    Cell::from(Span::styled(
                        applied.clone(),
                        Style::default().fg(pal.accent2),
                    )),
                ])
            })
            .collect();
        let table_height = (outputs.len() + 1) as u16;
        let [table_area, note_area] =
            Layout::vertical([Constraint::Length(table_height + 1), Constraint::Min(0)])
                .areas(inner);
        frame.render_widget(
            Table::new(
                rows,
                [
                    Constraint::Length(12),
                    Constraint::Length(12),
                    Constraint::Min(8),
                ],
            )
            .header(
                Row::new(vec!["output", "gamma ramp", "applied"])
                    .style(Style::default().fg(pal.faint)),
            ),
            table_area,
        );
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "every output wears the same temperature today",
                    Style::default().fg(pal.muted),
                )),
                Line::from(Span::styled(
                    "per-output control is #34 — this is where it lands",
                    Style::default().fg(pal.faint),
                )),
            ]),
            note_area,
        );
    }

    /// Tab 5: the settings — the two bounds, the theme, autostart, and where
    /// the config lives. Row-based: arrows select and adjust, enter acts.
    fn draw_settings_tab(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let block = card(" settings ", pal).padding(Padding::new(2, 1, 1, 0));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let value = |v: Option<String>| v.unwrap_or_else(|| "—".into());
        let day = value(self.status.as_ref().map(|s| format!("{} K", s.day_temp)));
        let night = value(self.status.as_ref().map(|s| format!("{} K", s.night_temp)));
        let gamma = value(self.status.as_ref().map(|s| format!("{:.2}", s.gamma)));
        let dim = value(
            self.status
                .as_ref()
                .map(|s| format!("{:.0}%", s.night_brightness * 100.0)),
        );
        // Each slider row carries (value, min, max) for the rail underneath.
        // Both temperature rails share one scale, so the two bounds can be
        // read against each other rather than each against itself.
        let day_rail = self
            .status
            .as_ref()
            .map(|s| (f64::from(s.day_temp), RAIL_RANGE.0, RAIL_RANGE.1));
        let night_rail = self
            .status
            .as_ref()
            .map(|s| (f64::from(s.night_temp), RAIL_RANGE.0, RAIL_RANGE.1));
        let gamma_rail = self
            .status
            .as_ref()
            .map(|s| (s.gamma, GAMMA_UI_MIN, GAMMA_UI_MAX));
        let dim_rail = self.status.as_ref().map(|s| (s.night_brightness, 0.1, 1.0));
        let rows: [(&str, String, &str, Option<Rail>); SETTINGS_ITEMS] = [
            ("daytime", day, "‹ › adjust", day_rail),
            ("nighttime", night, "‹ › adjust", night_rail),
            ("gamma", gamma, "‹ › adjust", gamma_rail),
            ("night dim", dim, "‹ › adjust", dim_rail),
            (
                "fade",
                match self.fade {
                    Some(true) => "[x] on".to_string(),
                    Some(false) => "[ ] off".to_string(),
                    None => "—".to_string(),
                },
                "⏎ toggle",
                None,
            ),
            (
                "theme",
                THEMES[self.theme_index].name.to_string(),
                "⏎ choose · ‹ › cycle",
                None,
            ),
            (
                "start at login",
                if self.start_at_login {
                    "[x] enabled".to_string()
                } else {
                    "[ ] disabled".to_string()
                },
                "⏎ toggle",
                None,
            ),
        ];

        // Every row costs a line, a rail row one more beneath it, and a blank
        // between rows when the card can afford one. It cannot always: the two
        // band rows (#45) pushed the list past a short terminal, and a clipped
        // settings list hides the row you were reaching for. Breathing room is
        // the first thing to go, the rows themselves the last.
        let rails = rows.iter().filter(|(.., rail)| rail.is_some()).count() as u16;
        let dense = SETTINGS_ITEMS as u16 + rails + 2;
        let roomy = inner.height >= dense + SETTINGS_ITEMS as u16;

        let mut lines: Vec<Line<'_>> = Vec::new();
        let mut sliders: Vec<(u16, Rail, bool)> = Vec::new();
        for (index, (label, val, hint, slider)) in rows.into_iter().enumerate() {
            let selected = index == self.settings_selected;
            let body = format!(" {label:<16} {val:<14}");
            if selected {
                lines.push(Line::from(vec![
                    Span::styled(body, Style::default().fg(pal.bg).bg(pal.accent).bold()),
                    Span::styled(format!("  {hint}"), Style::default().fg(pal.muted)),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(format!(" {label:<16} "), Style::default().fg(pal.muted)),
                    Span::styled(format!("{val:<14}"), Style::default().fg(pal.text)),
                ]));
            }
            if let Some(rail) = slider {
                sliders.push((inner.y + lines.len() as u16, rail, selected));
                lines.push(Line::default());
            }
            if roomy {
                lines.push(Line::default());
            }
        }
        lines.push(Line::from(Span::styled(
            format!(" config  {}", config_path_display()),
            Style::default().fg(pal.faint),
        )));
        lines.push(Line::from(Span::styled(
            "         day & night changes persist there automatically",
            Style::default().fg(pal.faint),
        )));
        let content_rows = lines.len() as u16;
        frame.render_widget(Paragraph::new(lines), inner);

        // Credit where credit is due, sized to the room the content leaves:
        // the official two-row logo when it fits clear of the text, one line
        // of plain words when the card is short, nothing when there is no
        // room at all. Never clipped — a half-eaten logo credits nobody.
        let room = inner.height.saturating_sub(content_rows);
        if inner.width > 40 && room >= 4 {
            let logo_area = Rect {
                x: inner.right().saturating_sub(18),
                y: inner.bottom().saturating_sub(3),
                width: 15,
                height: 2,
            };
            frame.render_widget(RatatuiLogo::tiny(), logo_area);
            // The logo widget carries no style of its own; painting the area
            // afterwards lifts it to legible-but-secondary, the same tier as
            // the map's coastlines.
            frame
                .buffer_mut()
                .set_style(logo_area, Style::default().fg(pal.muted));
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "powered by",
                    Style::default().fg(pal.muted),
                )))
                .alignment(Alignment::Right),
                Rect {
                    x: inner.x,
                    y: inner.bottom().saturating_sub(4),
                    width: inner.width.saturating_sub(3),
                    height: 1,
                },
            );
        } else if inner.width > 30 && room >= 1 {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "powered by ratatui",
                    Style::default().fg(pal.muted),
                )))
                .alignment(Alignment::Right),
                Rect {
                    x: inner.x,
                    y: inner.bottom().saturating_sub(1),
                    width: inner.width.saturating_sub(3),
                    height: 1,
                },
            );
        }

        // The temperature rows carry a real slider (tui-slider) on the row
        // reserved beneath each — the segmented ▰▱ style, no thumb: filled to
        // where the bound sits in the shared [`RAIL_RANGE`], ticks in
        // the accent so they track the live tint; the empty ticks brighten when
        // their row is selected.
        for (y, (value, min, max), selected) in sliders {
            let width = inner.width.saturating_sub(6).min(28);
            if width < 2 {
                continue;
            }
            let rail = if selected { pal.muted } else { pal.faint };
            let slider = Slider::new(value.clamp(min, max), min, max)
                .show_value(false)
                .show_handle(false)
                .filled_symbol("▰")
                .empty_symbol("▱")
                .filled_color(pal.accent)
                .empty_color(rail);
            frame.render_widget(
                slider,
                Rect {
                    x: inner.x + 3,
                    y,
                    width,
                    height: 1,
                },
            );
        }
    }

    /// The footer: the mode as a status lamp bottom-left, the key hints
    /// tucked to the right — the quietest layer of the screen: accent keys,
    /// muted labels, faint dots, no fills.
    fn draw_footer(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let lamp = match &self.status {
            Some(status) => {
                let (dot, colour) = if status.enabled {
                    ("●", pal.ok)
                } else {
                    ("○", pal.err)
                };
                let mode = if !status.enabled {
                    "OFF"
                } else if status.following {
                    "AUTO"
                } else {
                    "MANUAL"
                };
                Line::from(vec![
                    Span::styled(format!(" {dot} "), Style::default().fg(colour)),
                    Span::styled(mode, Style::default().fg(pal.accent).bold()),
                ])
            }
            None => Line::from(Span::styled(" ○ offline", Style::default().fg(pal.err))),
        };
        frame.render_widget(Paragraph::new(lamp), area);

        // Three keys, the same three on every tab. The footer had grown to
        // nine and read as a toolbar; the full set lives behind `?` now,
        // where it can be laid out properly instead of queued along one row.
        // What stays is what you need before you know anything: how to move
        // between tabs, how to find the rest, how to leave.
        let pairs = [("⇥", "tab"), ("?", "keys"), ("q", "quit")];
        let mut spans = Vec::new();
        for (index, (key, label)) in pairs.into_iter().enumerate() {
            if index > 0 {
                spans.push(Span::styled(" · ", Style::default().fg(pal.faint)));
            }
            spans.push(Span::styled(
                key.to_string(),
                Style::default().fg(pal.accent).bold(),
            ));
            spans.push(Span::styled(
                format!(" {label}"),
                Style::default().fg(pal.muted),
            ));
        }
        spans.push(Span::raw(" "));
        frame.render_widget(
            Paragraph::new(Line::from(spans)).alignment(Alignment::Right),
            area,
        );
    }

    /// The band editor (#45): the two transition bounds, live over the curve
    /// they reshape. Anchored to the top left of the curve instead of centred
    /// like every other popup, because here the feedback *is* the point — a
    /// centred panel would cover the only thing worth watching. That corner
    /// is the emptiest part of both pictures: midnight sits at the left edge,
    /// where the schedule is at full night and the line is on the floor.
    fn draw_band_editor(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let Some(edit) = self.band_edit.as_ref() else {
            return;
        };
        let (width, height) = (32u16, 7u16);
        if area.width < width || area.height < height {
            return;
        }
        // Inset by one where there is room to spare, flush to the corner
        // where there is not — the card can be exactly the panel's size, and
        // a panel hanging over the edge of its own card is worse than a
        // panel with no margin.
        let popup = Rect {
            x: (area.x + 1).min(area.right() - width),
            y: (area.y + 1).min(area.bottom() - height),
            width,
            height,
        };
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Block::new().style(Style::default().bg(pal.overlay).fg(pal.text)),
            popup,
        );

        let bound = |label: &str, degrees: f64, picked: bool| {
            let reading = format!("{degrees:+.1}°");
            let (open, close) = if picked {
                ("‹ ", " ›")
            } else {
                ("  ", "  ")
            };
            let tint = if picked { pal.accent } else { pal.text };
            Line::from(vec![
                Span::styled(format!(" {label:<12}"), Style::default().fg(pal.muted)),
                Span::styled(open.to_string(), Style::default().fg(pal.accent)),
                Span::styled(format!("{reading:>6}"), Style::default().fg(tint)),
                Span::styled(close.to_string(), Style::default().fg(pal.accent)),
            ])
        };
        let title = Line::from(Span::styled(
            " transition band",
            Style::default().fg(pal.accent2),
        ));
        let hint = |text: &str| {
            Line::from(Span::styled(
                format!(" {text}"),
                Style::default().fg(pal.faint),
            ))
        };
        let lines = if edit.confirming {
            vec![
                title,
                Line::from(Span::styled(
                    " not applied yet",
                    Style::default().fg(pal.accent),
                )),
                Line::from(Span::styled(
                    " apply the new band?",
                    Style::default().fg(pal.text),
                )),
                hint("⏎ apply · esc revert"),
                Line::default(),
            ]
        } else {
            vec![
                title,
                bound("day above", edit.draft.day_elevation, edit.selected == 0),
                bound(
                    "night below",
                    edit.draft.night_elevation,
                    edit.selected == 1,
                ),
                // Two hint rows rather than one crowded line: the keys that
                // move things, then the keys that decide. The draft is drawn
                // but unsent, so which one commits it has to be said.
                hint("↑↓ ‹› adjust · d default"),
                hint(if edit.touched() {
                    "⏎ apply · esc revert"
                } else {
                    "esc close"
                }),
            ]
        };
        frame.render_widget(
            Paragraph::new(lines),
            Rect {
                x: popup.x + 1,
                y: popup.y + 1,
                width: popup.width.saturating_sub(2),
                height: popup.height.saturating_sub(2),
            },
        );
    }

    /// Clears and paints a centred overlay surface — the shared chrome of
    /// every popup: the lighter shade does the lifting, no border. Returns
    /// the padded inner area.
    fn overlay(frame: &mut Frame<'_>, area: Rect, width: u16, height: u16, pal: &Palette) -> Rect {
        let width = width.min(area.width);
        let height = height.min(area.height);
        let popup = Rect {
            x: area.x + (area.width.saturating_sub(width)) / 2,
            y: area.y + (area.height.saturating_sub(height)) / 2,
            width,
            height,
        };
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Block::new().style(Style::default().bg(pal.overlay).fg(pal.text)),
            popup,
        );
        Rect {
            x: popup.x + 2,
            y: popup.y + 1,
            width: popup.width.saturating_sub(4),
            height: popup.height.saturating_sub(2),
        }
    }

    /// A popup's first row: the title in the accent, the closing hint tucked
    /// against the opposite edge.
    fn overlay_title(frame: &mut Frame<'_>, inner: Rect, title: &str, hint: &str, pal: &Palette) {
        let row = Rect { height: 1, ..inner };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                title.to_string(),
                Style::default().fg(pal.accent).bold(),
            ))),
            row,
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                hint.to_string(),
                Style::default().fg(pal.muted),
            )))
            .alignment(Alignment::Right),
            row,
        );
    }

    /// `?`: every key in one place, folded into an accordion. Section titles
    /// are always visible so the shape of the thing is legible at a glance;
    /// the open one shows its keys. Sized to the tallest section, so walking
    /// through them never resizes the box under the eye.
    fn draw_help_popup(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let tallest = HELP.iter().map(|(_, keys)| keys.len()).max().unwrap_or(0);
        let content = 2 + HELP.len() + tallest + 2;
        let inner = Self::overlay(frame, area, 52, content as u16 + 2, pal);
        Self::overlay_title(frame, inner, "keys", "↑↓ section · esc", pal);

        let mut lines = vec![Line::default()];
        for (index, (title, keys)) in HELP.iter().enumerate() {
            let open = index == self.help_section;
            let (marker, tint) = if open {
                ("▾ ", pal.accent2)
            } else {
                ("▸ ", pal.muted)
            };
            lines.push(Line::from(vec![
                Span::styled(marker, Style::default().fg(tint)),
                Span::styled(title.to_string(), Style::default().fg(tint)),
            ]));
            if !open {
                continue;
            }
            for (key, label) in keys.iter() {
                lines.push(Line::from(vec![
                    Span::styled(
                        format!("    {key:<8}"),
                        Style::default().fg(pal.accent).bold(),
                    ),
                    Span::styled(label.to_string(), Style::default().fg(pal.muted)),
                ]));
            }
        }
        frame.render_widget(
            Paragraph::new(lines),
            Rect {
                y: inner.y + 1,
                height: inner.height.saturating_sub(2),
                ..inner
            },
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(
                    "v{} · github.com/umutdinceryananer/nightlightd",
                    env!("CARGO_PKG_VERSION")
                ),
                Style::default().fg(pal.faint),
            ))),
            Rect {
                y: inner.y + inner.height.saturating_sub(1),
                height: 1,
                ..inner
            },
        );
    }

    /// `s`: the solar facts behind the dashboard's summaries — day length and
    /// how it drifts, solar noon, the day's frame, tomorrow's sunrise. All
    /// pure maths from the same milestones the schedule uses.
    fn draw_sun_popup(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let Some(status) = self.status.as_ref().filter(|s| s.has_location) else {
            let inner = Self::overlay(frame, area, 36, 5, pal);
            Self::overlay_title(frame, inner, "sun", "esc", pal);
            frame.render_widget(
                Paragraph::new(vec![
                    Line::default(),
                    Line::default(),
                    Line::from(Span::styled(
                        "no location resolved",
                        Style::default().fg(pal.muted),
                    )),
                ]),
                inner,
            );
            return;
        };

        let (midnight, _) = self.day_context();
        let compute = |offset_days: f64| {
            milestones(
                status.latitude,
                status.longitude,
                midnight + offset_days * 86_400.0,
                self.shown_band(),
                status.day_temp,
                status.night_temp,
            )
        };
        let today_events = compute(0.0);
        let yesterday = compute(-1.0);
        let tomorrow = compute(1.0);
        let hour_of = |events: &[Milestone], name: &str| {
            events.iter().find(|e| e.name == name).map(|e| e.hour)
        };
        let hhmm_of = |events: &[Milestone], name: &str| {
            events.iter().find(|e| e.name == name).map(|e| e.hhmm())
        };

        let mut rows: Vec<(String, String)> = Vec::new();
        match (
            hour_of(&today_events, "sunrise"),
            hour_of(&today_events, "sunset"),
        ) {
            (Some(rise), Some(set)) => {
                let length = set - rise;
                rows.push(("day length".into(), hm(length)));
                if let (Some(y_rise), Some(y_set)) = (
                    hour_of(&yesterday, "sunrise"),
                    hour_of(&yesterday, "sunset"),
                ) {
                    let delta = length - (y_set - y_rise);
                    let seconds = (delta.abs() * 3600.0).round() as i64;
                    let word = if delta >= 0.0 { "longer" } else { "shorter" };
                    rows.push((
                        "vs yesterday".into(),
                        format!("{}m {:02}s {word}", seconds / 60, seconds % 60),
                    ));
                }
                rows.push((
                    "sunrise → sunset".into(),
                    format!(
                        "{} → {}",
                        hhmm_of(&today_events, "sunrise").unwrap_or_default(),
                        hhmm_of(&today_events, "sunset").unwrap_or_default()
                    ),
                ));
            }
            _ => {
                let which = if status.elevation > 0.0 {
                    "polar day — the sun does not set"
                } else {
                    "polar night — the sun does not rise"
                };
                rows.push(("today".into(), which.into()));
            }
        }
        if let Some(noon) = today_events.iter().find(|e| e.name == "solar noon") {
            let elevation = solar_elevation(
                status.latitude,
                status.longitude,
                midnight + noon.hour * 3600.0,
            );
            rows.push((
                "solar noon".into(),
                format!("{} · {:+.1}°", noon.hhmm(), elevation),
            ));
        }
        if let (Some(rise), Some(t_rise)) = (
            hour_of(&today_events, "sunrise"),
            hhmm_of(&tomorrow, "sunrise"),
        ) {
            let t_hour = hour_of(&tomorrow, "sunrise").unwrap_or(rise);
            let minutes = ((t_hour - rise) * 60.0).round() as i64;
            rows.push((
                "tomorrow's sunrise".into(),
                format!("{t_rise} ({minutes:+}m)"),
            ));
        }
        rows.push(("sun right now".into(), format!("{:+.1}°", status.elevation)));

        let mut lines = vec![Line::default(), Line::default()];
        for (label, value) in rows {
            lines.push(Line::from(vec![
                Span::styled(format!("{label:<20}"), Style::default().fg(pal.muted)),
                Span::styled(value, Style::default().fg(pal.accent2)),
            ]));
            lines.push(Line::default());
        }
        lines.pop();
        let inner = Self::overlay(frame, area, 48, lines.len() as u16 + 2, pal);
        Self::overlay_title(frame, inner, "sun", "esc", pal);
        frame.render_widget(Paragraph::new(lines), inner);
    }

    /// `m` on the location tab: the world at full size, with the picker live
    /// inside it. The lower card gives the map a strip; this gives it the
    /// screen, which on most terminals is enough rows to show the whole
    /// −55°..75° range undistorted.
    fn draw_map_popup(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let inner = Self::overlay(
            frame,
            area,
            area.width.saturating_sub(4),
            area.height.saturating_sub(2),
            pal,
        );
        // The title row carries the context: where the cursor (or the pin)
        // sits, and the local clock; the hint follows the picking state.
        let (spot, spot_place) = match self.picker {
            Some((lat, lon)) => (Some((lat, lon)), self.picker_place.clone()),
            None => (
                self.status
                    .as_ref()
                    .filter(|s| s.has_location)
                    .map(|s| (s.latitude, s.longitude)),
                self.place.as_ref().map(|(_, _, name)| name.clone()),
            ),
        };
        let mut title = vec![Span::styled("map", Style::default().fg(pal.accent).bold())];
        if let Some((lat, lon)) = spot {
            let place = spot_place.unwrap_or_else(|| "somewhere".into());
            title.push(Span::styled(
                format!("  {place} · {}", format_coords(lat, lon)),
                Style::default().fg(pal.muted),
            ));
            title.push(Span::styled(" · ", Style::default().fg(pal.faint)));
            title.push(Span::styled(
                self.local_hhmm(),
                Style::default().fg(pal.accent2),
            ));
        }
        let title_row = Rect { height: 1, ..inner };
        frame.render_widget(Paragraph::new(Line::from(title)), title_row);
        let hint = if self.picker.is_some() {
            "←↑↓→ move · ⏎ pin · esc"
        } else {
            "⏎ pick · c timezone · esc"
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                hint,
                Style::default().fg(pal.muted),
            )))
            .alignment(Alignment::Right),
            title_row,
        );
        let map_area = Rect {
            x: inner.x,
            y: inner.y + 2,
            width: inner.width,
            height: inner.height.saturating_sub(2),
        };
        let active = self
            .status
            .as_ref()
            .filter(|s| s.has_location)
            .map(|s| (s.latitude, s.longitude));
        let picker = self.picker;
        let accent = pal.accent;
        let map_color = pal.muted;
        let text = pal.text;
        let canvas = Canvas::default()
            .background_color(pal.overlay)
            .marker(Marker::Braille)
            .x_bounds([-180.0, 180.0])
            .y_bounds([MAP_LAT_MIN, MAP_LAT_MAX])
            .paint(move |ctx| {
                ctx.draw(&Map {
                    resolution: MapResolution::High,
                    color: map_color,
                });
                if let Some((lat, lon)) = active {
                    ctx.print(
                        lon,
                        lat,
                        Span::styled("◉", Style::default().fg(accent).bold()),
                    );
                }
                if let Some((lat, lon)) = picker {
                    ctx.layer();
                    ctx.print(
                        lon,
                        lat,
                        Span::styled("✛", Style::default().fg(text).bold()),
                    );
                }
            });
        frame.render_widget(canvas, map_area);
    }

    /// The theme picker: a floating overlay in the noodle idiom — no border,
    /// a visibly lighter surface doing the lifting, a title row with the key
    /// hint opposite it, and full-width selection bars.
    fn draw_theme_popup(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let Some(selected) = self.theme_popup else {
            return;
        };
        let width = 32.min(area.width);
        let height = (THEMES.len() as u16 + 4).min(area.height);
        let popup = Rect {
            x: area.x + (area.width.saturating_sub(width)) / 2,
            y: area.y + (area.height.saturating_sub(height)) / 2,
            width,
            height,
        };
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Block::new().style(Style::default().bg(pal.overlay).fg(pal.text)),
            popup,
        );
        let inner = Rect {
            x: popup.x + 2,
            y: popup.y + 1,
            width: popup.width.saturating_sub(4),
            height: popup.height.saturating_sub(2),
        };

        let mut lines: Vec<Line<'_>> = vec![
            Line::from(Span::styled(
                "theme",
                Style::default().fg(pal.accent).bold(),
            )),
            Line::default(),
        ];
        let row_width = usize::from(inner.width);
        lines.extend(THEMES.iter().enumerate().map(|(index, theme)| {
            let current = if index == self.theme_index {
                "•"
            } else {
                " "
            };
            let body = format!(" {current} {:<width$}", theme.name, width = row_width - 3);
            if index == selected {
                Line::from(Span::styled(
                    body,
                    Style::default().fg(pal.bg).bg(pal.accent).bold(),
                ))
            } else {
                Line::from(Span::styled(body, Style::default().fg(pal.text)))
            }
        }));
        frame.render_widget(Paragraph::new(lines), inner);
        // The hint, opposite the title on the same row.
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "⏎ apply · esc",
                Style::default().fg(pal.muted),
            )))
            .alignment(Alignment::Right),
            Rect {
                x: inner.x,
                y: inner.y,
                width: inner.width,
                height: 1,
            },
        );
    }

    /// The sidebar: brand and daemon state up top, the tab list, and a live
    /// summary pinned to the bottom — the glance that works from every tab.
    fn draw_sidebar(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let [_, brand, _, nav, _, summary] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(TABS.len() as u16),
            Constraint::Min(1),
            Constraint::Length(9),
        ])
        .areas(area);

        // The brand: a moon, then the name in two tones — "night" quiet,
        // "lightd" lit. Reads as a wordmark without needing figlet rows.
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  ☾ ", Style::default().fg(pal.accent)),
                Span::styled("night", Style::default().fg(pal.text).bold()),
                Span::styled("lightd", Style::default().fg(pal.accent).bold()),
            ])),
            brand,
        );

        // Navigation: numbered rows, the digits being the shortcut hint; the
        // active tab is a full-width accent bar across the panel.
        let bar_width = usize::from(area.width).saturating_sub(5);
        let items: Vec<Line<'_>> = TABS
            .iter()
            .enumerate()
            .map(|(index, name)| {
                if index == self.tab {
                    Line::from(Span::styled(
                        format!(" {}  {:<bar_width$}", index + 1, name),
                        Style::default().fg(pal.bg).bg(pal.accent).bold(),
                    ))
                } else {
                    Line::from(vec![
                        Span::styled(format!(" {}  ", index + 1), Style::default().fg(pal.faint)),
                        Span::styled(*name, Style::default().fg(pal.muted)),
                    ])
                }
            })
            .collect();
        frame.render_widget(Paragraph::new(items), nav);

        // The live summary, kept to three plain lines with air between them:
        // the phase, the next sun event, the place and clock. Every line is
        // written to fit the panel's fixed 18 usable columns — "transition"
        // is the yardstick — so nothing ever clips; the elevation itself
        // lives on the now tab and in the `s` overlay.
        let width = usize::from(area.width).saturating_sub(4);
        let mut lines: Vec<Line<'_>> = Vec::new();
        if let Some(status) = self.status.as_ref().filter(|s| s.has_location) {
            let phase = phase(status.elevation, self.shown_band());
            let icon = match phase {
                "day" => "☀",
                "night" => "☾",
                _ => "◐",
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  {icon} "), Style::default().fg(pal.accent)),
                Span::styled(phase, Style::default().fg(pal.text)),
            ]));
            lines.push(Line::default());
            let (_, short, _, _) = self.daylight(status);
            let label = if let Some(rest) = short.strip_prefix("in ") {
                format!("sunrise {rest}")
            } else if short.starts_with("polar") {
                short
            } else {
                format!("{short} of light")
            };
            lines.push(Line::from(Span::styled(
                format!("  {label}"),
                Style::default().fg(pal.muted),
            )));
            lines.push(Line::default());
            let place = self
                .place
                .as_ref()
                .map(|(_, _, name)| name.as_str())
                .unwrap_or("resolved");
            let mut where_line = format!("{place} · {}", self.local_hhmm());
            if where_line.chars().count() > width {
                where_line = where_line.chars().take(width).collect();
            }
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(where_line, Style::default().fg(pal.muted)),
            ]));
        } else {
            lines.push(Line::from(Span::styled(
                "  no location",
                Style::default().fg(pal.muted),
            )));
        }
        while lines.len() < 7 {
            lines.push(Line::default());
        }
        lines.push(Line::from(Span::styled(
            format!(
                "  v{} · {}",
                env!("CARGO_PKG_VERSION"),
                THEMES[self.theme_index].name
            ),
            Style::default().fg(pal.faint),
        )));
        frame.render_widget(Paragraph::new(lines), summary);
    }

    /// Tab 1: the dashboard — state cards on top, the curve below. The cards
    /// row is 10 tall to match the today tab's schedule card (7 events plus
    /// header plus borders), so the curve sits at the same height on both tabs
    /// and does not jump when switching.
    fn draw_now_tab(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        // One-cell gutters between the raised cards, or their surfaces fuse
        // into a single slab.
        let [cards, _, curve] = Layout::vertical([
            Constraint::Length(10),
            Constraint::Length(1),
            Constraint::Min(4),
        ])
        .areas(area);
        let [now_card, _, sun_card] = Layout::horizontal([
            Constraint::Length(28),
            Constraint::Length(2),
            Constraint::Min(30),
        ])
        .areas(cards);
        self.draw_now_card(frame, now_card, pal);
        self.draw_sun_card(frame, sun_card, pal);
        self.draw_curve_card(frame, curve, pal);
        self.draw_band_editor(frame, curve, pal);
    }

    /// Tab 2: the day's solar milestones, derived from the real curve, with
    /// the next event highlighted — then the curve for context.
    fn draw_today_tab(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let Some(status) = self.status.as_ref().filter(|s| s.has_location) else {
            let block = card(" today ", pal).padding(Padding::new(2, 1, 1, 0));
            let inner = block.inner(area);
            frame.render_widget(block, area);
            frame.render_widget(
                Paragraph::new("no location — the schedule needs one")
                    .style(Style::default().fg(pal.muted)),
                inner,
            );
            return;
        };

        let (midnight, now_hour) = self.day_context();
        let events = milestones(
            status.latitude,
            status.longitude,
            midnight,
            self.shown_band(),
            status.day_temp,
            status.night_temp,
        );
        let next = events.iter().position(|e| e.hour > now_hour);

        // Two raised cards, like the now tab: the schedule table and the
        // curve, a gutter between them so the surfaces stay distinct.
        let table_height = (events.len() + 1) as u16;
        let [schedule_area, _, curve_area] = Layout::vertical([
            Constraint::Length(table_height + 3),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .areas(area);
        let schedule = card(" schedule ", pal).padding(Padding::new(2, 1, 1, 1));
        let table_area = schedule.inner(schedule_area);
        frame.render_widget(schedule, schedule_area);

        let rows: Vec<Row<'_>> = events
            .iter()
            .enumerate()
            .map(|(i, event)| {
                if Some(i) == next {
                    Row::new(vec![
                        event.name.to_string(),
                        event.hhmm(),
                        format!("{} K", event.kelvin),
                        relative(event.hour - now_hour),
                    ])
                    .style(Style::default().fg(pal.bg).bg(pal.accent).bold())
                } else if event.hour < now_hour {
                    Row::new(vec![
                        event.name.to_string(),
                        event.hhmm(),
                        format!("{} K", event.kelvin),
                        relative(event.hour - now_hour),
                    ])
                    .style(Style::default().fg(pal.muted))
                } else {
                    Row::new(vec![
                        Cell::from(event.name.to_string()),
                        Cell::from(Span::styled(event.hhmm(), Style::default().fg(pal.accent2))),
                        Cell::from(Span::styled(
                            format!("{} K", event.kelvin),
                            Style::default().fg(pal.accent2),
                        )),
                        Cell::from(Span::styled(
                            relative(event.hour - now_hour),
                            Style::default().fg(pal.muted),
                        )),
                    ])
                }
            })
            .collect();
        let table = Table::new(
            rows,
            [
                Constraint::Length(14),
                Constraint::Length(7),
                Constraint::Length(8),
                Constraint::Min(10),
            ],
        )
        .header(
            Row::new(vec!["event", "time", "kelvin", "when"]).style(Style::default().fg(pal.faint)),
        );
        frame.render_widget(table, table_area);

        if curve_area.height >= 7 {
            let arc = card(" sun arc ", pal).padding(Padding::new(1, 1, 0, 0));
            let chart_area = arc.inner(curve_area);
            frame.render_widget(arc, curve_area);
            self.draw_sun_arc(frame, chart_area, pal);
            self.draw_band_editor(frame, curve_area, pal);
        } else {
            // No arc to sit over on a short terminal, but the keys are still
            // captured — the editor has to appear somewhere or it is a
            // keyboard trap.
            self.draw_band_editor(frame, area, pal);
        }
    }

    /// Local midnight (unix) and the fractional local hour of "now".
    /// Under `--demo` the hour comes from the compressed clock instead.
    fn day_context(&self) -> (f64, f64) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let secs_into_day = (now as i64 + i64::from(self.offset_secs)).rem_euclid(86_400) as f64;
        let hour = self.demo_hour().unwrap_or(secs_into_day / 3600.0);
        (now - secs_into_day, hour)
    }

    fn local_hhmm(&self) -> String {
        if let Some(hour) = self.demo_hour() {
            let minutes = (hour * 60.0) as u32;
            return format!("{:02}:{:02}", minutes / 60, minutes % 60);
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let day_secs = (now + i64::from(self.offset_secs)).rem_euclid(86_400);
        format!("{:02}:{:02}", day_secs / 3600, (day_secs % 3600) / 60)
    }

    /// The compressed clock: noon at launch, a whole day every
    /// [`DEMO_DAY_SECONDS`], so a recording is the same run every time.
    fn demo_hour(&self) -> Option<f64> {
        self.demo
            .map(|start| (12.0 + start.elapsed().as_secs_f64() / DEMO_DAY_SECONDS * 24.0) % 24.0)
    }

    /// Rewrites the polled snapshot with the demo clock's sun (#30): the
    /// temperature and elevation the daemon would report at the demo hour.
    /// Without a daemon (or a location) a stand-in snapshot is synthesised,
    /// so the demo runs on a machine that has never seen the daemon.
    fn apply_demo(&mut self) {
        let Some(hour) = self.demo_hour() else {
            return;
        };
        // The demo has no daemon to ask, and the reel should show the
        // default, not a dash.
        self.fade = Some(true);
        self.band = Band::default();
        self.band_known = true;
        let mut status = self
            .status
            .take()
            .filter(|s| s.has_location)
            .unwrap_or_else(demo_status);
        let (midnight, _) = self.day_context();
        let elevation =
            solar_elevation(status.latitude, status.longitude, midnight + hour * 3600.0);
        status.elevation = elevation;
        status.temperature = target_temperature(
            elevation,
            Band::default(),
            status.day_temp,
            status.night_temp,
        );
        self.status = Some(status);
    }

    /// Feeds the scripted tour (#30): each due keypress goes through the real
    /// key handler, so the demo can only do what a hand on the keyboard could
    /// — and the chip shows the viewer the cause of every change.
    fn run_demo_script(&mut self) {
        let Some(start) = self.demo else {
            return;
        };
        let elapsed = start.elapsed().as_secs_f64();
        loop {
            let lap = (self.demo_cursor / DEMO_SCRIPT.len()) as f64;
            let (at, code, label) = DEMO_SCRIPT[self.demo_cursor % DEMO_SCRIPT.len()];
            if elapsed < lap * DEMO_DAY_SECONDS + at {
                break;
            }
            let _ = self.handle_key(code, KeyModifiers::NONE);
            self.demo_key = Some((label, Instant::now()));
            self.demo_cursor += 1;
        }
    }

    /// The fallback for small terminals: no wordmark, no cards — just the
    /// status lines, the curve, and the keys.
    fn draw_compact(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let outer = card(" nightlightd ", pal).padding(Padding::new(1, 1, 0, 0));
        let inner = outer.inner(area);
        frame.render_widget(outer, area);
        let [header, chart, footer] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .areas(inner);
        frame.render_widget(Paragraph::new(self.compact_header(pal)), header);
        self.draw_chart(frame, chart, pal);
        self.draw_footer(frame, footer, pal);
    }

    fn compact_header(&self, pal: &Palette) -> Vec<Line<'_>> {
        match &self.status {
            Some(status) => {
                let onoff = if status.enabled { "on" } else { "off" };
                vec![
                    Line::from(format!(
                        " {} · {} K · {}",
                        onoff, status.temperature, status.source
                    )),
                    Line::from(Span::styled(
                        format!(
                            " sun {:+.1}° ({}) · day {} K / night {} K",
                            status.elevation,
                            phase(status.elevation, self.shown_band()),
                            status.day_temp,
                            status.night_temp,
                        ),
                        Style::default().fg(pal.muted),
                    )),
                ]
            }
            None => vec![Line::from(Span::styled(
                " daemon not running",
                Style::default().fg(pal.err),
            ))],
        }
    }

    /// Left card: state badges and the big temperature readout. The number
    /// always wears the screen's own tint (semantic, theme-independent):
    /// white at 6500 K, candle-orange when warm, muted when off.
    fn draw_now_card(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let block = card(" now ", pal).padding(Padding::new(2, 1, 1, 0));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let Some(status) = &self.status else {
            let message = if self.mismatch {
                "update needed\ndashboard and daemon differ\ninstall matching versions\npress r to restart the daemon"
            } else {
                "daemon not running\npress r to start it"
            };
            frame.render_widget(
                Paragraph::new(message)
                    .style(Style::default().fg(pal.err))
                    .wrap(Wrap { trim: true }),
                inner,
            );
            return;
        };

        let [badges, big] =
            Layout::vertical([Constraint::Length(2), Constraint::Min(4)]).areas(inner);

        let (dot, dot_colour) = if status.enabled {
            ("●", pal.ok)
        } else {
            ("○", pal.err)
        };
        let mode = if !status.enabled {
            "OFF"
        } else if status.following {
            "AUTO"
        } else {
            "MANUAL"
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(format!(" {dot} "), Style::default().fg(dot_colour)),
                Span::styled(
                    if status.enabled { "ON" } else { "OFF" },
                    Style::default().fg(dot_colour).bold(),
                ),
                Span::styled("  ·  ", Style::default().fg(pal.faint)),
                Span::styled(mode, Style::default().fg(pal.accent).bold()),
                // The dim badge appears only while the screen is actually
                // dimmed, so the common daytime line stays as it was.
                if status.enabled && (status.brightness - 1.0).abs() > 1e-9 {
                    Span::styled(
                        format!("  ·  {:.0}%", status.brightness * 100.0),
                        Style::default().fg(pal.muted),
                    )
                } else {
                    Span::raw("")
                },
            ])),
            badges,
        );

        let tint = if status.enabled {
            let (r, g, b) = temperature_to_rgb(status.temperature);
            Color::Rgb(
                (r * 255.0).round() as u8,
                (g * 255.0).round() as u8,
                (b * 255.0).round() as u8,
            )
        } else {
            pal.muted
        };
        frame.render_widget(
            BigText::builder()
                .pixel_size(PixelSize::Quadrant)
                .style(Style::default().fg(tint))
                .centered()
                .lines(vec![Line::from(format!("{}K", status.temperature))])
                .build(),
            big,
        );
    }

    /// Right card: where the sun is, where we are, and the temperature band,
    /// with a sky scene for the current phase on the right.
    fn draw_sun_card(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let block = card(" sun ", pal).padding(Padding::new(2, 1, 1, 0));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let Some(status) = self.status.as_ref().filter(|s| s.has_location) else {
            frame.render_widget(
                Paragraph::new("no location resolved").style(Style::default().fg(pal.muted)),
                inner,
            );
            return;
        };

        // One story: the headline says what is coming ("sunrise in 5h 05m"),
        // the timeline bar under it shows the period's endpoints and where
        // "now" sits between them, and the temperature band closes the card.
        // Each piece steps back gracefully as the card narrows: first the sky
        // scene goes, then the endpoint times, then the long wordings.
        let art_width = if inner.width >= 40 { 16 } else { 0 };
        let [left, art] =
            Layout::horizontal([Constraint::Min(22), Constraint::Length(art_width)]).areas(inner);
        let [headline_row, _, bar_row, _, band_row] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(left);

        let phase = phase(status.elevation, self.shown_band());
        let (daylight_ratio, daylight_short, daylight_label, endpoints) = self.daylight(status);
        // The long label leads with its own icon; give the icon the accent
        // and the words the text tone. Narrow cards get the short wording.
        let mut label_chars = daylight_label.chars();
        let label_icon: String = label_chars.by_ref().take(1).collect();
        let label_rest: String = if usize::from(left.width) >= daylight_label.chars().count() {
            label_chars.collect()
        } else {
            format!(" {daylight_short}")
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(label_icon, Style::default().fg(pal.accent)),
                Span::styled(label_rest, Style::default().fg(pal.text).bold()),
            ])),
            headline_row,
        );
        // The timeline bar, bracketed by the clock times it runs between; the
        // fill colour is warm across the day, cool across the night.
        let towards_sunrise = daylight_short.starts_with("in ") || daylight_short == "polar night";
        let bar_colour = if towards_sunrise {
            pal.accent2
        } else {
            pal.accent
        };
        match endpoints {
            Some((starts, ends)) if bar_row.width >= 20 => {
                let [start_col, bar_col, end_col] = Layout::horizontal([
                    Constraint::Length(6),
                    Constraint::Min(4),
                    Constraint::Length(6),
                ])
                .areas(bar_row);
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        starts,
                        Style::default().fg(pal.muted),
                    ))),
                    start_col,
                );
                frame.render_widget(
                    BrailleBar::new(daylight_ratio, 1.0).fill_color(bar_colour),
                    Rect {
                        x: bar_col.x,
                        y: bar_col.y,
                        width: bar_col.width.saturating_sub(1),
                        height: bar_col.height,
                    },
                );
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        ends,
                        Style::default().fg(pal.muted),
                    )))
                    .alignment(Alignment::Right),
                    end_col,
                );
            }
            // Polar day or night (no endpoints), or a card too narrow to
            // bracket the bar with times: the bar runs the full width.
            _ => {
                frame.render_widget(
                    BrailleBar::new(daylight_ratio, 1.0).fill_color(bar_colour),
                    Rect {
                        width: bar_row.width.saturating_sub(2),
                        ..bar_row
                    },
                );
            }
        }
        // The temperature band, folding to "6500 → 3400 K" when the long
        // wording would clip.
        let band = if band_row.width >= 26 {
            Line::from(vec![
                Span::styled("day ", Style::default().fg(pal.muted)),
                Span::styled(
                    format!("{} K", status.day_temp),
                    Style::default().fg(pal.accent2),
                ),
                Span::styled(" · night ", Style::default().fg(pal.muted)),
                Span::styled(
                    format!("{} K", status.night_temp),
                    Style::default().fg(pal.accent2),
                ),
            ])
        } else {
            Line::from(vec![
                Span::styled(
                    format!("{}", status.day_temp),
                    Style::default().fg(pal.accent2),
                ),
                Span::styled(" → ", Style::default().fg(pal.muted)),
                Span::styled(
                    format!("{} K", status.night_temp),
                    Style::default().fg(pal.accent2),
                ),
            ])
        };
        frame.render_widget(Paragraph::new(band), band_row);
        if art_width > 0 {
            frame.render_widget(sky_art(phase, pal), art);
        }
    }

    /// The sun-card bar data: how far the current period has run (0..1), a
    /// short label, a long label, and the period's endpoints as clock times.
    /// The bar is a timeline — by day it runs sunrise → sunset, by night
    /// sunset → sunrise, the fill marking where "now" sits between the two.
    /// The honest polar cases carry no endpoints.
    #[allow(clippy::type_complexity)]
    fn daylight(&self, status: &Status) -> (f64, String, String, Option<(String, String)>) {
        let (midnight, now) = self.day_context();
        let events = milestones(
            status.latitude,
            status.longitude,
            midnight,
            self.shown_band(),
            status.day_temp,
            status.night_temp,
        );
        let find = |name: &str| events.iter().find(|e| e.name == name);
        match (find("sunrise"), find("sunset")) {
            (Some(rise), Some(set)) if now >= rise.hour && now < set.hour => {
                let left = set.hour - now;
                let ratio = ((now - rise.hour) / (set.hour - rise.hour)).clamp(0.0, 1.0);
                (
                    ratio,
                    hm(left),
                    format!("☀ {} of daylight left", hm(left)),
                    Some((rise.hhmm(), set.hhmm())),
                )
            }
            (Some(rise), set) if now < rise.hour => {
                // Pre-dawn: the night began at (roughly) yesterday's sunset.
                let until = rise.hour - now;
                let ratio = set.map_or(0.0, |set| {
                    let began = set.hour - 24.0;
                    ((now - began) / (rise.hour - began)).clamp(0.0, 1.0)
                });
                (
                    ratio,
                    format!("in {}", hm(until)),
                    format!("☾ sunrise in {}", hm(until)),
                    set.map(|set| (set.hhmm(), rise.hhmm())),
                )
            }
            (Some(rise), Some(set)) => {
                // After sunset: the next sunrise is tomorrow's, ~24 h on.
                let next = rise.hour + 24.0;
                let until = next - now;
                let ratio = ((now - set.hour) / (next - set.hour)).clamp(0.0, 1.0);
                (
                    ratio,
                    format!("in {}", hm(until)),
                    format!("☾ sunrise in {}", hm(until)),
                    Some((set.hhmm(), rise.hhmm())),
                )
            }
            _ => {
                // No crossing today: polar day or polar night.
                if status.elevation > 0.0 {
                    (
                        1.0,
                        "polar day".into(),
                        "☀ midnight sun · no sunset today".into(),
                        None,
                    )
                } else {
                    (
                        0.0,
                        "polar night".into(),
                        "☾ polar night · no sunrise today".into(),
                        None,
                    )
                }
            }
        }
    }

    fn draw_curve_card(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let block = card(" curve ", pal).padding(Padding::new(1, 1, 0, 0));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.draw_chart(frame, inner, pal);
    }

    /// The day/night curve, read the f.lux way (#35): a step in box
    /// characters — each column sits at the row its kelvin maps to, corners
    /// and a vertical run joining neighbours that disagree — over a pair of
    /// faint crossing sun-arcs, a per-hour tint strip along the floor carrying
    /// what f.lux's coloured fill carries, a sunlight-hours caption, and a dot
    /// riding the line at "now". With the default band the transition spans a
    /// column or two and this collapses to the square wave it always was; a
    /// widened band (#39) stretches it into a staircase, so the dot stays on
    /// the line instead of floating between the levels. Falls back to a hint
    /// without a location.
    fn draw_chart(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let Some(status) = self.status.as_ref().filter(|s| s.has_location) else {
            frame.render_widget(
                Paragraph::new("no location — the curve needs one")
                    .style(Style::default().fg(pal.muted)),
                area,
            );
            return;
        };

        // Whether this schedule is what the screen is following (#52). A
        // manual hold and a filter switched off both leave it true as a
        // schedule and false as a picture: it is what "automatic" returns
        // to, so it stays drawn, but it stops answering "what colour is my
        // screen" and everything below that reads as an answer is demoted.
        let out_of_force = out_of_force(status.enabled, status.following, status.temperature);
        let in_force = out_of_force.is_none();

        let (midnight, now_hour) = self.day_context();
        let elev_at = |hour: f64| -> f64 {
            solar_elevation(status.latitude, status.longitude, midnight + hour * 3600.0)
        };
        let kelvin_at = |hour: f64| -> f64 {
            f64::from(target_temperature(
                elev_at(hour),
                self.shown_band(),
                status.day_temp,
                status.night_temp,
            ))
        };
        // The one colour the screen is actually wearing, when it is not
        // wearing the curve's.
        let held_tint = theme::display_tint(status.temperature);

        // Hand-drawn axes as before: a left gutter for the kelvin labels, a
        // bottom row for the hours (ratatui's built-in x labels sit off by
        // one — they divide by the label count, not the gaps).
        let [top, x_row] =
            Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
        let [y_col, plot] =
            Layout::horizontal([Constraint::Length(7), Constraint::Min(10)]).areas(top);
        let w = plot.width as usize;
        if w < 8 || plot.height < 3 {
            return;
        }
        let strip_y = plot.y + plot.height - 1;
        let night_row = strip_y - 1;
        let day_row = plot.y;

        // Layer 1: two crossing sun-arcs behind everything, barely there —
        // f.lux's backdrop. Skipped when the card is too short to breathe.
        if plot.height >= 5 {
            let wave_rect = Rect {
                height: plot.height - 1,
                ..plot
            };
            let samples = 2 * w;
            let arc: Vec<(f64, f64)> = (0..=samples)
                .map(|i| {
                    let hour = i as f64 / samples as f64 * 24.0;
                    (hour, elev_at(hour))
                })
                .collect();
            let (emin, emax) = arc.iter().fold((f64::MAX, f64::MIN), |(lo, hi), &(_, e)| {
                (lo.min(e), hi.max(e))
            });
            let mirror: Vec<(f64, f64)> = arc.iter().map(|&(h, e)| (h, emin + emax - e)).collect();
            let chart = Chart::new(vec![
                Dataset::default()
                    .marker(Marker::Braille)
                    .graph_type(GraphType::Line)
                    .style(Style::default().fg(pal.faint))
                    .data(&arc),
                Dataset::default()
                    .marker(Marker::Braille)
                    .graph_type(GraphType::Line)
                    .style(Style::default().fg(pal.faint))
                    .data(&mirror),
            ])
            // Paint the plot ourselves so it doesn't fall back to the
            // terminal's grey; the card surface is the canvas.
            .style(Style::default().bg(pal.surface))
            .x_axis(Axis::default().bounds([0.0, 24.0]))
            .y_axis(Axis::default().bounds([emin - 2.0, emax + 2.0]));
            frame.render_widget(chart, wave_rect);
        }

        // Widget passes before the buffer work: kelvin labels in the gutter,
        // hours along the bottom.
        let mut y_label = |kelvin: u32, y: u16| {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    format!("{kelvin} K"),
                    Style::default().fg(pal.muted),
                )))
                .alignment(Alignment::Right),
                Rect {
                    x: y_col.x,
                    y,
                    width: y_col.width.saturating_sub(1),
                    height: 1,
                },
            );
        };
        y_label(status.day_temp, day_row);
        y_label(status.night_temp, night_row);
        hour_labels(frame, plot, x_row.y, pal);

        // Layer 2: the step, following the real curve. Each column sits at
        // the row its kelvin maps to, wearing that kelvin's colour; corners
        // and a vertical run join neighbours that disagree. A narrow band
        // drops every row in one column — f.lux's square wave — and a wide
        // one (#39) descends a row or two at a time, a staircase.
        let day = f64::from(status.day_temp);
        let span = f64::from(status.day_temp.saturating_sub(status.night_temp)).max(1.0);
        let row_of = |kelvin: f64| -> u16 {
            let frac = ((day - kelvin) / span).clamp(0.0, 1.0);
            day_row + (frac * f64::from(night_row - day_row)).round() as u16
        };
        let columns: Vec<(u16, f64)> = (0..w)
            .map(|x| {
                let kelvin = kelvin_at((x as f64 + 0.5) / w as f64 * 24.0);
                (row_of(kelvin), kelvin)
            })
            .collect();
        let buf = frame.buffer_mut();
        // Out of force the step gives up its per-hour tint for flat chrome:
        // those colours are a claim about the screen, and the claim is false.
        let step_tint = |kelvin: f64| {
            if in_force {
                theme::display_tint(kelvin.round() as u32)
            } else {
                pal.muted
            }
        };
        for (x, &(row, kelvin)) in columns.iter().enumerate() {
            if let Some(cell) = buf.cell_mut(Position::new(plot.x + x as u16, row)) {
                cell.set_symbol("─").set_fg(step_tint(kelvin));
            }
        }
        for x in 0..w.saturating_sub(1) {
            let (from, before) = columns[x];
            let (to, after) = columns[x + 1];
            if from == to {
                continue;
            }
            let col = plot.x + (x + 1) as u16;
            // The join wears the blend of its two ends; on a full flip that
            // is the old mid tint, on a staircase step a local gradient.
            let colour = step_tint((before + after) / 2.0);
            let (top_sym, bottom_sym) = if to > from {
                ("╮", "╰")
            } else {
                ("╭", "╯")
            };
            let (top, bottom) = (from.min(to), from.max(to));
            if let Some(cell) = buf.cell_mut(Position::new(col, top)) {
                cell.set_symbol(top_sym).set_fg(colour);
            }
            if let Some(cell) = buf.cell_mut(Position::new(col, bottom)) {
                cell.set_symbol(bottom_sym).set_fg(colour);
            }
            for row in top + 1..bottom {
                if let Some(cell) = buf.cell_mut(Position::new(col, row)) {
                    cell.set_symbol("│").set_fg(colour);
                }
            }
        }

        // Layer 3: the tint strip — every column wears the colour the screen
        // will be at that hour. This is f.lux's fill, one row tall.
        for x in 0..w {
            // Held, the screen is this colour at every hour of the day, so
            // the strip says so — one flat band instead of a sunset.
            let tint = if in_force {
                let kelvin = kelvin_at((x as f64 + 0.5) / w as f64 * 24.0).round() as u32;
                theme::display_tint(kelvin)
            } else {
                held_tint
            };
            if let Some(cell) = buf.cell_mut(Position::new(plot.x + x as u16, strip_y)) {
                cell.set_symbol("▂").set_fg(tint);
            }
        }

        // Layer 4: the sunlight caption on the plateau's floor, skipped when
        // the plateau is too narrow to hold it.
        let daylight = (0..=960)
            .filter(|i| elev_at(f64::from(*i) * 0.025) > 0.0)
            .count() as f64
            * 0.025;
        let caption = format!("{daylight:.0}h sunlight");
        let day_cols: Vec<usize> = (0..w).filter(|&x| columns[x].0 == day_row).collect();
        if let (Some(&first), Some(&last)) = (day_cols.first(), day_cols.last()) {
            let run = last - first + 1;
            if run > caption.len() + 4 {
                let start = plot.x + (first + (run - caption.len()) / 2) as u16;
                for (offset, glyph) in caption.chars().enumerate() {
                    if let Some(cell) =
                        buf.cell_mut(Position::new(start + offset as u16, night_row))
                    {
                        cell.set_symbol(&glyph.to_string()).set_fg(pal.faint);
                    }
                }
            }
        }

        // Layer 4b (#52): the line the screen is actually on, when that is
        // not the curve. Flat across the whole day, because a held
        // temperature has no schedule — having none is the point of holding
        // it. Heavier than the step it crosses, in the colour the screen
        // really is, and worded at its left end so it cannot be read as a
        // third bound. Its row comes from the same `row_of` as everything
        // else, so a held temperature outside the day/night span lands on
        // the nearest edge; the words carry the true number.
        let held_row = (!in_force).then(|| row_of(f64::from(status.temperature)));
        if let Some(row) = held_row {
            for x in 0..w {
                if let Some(cell) = buf.cell_mut(Position::new(plot.x + x as u16, row)) {
                    cell.set_symbol("━").set_fg(held_tint);
                }
            }
            let label = format!(" {} ", out_of_force.unwrap_or_default());
            if label.chars().count() + 2 < w {
                for (offset, glyph) in label.chars().enumerate() {
                    let col = plot.x + 1 + offset as u16;
                    if let Some(cell) = buf.cell_mut(Position::new(col, row)) {
                        cell.set_symbol(&glyph.to_string()).set_fg(held_tint);
                    }
                }
            }
        }

        // Layer 5: now — a dot riding the line, f.lux's sun bead. The row is
        // looked up from the drawn line, not recomputed: on a staircase the
        // slope runs rows per column, so any independent rounding of "now"
        // parks the dot beside the line instead of on it. Out of force it
        // rides the flat line instead: the dot marks where the screen is.
        let now_x = (now_hour / 24.0 * w as f64 - 0.5)
            .round()
            .clamp(0.0, (w - 1) as f64) as usize;
        let now_col = plot.x + now_x as u16;
        let now_row = held_row.unwrap_or(columns[now_x].0);
        if let Some(cell) = buf.cell_mut(Position::new(now_col, now_row)) {
            cell.set_symbol("●").set_fg(pal.text);
        }
    }

    /// The sun's arc for the today tab (#35): the day's elevation across the
    /// hours — the smooth thing the schedule is derived from — drawn with the
    /// now tab's hand (#45), because two pictures of the same day in two
    /// different styles read as two unrelated widgets. Same box-drawn step,
    /// same per-hour tint, same floor strip, same bead at now; only the
    /// quantity differs. The now tab's curve shows the schedule; this shows
    /// why, and a dashed horizon rule marks where the sun crosses it.
    fn draw_sun_arc(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let Some(status) = self.status.as_ref().filter(|s| s.has_location) else {
            return;
        };
        let (midnight, now_hour) = self.day_context();
        let elev_at = |hour: f64| -> f64 {
            solar_elevation(status.latitude, status.longitude, midnight + hour * 3600.0)
        };
        let tint_at = |hour: f64| -> Color {
            let kelvin = target_temperature(
                elev_at(hour),
                self.shown_band(),
                status.day_temp,
                status.night_temp,
            );
            theme::display_tint(kelvin)
        };

        let [top, x_row] =
            Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
        let [y_col, plot] =
            Layout::horizontal([Constraint::Length(7), Constraint::Min(10)]).areas(top);
        let w = plot.width as usize;
        if w < 8 || plot.height < 3 {
            return;
        }
        // The floor carries the tint strip, exactly as it does on the now
        // tab; the arc itself lives in the rows above it.
        let strip_y = plot.y + plot.height - 1;
        let (top_row, bottom_row) = (plot.y, strip_y - 1);

        // Padded bounds so the peak and the trough never touch the edges.
        let hour_at = |x: usize| (x as f64 + 0.5) / w as f64 * 24.0;
        let elevations: Vec<f64> = (0..w).map(|x| elev_at(hour_at(x))).collect();
        let (emin, emax) = elevations
            .iter()
            .fold((f64::MAX, f64::MIN), |(lo, hi), &e| (lo.min(e), hi.max(e)));
        let (lo, hi) = (emin - 3.0, emax + 3.0);
        let rows = f64::from(bottom_row - top_row);
        let row_of = |elevation: f64| -> u16 {
            let frac = ((elevation - lo) / (hi - lo)).clamp(0.0, 1.0);
            bottom_row - (frac * rows).round() as u16
        };
        let columns: Vec<u16> = elevations.iter().map(|&e| row_of(e)).collect();

        // The horizon first, so the arc is drawn over it rather than under.
        let horizon_row = (emin < 0.0 && emax > 0.0).then(|| row_of(0.0));
        if let Some(row) = horizon_row {
            let buf = frame.buffer_mut();
            for x in (0..w).step_by(2) {
                if let Some(cell) = buf.cell_mut(Position::new(plot.x + x as u16, row)) {
                    cell.set_symbol("·").set_fg(pal.faint);
                }
            }
            let word = "horizon";
            if w > word.len() + 4 {
                let start = plot.x + (w - word.len() - 1) as u16;
                for (offset, glyph) in word.chars().enumerate() {
                    if let Some(cell) = buf.cell_mut(Position::new(start + offset as u16, row)) {
                        cell.set_symbol(&glyph.to_string()).set_fg(pal.muted);
                    }
                }
            }
        }

        // The arc as a step: one cell per column at the row its elevation
        // maps to, wearing the colour the screen will be at that hour, then
        // corners and a vertical run wherever neighbours disagree.
        let buf = frame.buffer_mut();
        for (x, &row) in columns.iter().enumerate() {
            if let Some(cell) = buf.cell_mut(Position::new(plot.x + x as u16, row)) {
                cell.set_symbol("─").set_fg(tint_at(hour_at(x)));
            }
        }
        for x in 0..w.saturating_sub(1) {
            let (from, to) = (columns[x], columns[x + 1]);
            if from == to {
                continue;
            }
            let col = plot.x + (x + 1) as u16;
            let colour = tint_at((hour_at(x) + hour_at(x + 1)) / 2.0);
            let (top_sym, bottom_sym) = if to > from {
                ("╮", "╰")
            } else {
                ("╭", "╯")
            };
            let (high, low) = (from.min(to), from.max(to));
            if let Some(cell) = buf.cell_mut(Position::new(col, high)) {
                cell.set_symbol(top_sym).set_fg(colour);
            }
            if let Some(cell) = buf.cell_mut(Position::new(col, low)) {
                cell.set_symbol(bottom_sym).set_fg(colour);
            }
            for row in high + 1..low {
                if let Some(cell) = buf.cell_mut(Position::new(col, row)) {
                    cell.set_symbol("│").set_fg(colour);
                }
            }
        }

        // The floor strip: every column wears the hour's screen colour.
        for x in 0..w {
            if let Some(cell) = buf.cell_mut(Position::new(plot.x + x as u16, strip_y)) {
                cell.set_symbol("▂").set_fg(tint_at(hour_at(x)));
            }
        }

        // Now, riding the arc — the row comes from the drawn line, never
        // recomputed, so the bead cannot land beside it.
        let now_x = (now_hour / 24.0 * w as f64 - 0.5)
            .round()
            .clamp(0.0, (w - 1) as f64) as usize;
        if let Some(cell) = buf.cell_mut(Position::new(plot.x + now_x as u16, columns[now_x])) {
            cell.set_symbol("●").set_fg(pal.text);
        }

        // Degree labels in the gutter; the horizon's only when the rule is
        // actually on the plot (a polar day or night has no horizon).
        let mut y_label = |text: String, y: u16| {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    text,
                    Style::default().fg(pal.muted),
                )))
                .alignment(Alignment::Right),
                Rect {
                    x: y_col.x,
                    y,
                    width: y_col.width.saturating_sub(1),
                    height: 1,
                },
            );
        };
        y_label(format!("{emax:+.0}°"), top_row);
        y_label(format!("{emin:+.0}°"), bottom_row);
        if let Some(row) = horizon_row
            && row > top_row
            && row < bottom_row
        {
            y_label("0°".into(), row);
        }
        hour_labels(frame, plot, x_row.y, pal);
    }
}

/// An even hour every two hours on the axis row, each centred on the column
/// the plot maps that hour to — the same mapping the data uses, so ticks,
/// step and dot agree.
fn hour_labels(frame: &mut Frame<'_>, plot: Rect, y: u16, pal: &Palette) {
    let width = plot.width as usize;
    if width < 8 {
        return;
    }
    let mut cells = vec![' '; width];
    for hour in (0..=24).step_by(2) {
        let col = (f64::from(hour) / 24.0 * (width - 1) as f64).round() as usize;
        let text = format!("{hour:02}");
        let start = col.saturating_sub(1).min(width - text.len());
        for (offset, glyph) in text.chars().enumerate() {
            cells[start + offset] = glyph;
        }
    }
    let line: String = cells.into_iter().collect();
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            line,
            Style::default().fg(pal.muted),
        ))),
        Rect {
            x: plot.x,
            y,
            width: plot.width,
            height: 1,
        },
    );
}

/// A little sky scene for the sun card: a rayed sun by day, a sun sinking to
/// the horizon through the transition, a shaded moon under stars at night.
///
/// The half-block shaded-disc idiom — a lit body (`█`), a shaded terminator
/// (`▒░`), stars (`✦ · ✧`) scattered around it — is the visual language
/// SunReactor uses for its weather orb (GPL-3.0, github.com/arcanorca). These
/// discs are drawn fresh here, not copied: same technique, our own shapes.
///
/// Each glyph is coloured by what it is, so a scene reads in two tones like the
/// rest of the theme: the body in one hue, the shade faint, the stars and rays
/// in the other hue.
fn sky_art(phase: &str, pal: &Palette) -> Paragraph<'static> {
    // (art, body colour, star/ray colour). The shaded side is always faint.
    let (art, body, glow): (&[&str], Color, Color) = match phase {
        "day" => (
            &[
                r"   \   |   /   ",
                r"     ▄███▄     ",
                r"  ─ ███████ ─  ",
                r"     ▀███▀     ",
                r"   /   |   \   ",
            ],
            pal.accent,
            pal.muted,
        ),
        "night" => (
            &[
                r" ✦    ▄▄▄▄     ",
                r"    ▄██████▒   ",
                r"   ███████▒▒  ·",
                r"    ▀█████▒▒   ",
                r" ·    ▀▀▀▀   ✦ ",
            ],
            pal.accent2,
            pal.accent,
        ),
        _ => (
            &[
                r"       |       ",
                r"     ▄███▄     ",
                r"  ─ ███████ ─  ",
                r"  ▁▁▁▀█▀▁▁▁▁▁  ",
                r"      ░▒░      ",
            ],
            pal.accent,
            pal.muted,
        ),
    };
    let lines: Vec<Line<'static>> = art
        .iter()
        .map(|row| {
            Line::from(
                row.chars()
                    .map(|glyph| {
                        let colour = match glyph {
                            '█' | '▄' | '▀' => body,
                            '▒' | '░' => pal.faint,
                            '✦' | '✧' | '·' => glow,
                            _ => pal.muted,
                        };
                        Span::styled(glyph.to_string(), Style::default().fg(colour))
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    Paragraph::new(lines)
}

/// A short "3h 20m" / "45m" span from a duration in hours, for the daylight
/// label. Negative or sub-minute spans read as "0m".
fn hm(hours: f64) -> String {
    let minutes = (hours * 60.0).round().max(0.0) as i64;
    let (h, m) = (minutes / 60, minutes % 60);
    if h > 0 {
        format!("{h}h {m:02}m")
    } else {
        format!("{m}m")
    }
}

/// A borderless card: a bold accent title floating over the content, and a
/// surface one shade lighter than the page underneath it — raised panels
/// instead of boxes; the elevation does what frames used to do.
fn card<'a>(title: &'a str, pal: &Palette) -> Block<'a> {
    Block::new()
        .style(Style::default().bg(pal.surface))
        .title(Span::styled(title, Style::default().fg(pal.accent).bold()))
}

/// Where the daemon's config lives, for the settings tab's info line — the
/// same XDG derivation the daemon uses.
fn config_path_display() -> String {
    nightlightd_core::paths::config_file("config.toml")
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "~/.config/nightlightd/config.toml".into())
}

/// The theme this dashboard last wore, or `live` when it has never been
/// asked. Anything unreadable, empty, or naming a theme that no longer exists
/// lands on the default without complaint — the file is a convenience, and a
/// night light must not fail to start because somebody typed into it.
fn remembered_theme() -> usize {
    nightlightd_core::paths::config_file(THEME_FILE)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|name| theme::index_of(name.trim()))
        .unwrap_or(0)
}

/// Remembers the choice by name rather than by index — an index is one
/// release away from meaning a different theme. Failure is silence: the
/// colours still apply for this session, they just will not survive the
/// dashboard closing.
fn remember_theme(index: usize) {
    let Some(name) = THEMES.get(index).map(|theme| theme.name) else {
        return;
    };
    let Some(path) = nightlightd_core::paths::config_file(THEME_FILE) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, name);
}

/// "41.0°N 29.0°E" for a signed coordinate pair.
fn format_coords(latitude: f64, longitude: f64) -> String {
    format!(
        "{:.1}°{} {:.1}°{}",
        latitude.abs(),
        if latitude >= 0.0 { "N" } else { "S" },
        longitude.abs(),
        if longitude >= 0.0 { "E" } else { "W" },
    )
}

/// "in 2h 05m" / "3h 12m ago" / "now" for a signed hour delta.
fn relative(delta_hours: f64) -> String {
    let minutes = (delta_hours * 60.0).round() as i64;
    if minutes.abs() < 1 {
        return "now".into();
    }
    let (h, m) = (minutes.abs() / 60, minutes.abs() % 60);
    let span = if h > 0 {
        format!("{h}h {m:02}m")
    } else {
        format!("{m}m")
    };
    if minutes > 0 {
        format!("in {span}")
    } else {
        format!("{span} ago")
    }
}

/// The local clock's offset from UTC in seconds, read once from `date +%z`
/// (e.g. `+0300` → 10800). Zero on any failure — the curve then reads in UTC,
/// which is wrong by the offset but never crashes.
fn local_offset_seconds() -> i32 {
    let output = std::process::Command::new("date").arg("+%z").output();
    let text = output
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .unwrap_or_default();
    let text = text.trim();
    if text.len() < 5 {
        return 0;
    }
    let sign = if text.starts_with('-') { -1 } else { 1 };
    let hours: i32 = text[1..3].parse().unwrap_or(0);
    let minutes: i32 = text[3..5].parse().unwrap_or(0);
    sign * (hours * 3600 + minutes * 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The chart speaks only when the screen is not following what it draws
    /// (#52), and it distinguishes the two ways that happens. The `off` case
    /// is the one the issue did not think of: a switched-off filter leaves
    /// the screen neutral while the curve goes on drawing a full day.
    #[test]
    fn the_chart_speaks_only_when_the_screen_is_not_following_it() {
        assert_eq!(out_of_force(true, true, 4200), None);
        assert_eq!(
            out_of_force(true, false, 2800).as_deref(),
            Some("held at 2800 K")
        );
        assert_eq!(
            out_of_force(false, true, 6500).as_deref(),
            Some("off · 6500 K")
        );
        // Off outranks a hold, the way every readout here already orders them.
        assert_eq!(
            out_of_force(false, false, 2800).as_deref(),
            Some("off · 2800 K")
        );
    }

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    /// A bound dragged on the panel's curve lands on an arbitrary elevation.
    /// The first arrow press tidies it onto the half-degree grid; every press
    /// after that is a clean step, in both directions and across zero.
    #[test]
    fn an_arrow_press_lands_on_the_half_degree_grid() {
        assert!(approx(nudge(-13.957_860_553_564_76, true), -13.5));
        assert!(approx(nudge(-13.957_860_553_564_76, false), -14.0));
        assert!(approx(nudge(-6.0, true), -5.5));
        assert!(approx(nudge(-6.0, false), -6.5));
        assert!(approx(nudge(0.0, true), 0.5));
        assert!(approx(nudge(-0.25, true), 0.0));
        assert!(approx(nudge(3.0, false), 2.5));
    }

    /// The pair may never cross. Walking the night bound up runs it into the
    /// day bound and stops half a degree short, however long you lean on the
    /// key; the day bound is stopped the same way from below.
    #[test]
    fn the_bounds_stop_before_they_meet() {
        let mut band = Band::default();
        for _ in 0..40 {
            band = nudged_band(band, false, true);
        }
        assert!(approx(
            band.night_elevation,
            band.day_elevation - MIN_BAND_WIDTH
        ));
        assert!(
            band.day_elevation > band.night_elevation,
            "the pair crossed"
        );

        let mut band = Band::default();
        for _ in 0..40 {
            band = nudged_band(band, true, false);
        }
        assert!(approx(
            band.day_elevation,
            band.night_elevation + MIN_BAND_WIDTH
        ));
        assert!(band.sane() == band, "a pinched band must still be sane");
    }

    /// Neither bound leaves the window its rail draws, so the reading and
    /// the rail can never describe different values.
    #[test]
    fn the_bounds_stay_inside_the_drawn_window() {
        let mut band = Band::default();
        for _ in 0..80 {
            band = nudged_band(band, true, true);
            band = nudged_band(band, false, false);
        }
        assert!(approx(band.day_elevation, BAND_UI_MAX));
        assert!(approx(band.night_elevation, BAND_UI_MIN));
    }

    /// The rail's scale is what the arrow keys can reach, so every value a
    /// key can produce lands somewhere readable on it — and, the point of
    /// the whole thing, only the very top of the range fills the bar. A
    /// scale that followed the day bound made that bar full at every day
    /// bound from 6500 K up, which is no bar at all.
    #[test]
    fn every_reachable_bound_is_somewhere_readable_on_the_rail() {
        let (min, max) = RAIL_RANGE;
        assert_eq!((min, max), (f64::from(NIGHT_MIN), f64::from(DAY_MAX)));
        let fill = |kelvin: u32| (f64::from(kelvin) - min) / (max - min);
        // The default day sits well inside, not against the end.
        assert!((0.55..0.65).contains(&fill(6500)), "{}", fill(6500));
        // Past neutral it keeps moving, which is the defect this replaced.
        assert!(fill(8000) > fill(6500));
        assert!(fill(10_000) > fill(8000));
        // Only the two ends saturate, and neither overflows.
        assert_eq!(fill(NIGHT_MIN), 0.0);
        assert_eq!(fill(DAY_MAX), 1.0);
        for kelvin in NIGHT_MIN..=DAY_MAX {
            assert!((0.0..=1.0).contains(&fill(kelvin)), "{kelvin} K fell off");
        }
    }

    /// What escape asks about: a draft that has not moved needs no question,
    /// and one that has must not be thrown away silently.
    #[test]
    fn an_untouched_draft_has_nothing_to_confirm() {
        let mut edit = BandEdit {
            original: Band::default(),
            draft: Band::default(),
            selected: 1,
            confirming: false,
        };
        assert!(!edit.touched());
        edit.draft = nudged_band(edit.draft, false, false);
        assert!(edit.touched());
        // Walked back onto its own starting value, it is untouched again —
        // the question is about difference, not about history.
        edit.draft = nudged_band(edit.draft, false, true);
        assert!(!edit.touched());
    }

    /// The road back (#48). Filling the draft with the default makes the
    /// editor offer apply, so returning is a change like any other — and
    /// pressing it on a band that is already the default changes nothing,
    /// so escape still leaves without a question.
    #[test]
    fn the_default_band_is_a_draft_like_any_other() {
        let pinched = Band {
            day_elevation: 3.0,
            night_elevation: 2.5,
        };
        let mut edit = BandEdit {
            original: pinched,
            draft: pinched,
            selected: 1,
            confirming: false,
        };
        edit.draft = Band::default();
        assert!(edit.touched(), "returning is a change worth confirming");

        let mut edit = BandEdit {
            original: Band::default(),
            draft: Band::default(),
            selected: 1,
            confirming: false,
        };
        edit.draft = Band::default();
        assert!(!edit.touched(), "already there is not a change");
    }

    /// Pressing one way then the other must return the value it started
    /// from, once it is on the grid — an arrow that drifts is a bug the eye
    /// only catches after a dozen presses.
    #[test]
    fn opposite_presses_cancel_on_the_grid() {
        let mut value = -6.0;
        for _ in 0..8 {
            value = nudge(value, false);
        }
        for _ in 0..8 {
            value = nudge(value, true);
        }
        assert!(approx(value, -6.0));
    }
}
