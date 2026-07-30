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
mod today;

use std::io;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nightlightd_core::color::temperature_to_rgb;
use nightlightd_core::location::nearest_zone;
use nightlightd_core::solar::solar_elevation;
use nightlightd_core::transition::target_temperature;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Map, MapResolution};
use ratatui::widgets::{
    Axis, Block, Cell, Chart, Clear, Dataset, GraphType, Padding, Paragraph, RatatuiLogo, Row,
    Table,
};
use ratatui::{DefaultTerminal, Frame};
use ratatui_braille_bar::BrailleBar;
use tui_big_text::{BigText, PixelSize};
use tui_slider::Slider;

use crate::daemon::{Client, Status};
use crate::theme::{Palette, THEMES};

/// Bounds and step for the night-temperature keys, mirroring the panel.
const NIGHT_MIN: u32 = 1500;
const NIGHT_STEP: u32 = 100;

/// One full day in the `--demo` compressed clock, in real seconds (#30).
const DEMO_DAY_SECONDS: f64 = 28.0;

/// The demo's scripted tour (#30): (second, key, chip label). Starts at noon
/// on the now tab; dwells through sunset while the interface warms, walks the
/// tabs, rolls `T` through every theme back to `live` (sunrise lands during
/// the roll, so the return to `live` opens on morning gold), then jumps home.
/// One pass is exactly one compressed day, so a recording loops seamlessly.
const DEMO_SCRIPT: &[(f64, KeyCode, &str)] = &[
    (11.0, KeyCode::Tab, "⇥"),
    (14.0, KeyCode::Tab, "⇥"),
    (16.5, KeyCode::Tab, "⇥"),
    (18.0, KeyCode::Tab, "⇥"),
    (19.0, KeyCode::Char('T'), "T"),
    (19.8, KeyCode::Char('T'), "T"),
    (20.6, KeyCode::Char('T'), "T"),
    (22.2, KeyCode::Char('T'), "T"),
    (22.8, KeyCode::Char('T'), "T"),
    (23.4, KeyCode::Char('T'), "T"),
    (24.0, KeyCode::Char('T'), "T"),
    (24.6, KeyCode::Char('T'), "T"),
    (25.6, KeyCode::Char('1'), "1"),
];

/// The tab bar, in order. Each holds real content or it does not exist.
const TABS: &[&str] = &["now", "today", "location", "outputs", "settings"];
const LOCATION_TAB: usize = 2;
/// The settings tab's index and its selectable rows: day, night, theme, login.
const SETTINGS_TAB: usize = 4;
const SETTINGS_ITEMS: usize = 4;

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
    last_poll: Option<Instant>,
    offset_secs: i32,
    theme_index: usize,
    tab: usize,
    settings_selected: usize,
    /// The theme picker popup: `Some(highlighted index)` while open.
    theme_popup: Option<usize>,
    /// The `?` overlay: every key in one place.
    help_popup: bool,
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
        last_poll: None,
        offset_secs: local_offset_seconds(),
        theme_index,
        tab,
        settings_selected: 0,
        theme_popup: None,
        help_popup: false,
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
fn parse_args() -> Result<(usize, usize, bool), String> {
    let theme_names = || {
        THEMES
            .iter()
            .map(|theme| theme.name)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let usage = || {
        format!(
            "usage: nightlight-tui [--theme <{}>] [--tab <{}>] [--demo]",
            theme_names(),
            TABS.join(", ")
        )
    };
    let (mut theme_index, mut tab, mut demo) = (0, 0, false);
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--theme" | "-t" => {
                let name = args.next().ok_or_else(usage)?;
                theme_index = theme::index_of(&name).ok_or_else(|| {
                    format!("unknown theme {name:?} — available: {}", theme_names())
                })?;
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
        if self.help_popup || self.sun_popup {
            if matches!(
                code,
                KeyCode::Esc
                    | KeyCode::Enter
                    | KeyCode::Char('q')
                    | KeyCode::Char('?')
                    | KeyCode::Char('s')
            ) {
                self.help_popup = false;
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
                self.theme_index = (self.theme_index + 1) % THEMES.len();
            }
            KeyCode::Char('?') => self.help_popup = true,
            KeyCode::Char('s') => self.sun_popup = true,
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
                    .clamp(NIGHT_MIN, status.day_temp);
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
                2 => self.theme_popup = Some(self.theme_index),
                3 => self.toggle_login(),
                _ => {}
            },
            _ => {}
        }
    }

    /// Left/right on the selected settings row: nudge the bounds (the daemon
    /// clamps and persists), cycle the theme, or flip the login toggle.
    fn adjust_setting(&mut self, increase: bool) {
        match self.settings_selected {
            0 => {
                if let Some(status) = &self.status {
                    let day = if increase {
                        status.day_temp.saturating_add(NIGHT_STEP)
                    } else {
                        status.day_temp.saturating_sub(NIGHT_STEP)
                    }
                    .clamp(status.night_temp, 6500);
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
                    .clamp(NIGHT_MIN, status.day_temp);
                    self.client.set_night_temp(night);
                    self.last_poll = None;
                }
            }
            2 => {
                let count = THEMES.len();
                self.theme_index = if increase {
                    (self.theme_index + 1) % count
                } else {
                    (self.theme_index + count - 1) % count
                };
            }
            3 => self.toggle_login(),
            _ => {}
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
                self.theme_index = selected;
                self.theme_popup = None;
            }
            KeyCode::Esc | KeyCode::Char('q') => self.theme_popup = None,
            _ => {}
        }
    }

    fn palette(&self) -> Palette {
        THEMES[self.theme_index].palette(self.status.as_ref().map(|s| s.temperature))
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
                    "per-output control is #34, planned for v0.2 — this is its home",
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

        // Credit where credit is due: the official logo, bottom-right, with
        // air between it and the card's edge.
        if inner.width > 40 && inner.height > 8 {
            let logo_area = Rect {
                x: inner.right().saturating_sub(18),
                y: inner.bottom().saturating_sub(3),
                width: 15,
                height: 2,
            };
            frame.render_widget(RatatuiLogo::tiny(), logo_area);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "powered by",
                    Style::default().fg(pal.faint),
                )))
                .alignment(Alignment::Right),
                Rect {
                    x: inner.x,
                    y: inner.bottom().saturating_sub(4),
                    width: inner.width.saturating_sub(3),
                    height: 1,
                },
            );
        }

        let value = |v: Option<String>| v.unwrap_or_else(|| "—".into());
        let day = value(self.status.as_ref().map(|s| format!("{} K", s.day_temp)));
        let night = value(self.status.as_ref().map(|s| format!("{} K", s.night_temp)));
        let day_k = self.status.as_ref().map(|s| s.day_temp);
        let night_k = self.status.as_ref().map(|s| s.night_temp);
        let rows: [(&str, String, &str, Option<u32>); SETTINGS_ITEMS] = [
            ("daytime", day, "‹ › adjust", day_k),
            ("nighttime", night, "‹ › adjust", night_k),
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

        let mut lines: Vec<Line<'_>> = Vec::new();
        let mut sliders: Vec<(u16, u32, bool)> = Vec::new();
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
            if let Some(kelvin) = slider {
                sliders.push((inner.y + lines.len() as u16, kelvin, selected));
                lines.push(Line::default());
            }
            lines.push(Line::default());
        }
        lines.push(Line::from(Span::styled(
            format!(" config  {}", config_path_display()),
            Style::default().fg(pal.faint),
        )));
        lines.push(Line::from(Span::styled(
            "         day & night changes persist there automatically",
            Style::default().fg(pal.faint),
        )));
        frame.render_widget(Paragraph::new(lines), inner);

        // The temperature rows carry a real slider (tui-slider) on the row
        // reserved beneath each — the segmented ▰▱ style, no thumb: filled to
        // where the bound sits in the shared NIGHT_MIN..=6500 K range, ticks in
        // the accent so they track the live tint; the empty ticks brighten when
        // their row is selected.
        for (y, kelvin, selected) in sliders {
            let width = inner.width.saturating_sub(6).min(28);
            if width < 2 {
                continue;
            }
            let rail = if selected { pal.muted } else { pal.faint };
            let slider = Slider::new(f64::from(kelvin), f64::from(NIGHT_MIN), 6500.0)
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

        let pairs: Vec<(&str, &str)> = if self.tab == SETTINGS_TAB {
            vec![
                ("⇥", "tab"),
                ("↑↓", "select"),
                ("‹›", "adjust"),
                ("⏎", "apply"),
                ("s", "sun"),
                ("?", "help"),
                ("q", "quit"),
            ]
        } else if self.tab == LOCATION_TAB {
            vec![
                ("⇥", "tab"),
                ("⏎", "pick"),
                ("m", "map"),
                ("c", "timezone"),
                ("s", "sun"),
                ("?", "help"),
                ("q", "quit"),
            ]
        } else {
            vec![
                ("⇥", "tab"),
                ("t", "toggle"),
                ("a", "auto"),
                ("↑↓", "night temp"),
                ("T", "theme"),
                ("s", "sun"),
                ("?", "help"),
                ("q", "quit"),
            ]
        };
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

    /// `?`: every key in one place, grouped by where it works.
    fn draw_help_popup(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let key = |k: &str, label: &str| {
            Line::from(vec![
                Span::styled(format!("{k:<8}"), Style::default().fg(pal.accent).bold()),
                Span::styled(label.to_string(), Style::default().fg(pal.muted)),
            ])
        };
        let section = |name: &str| {
            Line::from(Span::styled(
                name.to_string(),
                Style::default().fg(pal.accent2),
            ))
        };
        let mut lines = vec![Line::default(), Line::default(), section("everywhere")];
        lines.push(key("⇥ · 1-5", "switch tab"));
        lines.push(key("t", "toggle the filter"));
        lines.push(key("a", "back to automatic"));
        lines.push(key("↑↓", "nudge the night temperature"));
        lines.push(key("T", "cycle the theme"));
        lines.push(key("s", "sun details"));
        lines.push(key("?", "this help"));
        lines.push(key("q", "quit"));
        lines.push(Line::default());
        lines.push(section("location"));
        lines.push(key("⏎", "pick a spot · pin it"));
        lines.push(key("m", "the map, full size"));
        lines.push(key("c", "back to the timezone"));
        lines.push(Line::default());
        lines.push(section("settings"));
        lines.push(key("↑↓ ‹›", "select · adjust"));
        lines.push(key("⏎", "apply the row"));
        lines.push(Line::default());
        lines.push(Line::from(Span::styled(
            format!(
                "v{} · github.com/umutdinceryananer/nightlightd",
                env!("CARGO_PKG_VERSION")
            ),
            Style::default().fg(pal.faint),
        )));
        let inner = Self::overlay(frame, area, 52, lines.len() as u16 + 2, pal);
        Self::overlay_title(frame, inner, "keys", "esc", pal);
        frame.render_widget(Paragraph::new(lines), inner);
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
            today::milestones(
                status.latitude,
                status.longitude,
                midnight + offset_days * 86_400.0,
                status.day_temp,
                status.night_temp,
            )
        };
        let today_events = compute(0.0);
        let yesterday = compute(-1.0);
        let tomorrow = compute(1.0);
        let hour_of = |events: &[today::Milestone], name: &str| {
            events.iter().find(|e| e.name == name).map(|e| e.hour)
        };
        let hhmm_of = |events: &[today::Milestone], name: &str| {
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
            let phase = sun_phase(status.elevation);
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
        let events = today::milestones(
            status.latitude,
            status.longitude,
            midnight,
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
        let mut status = self
            .status
            .take()
            .filter(|s| s.has_location)
            .unwrap_or_else(demo_status);
        let (midnight, _) = self.day_context();
        let elevation =
            solar_elevation(status.latitude, status.longitude, midnight + hour * 3600.0);
        status.elevation = elevation;
        status.temperature = target_temperature(elevation, status.day_temp, status.night_temp);
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
                            sun_phase(status.elevation),
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
            frame.render_widget(
                Paragraph::new("daemon not running").style(Style::default().fg(pal.err)),
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

        let phase = sun_phase(status.elevation);
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
        let events = today::milestones(
            status.latitude,
            status.longitude,
            midnight,
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

    /// The day/night curve, read the f.lux way (#35): a square-wave step in
    /// box characters — the transition really is one or two columns wide at
    /// this scale, so it is drawn as the square wave it is — over a pair of
    /// faint crossing sun-arcs, a per-hour tint strip along the floor carrying
    /// what f.lux's coloured fill carries, a sunlight-hours caption, and a dot
    /// riding the line at "now". Falls back to a hint without a location.
    fn draw_chart(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let Some(status) = self.status.as_ref().filter(|s| s.has_location) else {
            frame.render_widget(
                Paragraph::new("no location — the curve needs one")
                    .style(Style::default().fg(pal.muted)),
                area,
            );
            return;
        };

        let (midnight, now_hour) = self.day_context();
        let elev_at = |hour: f64| -> f64 {
            solar_elevation(status.latitude, status.longitude, midnight + hour * 3600.0)
        };
        let kelvin_at = |hour: f64| -> f64 {
            f64::from(target_temperature(
                elev_at(hour),
                status.day_temp,
                status.night_temp,
            ))
        };

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

        // Layer 2: the step. One day-or-night level per column, then corners
        // and a vertical run wherever the level flips.
        let mid = f64::from(status.day_temp + status.night_temp) / 2.0;
        let day_tint = theme::display_tint(status.day_temp);
        let night_tint = theme::display_tint(status.night_temp);
        let mid_tint = theme::display_tint((status.day_temp + status.night_temp) / 2);
        let levels: Vec<bool> = (0..w)
            .map(|x| kelvin_at((x as f64 + 0.5) / w as f64 * 24.0) >= mid)
            .collect();
        let buf = frame.buffer_mut();
        for (x, &is_day) in levels.iter().enumerate() {
            let (row, colour) = if is_day {
                (day_row, day_tint)
            } else {
                (night_row, night_tint)
            };
            if let Some(cell) = buf.cell_mut(Position::new(plot.x + x as u16, row)) {
                cell.set_symbol("─").set_fg(colour);
            }
        }
        for x in 0..w.saturating_sub(1) {
            if levels[x] == levels[x + 1] {
                continue;
            }
            let col = plot.x + (x + 1) as u16;
            let descending = levels[x];
            let (top_sym, bottom_sym) = if descending {
                ("╮", "╰")
            } else {
                ("╭", "╯")
            };
            if let Some(cell) = buf.cell_mut(Position::new(col, day_row)) {
                cell.set_symbol(top_sym).set_fg(mid_tint);
            }
            if let Some(cell) = buf.cell_mut(Position::new(col, night_row)) {
                cell.set_symbol(bottom_sym).set_fg(mid_tint);
            }
            for row in day_row + 1..night_row {
                if let Some(cell) = buf.cell_mut(Position::new(col, row)) {
                    cell.set_symbol("│").set_fg(mid_tint);
                }
            }
        }

        // Layer 3: the tint strip — every column wears the colour the screen
        // will be at that hour. This is f.lux's fill, one row tall.
        for x in 0..w {
            let kelvin = kelvin_at((x as f64 + 0.5) / w as f64 * 24.0).round() as u32;
            if let Some(cell) = buf.cell_mut(Position::new(plot.x + x as u16, strip_y)) {
                cell.set_symbol("▂").set_fg(theme::display_tint(kelvin));
            }
        }

        // Layer 4: the sunlight caption on the plateau's floor, skipped when
        // the plateau is too narrow to hold it.
        let daylight = (0..=960)
            .filter(|i| elev_at(f64::from(*i) * 0.025) > 0.0)
            .count() as f64
            * 0.025;
        let caption = format!("{daylight:.0}h sunlight");
        let day_cols: Vec<usize> = (0..w).filter(|&x| levels[x]).collect();
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

        // Layer 5: now — a dot riding the line, f.lux's sun bead.
        let now_temp = kelvin_at(now_hour);
        let now_col = plot.x + (now_hour / 24.0 * (w - 1) as f64).round() as u16;
        let span = f64::from(status.day_temp.saturating_sub(status.night_temp)).max(1.0);
        let frac = ((f64::from(status.day_temp) - now_temp) / span).clamp(0.0, 1.0);
        let now_row = day_row + (frac * f64::from(night_row - day_row)).round() as u16;
        if let Some(cell) = buf.cell_mut(Position::new(now_col, now_row)) {
            cell.set_symbol("●").set_fg(pal.text);
        }
    }

    /// The sun's arc for the today tab (#35): the day's elevation in braille —
    /// the smooth thing the schedule is derived from — cut into runs tinted by
    /// the temperature phase, over a dashed horizon rule, with the now
    /// crosshair. The now tab's curve shows the schedule; this shows why.
    fn draw_sun_arc(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let Some(status) = self.status.as_ref().filter(|s| s.has_location) else {
            return;
        };
        let (midnight, now_hour) = self.day_context();
        let elev_at = |hour: f64| -> f64 {
            solar_elevation(status.latitude, status.longitude, midnight + hour * 3600.0)
        };
        let kelvin_at = |hour: f64| -> f64 {
            f64::from(target_temperature(
                elev_at(hour),
                status.day_temp,
                status.night_temp,
            ))
        };

        let [top, x_row] =
            Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
        let [y_col, plot] =
            Layout::horizontal([Constraint::Length(7), Constraint::Min(10)]).areas(top);
        let w = plot.width as usize;
        if w < 8 || plot.height < 3 {
            return;
        }

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
        let (lo, hi) = (emin - 3.0, emax + 3.0);

        let day = f64::from(status.day_temp);
        let night = f64::from(status.night_temp);
        let phase_of = |kelvin: f64| {
            if kelvin >= day - 0.5 {
                0
            } else if kelvin <= night + 0.5 {
                2
            } else {
                1
            }
        };
        let tints = [
            theme::display_tint(status.day_temp),
            theme::display_tint((status.day_temp + status.night_temp) / 2),
            theme::display_tint(status.night_temp),
        ];
        let mut runs: Vec<(usize, Vec<(f64, f64)>)> = Vec::new();
        for &(hour, elev) in &arc {
            let phase = phase_of(kelvin_at(hour));
            match runs.last_mut() {
                Some((previous, run)) if *previous == phase => run.push((hour, elev)),
                _ => {
                    // Bridge from the previous run so the arc never gaps.
                    let mut run = Vec::new();
                    if let Some(&bridge) = runs.last().and_then(|(_, r)| r.last()) {
                        run.push(bridge);
                    }
                    run.push((hour, elev));
                    runs.push((phase, run));
                }
            }
        }

        // The dashed horizon rule and the now crosshair, as spaced scatter
        // dots; the Chart drops any point outside its bounds, so a polar day
        // or night simply loses the horizon instead of breaking.
        let horizon: Vec<(f64, f64)> = (0..=60).map(|i| (f64::from(i) * 0.4, 0.0)).collect();
        let now_line: Vec<(f64, f64)> = (0..=20)
            .map(|i| (now_hour, lo + f64::from(i) / 20.0 * (hi - lo)))
            .collect();
        let now_point = [(now_hour, elev_at(now_hour))];

        let mut datasets = vec![
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Scatter)
                .style(Style::default().fg(pal.faint))
                .data(&horizon),
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Scatter)
                .style(Style::default().fg(pal.faint))
                .data(&now_line),
        ];
        for (phase, run) in &runs {
            datasets.push(
                Dataset::default()
                    .marker(Marker::Braille)
                    .graph_type(GraphType::Line)
                    .style(Style::default().fg(tints[*phase]))
                    .data(run),
            );
        }
        datasets.push(
            Dataset::default()
                .marker(Marker::Dot)
                .style(Style::default().fg(pal.text))
                .data(&now_point),
        );
        let chart = Chart::new(datasets)
            .style(Style::default().bg(pal.surface))
            .x_axis(Axis::default().bounds([0.0, 24.0]))
            .y_axis(Axis::default().bounds([lo, hi]));
        frame.render_widget(chart, plot);

        // Degree labels in the gutter; the horizon label only when the rule
        // actually crosses the plot (a polar day or night has no horizon).
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
        y_label(format!("{emax:+.0}°"), plot.y);
        y_label(format!("{emin:+.0}°"), plot.y + plot.height - 1);
        if emin < 0.0 && emax > 0.0 {
            let horizon_row =
                plot.y + ((hi / (hi - lo)) * f64::from(plot.height - 1)).round() as u16;
            if horizon_row > plot.y && horizon_row < plot.y + plot.height - 1 {
                y_label("0°".into(), horizon_row);
                let word = "horizon";
                if w > word.len() + 4 {
                    let start = plot.x + (w - word.len() - 1) as u16;
                    let buf = frame.buffer_mut();
                    for (offset, glyph) in word.chars().enumerate() {
                        if let Some(cell) =
                            buf.cell_mut(Position::new(start + offset as u16, horizon_row))
                        {
                            cell.set_symbol(&glyph.to_string()).set_fg(pal.muted);
                        }
                    }
                }
            }
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
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config"))
        })
        .map(|base| {
            base.join("nightlightd")
                .join("config.toml")
                .display()
                .to_string()
        })
        .unwrap_or_else(|| "~/.config/nightlightd/config.toml".into())
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

/// Names the part of the day for a solar elevation, matching the daemon's
/// transition thresholds (full day at +3°, full night at -6°).
fn sun_phase(elevation: f64) -> &'static str {
    if elevation >= 3.0 {
        "day"
    } else if elevation <= -6.0 {
        "night"
    } else {
        "transition"
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
