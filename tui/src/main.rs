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
use nightlightd_core::solar::solar_elevation;
use nightlightd_core::transition::target_temperature;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Map, MapResolution};
use ratatui::widgets::{
    Axis, Block, BorderType, Chart, Clear, Dataset, GraphType, LineGauge, Paragraph, Row, Table,
    Tabs,
};
use ratatui::{DefaultTerminal, Frame};
use tui_big_text::{BigText, PixelSize};

use crate::daemon::{Client, Status};
use crate::theme::{Palette, THEMES};

/// Bounds and step for the night-temperature keys, mirroring the panel.
const NIGHT_MIN: u32 = 1500;
const NIGHT_STEP: u32 = 100;

/// The figlet "slant" wordmark, the same one the README and the panel use.
const WORDMARK: &str = include_str!("wordmark.txt");

/// The tab bar, in order. Each holds real content or it does not exist.
const TABS: &[&str] = &["now", "today", "location", "settings"];
const LOCATION_TAB: usize = 2;
/// The settings tab's index and its selectable rows: day, night, theme, login.
const SETTINGS_TAB: usize = 3;
const SETTINGS_ITEMS: usize = 4;

/// Picker steps in degrees — coarse on purpose; braille map cells are chunky.
const PICK_LAT_STEP: f64 = 3.0;
const PICK_LON_STEP: f64 = 5.0;

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
    start_at_login: bool,
    /// The map's location picker: `Some((lat, lon))` cursor while picking.
    picker: Option<(f64, f64)>,
}

fn main() -> io::Result<()> {
    let theme_index = match parse_theme_arg() {
        Ok(index) => index,
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
        tab: 0,
        settings_selected: 0,
        theme_popup: None,
        start_at_login: autostart::enabled(),
        picker: None,
    };

    let mut terminal = ratatui::init();
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
}

/// Minimal argument parsing: `--theme <name>` (or `-t <name>`); anything else
/// prints usage. No clap — two flags do not justify a dependency.
fn parse_theme_arg() -> Result<usize, String> {
    let names = || {
        THEMES
            .iter()
            .map(|theme| theme.name)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        None => Ok(0),
        Some("--theme" | "-t") => match args.next() {
            Some(name) => theme::index_of(&name)
                .ok_or_else(|| format!("unknown theme {name:?} — available: {}", names())),
            None => Err(format!("--theme needs a name — available: {}", names())),
        },
        Some(other) => Err(format!(
            "unknown argument {other:?}\nusage: nightlight-tui [--theme <{}>]",
            names()
        )),
    }
}

impl App {
    /// Draw, wait briefly for a key, repeat. The wait doubles as the refresh
    /// pace; the status itself is re-read at most once a second.
    fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        loop {
            if self
                .last_poll
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(1))
            {
                self.status = self.client.status();
                self.last_poll = Some(Instant::now());
            }
            terminal.draw(|frame| self.draw(frame))?;
            if event::poll(Duration::from_millis(250))?
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
            _ => {}
        }
    }

    /// Keys while the map picker is active; returns `true` to quit the app.
    fn picker_key(&mut self, code: KeyCode) -> bool {
        let Some((lat, lon)) = self.picker else {
            return false;
        };
        match code {
            KeyCode::Up => self.picker = Some(((lat + PICK_LAT_STEP).min(85.0), lon)),
            KeyCode::Down => self.picker = Some(((lat - PICK_LAT_STEP).max(-85.0), lon)),
            KeyCode::Right => self.picker = Some((lat, (lon + PICK_LON_STEP).min(179.0))),
            KeyCode::Left => self.picker = Some((lat, (lon - PICK_LON_STEP).max(-179.0))),
            KeyCode::Enter => {
                self.client.set_location(lat, lon);
                self.picker = None;
                self.last_poll = None;
            }
            KeyCode::Esc => self.picker = None,
            KeyCode::Char('q') => return true,
            _ => {}
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

    fn draw(&self, frame: &mut Frame<'_>) {
        let pal = self.palette();
        let area = frame.area();
        // Paint the whole screen in the theme's background and text tones —
        // the palette owns the canvas, not the terminal's default colours.
        frame.render_widget(
            Block::default().style(Style::default().bg(pal.bg).fg(pal.text)),
            area,
        );
        if area.width < 66 || area.height < 26 {
            self.draw_compact(frame, area, &pal);
            return;
        }

        let [wordmark, strip, tabs, content, footer] = Layout::vertical([
            Constraint::Length(6),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Min(9),
            Constraint::Length(1),
        ])
        .areas(area);

        frame.render_widget(
            Paragraph::new(WORDMARK.trim_end_matches('\n')).style(Style::default().fg(pal.accent)),
            wordmark,
        );
        self.draw_strip(frame, strip, &pal);
        self.draw_tabs(frame, tabs, &pal);
        match self.tab {
            1 => self.draw_today_tab(frame, content, &pal),
            2 => self.draw_location_tab(frame, content, &pal),
            3 => self.draw_settings_tab(frame, content, &pal),
            _ => self.draw_now_tab(frame, content, &pal),
        }
        frame.render_widget(footer_line(self.tab, &pal), footer);
        if self.theme_popup.is_some() {
            self.draw_theme_popup(frame, area, &pal);
        }
    }

    /// Tab 3: the world map — the resolved location marked on it, and a picker
    /// to pin a manual one. The map is ratatui's own braille world.
    fn draw_location_tab(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let block = card(" location ", pal);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let [map_area, info] =
            Layout::vertical([Constraint::Min(8), Constraint::Length(2)]).areas(inner);

        let active = self
            .status
            .as_ref()
            .filter(|s| s.has_location)
            .map(|s| (s.latitude, s.longitude));
        let picker = self.picker;
        let accent = pal.accent;
        let faint = pal.faint;
        let text = pal.text;
        let canvas = Canvas::default()
            .marker(Marker::Braille)
            .x_bounds([-180.0, 180.0])
            .y_bounds([-90.0, 90.0])
            .paint(move |ctx| {
                ctx.draw(&Map {
                    resolution: MapResolution::High,
                    color: faint,
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
                        Style::default().fg(pal.text),
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
        frame.render_widget(Paragraph::new(lines), info);
    }

    /// Tab 4: the settings — the two bounds, the theme, autostart, and where
    /// the config lives. Row-based: arrows select and adjust, enter acts.
    fn draw_settings_tab(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let block = card(" settings ", pal);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let value = |v: Option<String>| v.unwrap_or_else(|| "—".into());
        let day = value(self.status.as_ref().map(|s| format!("{} K", s.day_temp)));
        let night = value(self.status.as_ref().map(|s| format!("{} K", s.night_temp)));
        let rows: [(&str, String, &str); SETTINGS_ITEMS] = [
            ("daytime", day, "‹ › adjust"),
            ("nighttime", night, "‹ › adjust"),
            (
                "theme",
                THEMES[self.theme_index].name.to_string(),
                "⏎ choose · ‹ › cycle",
            ),
            (
                "start at login",
                if self.start_at_login {
                    "[x] enabled".to_string()
                } else {
                    "[ ] disabled".to_string()
                },
                "⏎ toggle",
            ),
        ];

        let mut lines: Vec<Line<'_>> = vec![Line::default()];
        for (index, (label, val, hint)) in rows.into_iter().enumerate() {
            let body = format!("  {label:<16} {val:<14}");
            if index == self.settings_selected {
                lines.push(Line::from(vec![
                    Span::styled(body, Style::default().fg(pal.bg).bg(pal.accent).bold()),
                    Span::styled(format!("  {hint}"), Style::default().fg(pal.muted)),
                ]));
            } else {
                lines.push(Line::from(vec![
                    Span::styled(format!("  {label:<16} "), Style::default().fg(pal.muted)),
                    Span::styled(format!("{val:<14}"), Style::default().fg(pal.text)),
                ]));
            }
            lines.push(Line::default());
        }
        lines.push(Line::from(Span::styled(
            format!("  config  {}", config_path_display()),
            Style::default().fg(pal.faint),
        )));
        lines.push(Line::from(Span::styled(
            "          day & night changes persist there automatically",
            Style::default().fg(pal.faint),
        )));
        frame.render_widget(Paragraph::new(lines), inner);
    }

    /// The theme picker, as a centered modal over everything.
    fn draw_theme_popup(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let Some(selected) = self.theme_popup else {
            return;
        };
        let width = 28.min(area.width);
        let height = (THEMES.len() as u16 + 2).min(area.height);
        let popup = Rect {
            x: area.x + (area.width.saturating_sub(width)) / 2,
            y: area.y + (area.height.saturating_sub(height)) / 2,
            width,
            height,
        };
        frame.render_widget(Clear, popup);
        let block = card(" theme — ⏎ apply ", pal).style(Style::default().bg(pal.bg));
        let inner = block.inner(popup);
        frame.render_widget(block, popup);

        let lines: Vec<Line<'_>> = THEMES
            .iter()
            .enumerate()
            .map(|(index, theme)| {
                let current = if index == self.theme_index {
                    "•"
                } else {
                    " "
                };
                let body = format!(" {current} {:<20}", theme.name);
                if index == selected {
                    Line::from(Span::styled(
                        body,
                        Style::default().fg(pal.bg).bg(pal.accent).bold(),
                    ))
                } else {
                    Line::from(Span::styled(body, Style::default().fg(pal.text)))
                }
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
    }

    /// The tab bar: numbered titles, the active one on an accent chip.
    fn draw_tabs(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let titles: Vec<Line<'_>> = TABS
            .iter()
            .enumerate()
            .map(|(i, name)| Line::from(format!(" {} {name} ", i + 1)))
            .collect();
        frame.render_widget(
            Tabs::new(titles)
                .select(self.tab)
                .style(Style::default().fg(pal.muted))
                .highlight_style(Style::default().fg(pal.bg).bg(pal.accent).bold())
                .divider(Span::styled("·", Style::default().fg(pal.faint))),
            area,
        );
    }

    /// Tab 1: the dashboard — state cards on top, the curve below.
    fn draw_now_tab(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let [cards, curve] =
            Layout::vertical([Constraint::Length(8), Constraint::Min(5)]).areas(area);
        let [now_card, sun_card] =
            Layout::horizontal([Constraint::Length(32), Constraint::Min(32)]).areas(cards);
        self.draw_now_card(frame, now_card, pal);
        self.draw_sun_card(frame, sun_card, pal);
        self.draw_curve_card(frame, curve, pal);
    }

    /// Tab 2: the day's solar milestones, derived from the real curve, with
    /// the next event highlighted — then the curve for context.
    fn draw_today_tab(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let block = card(" today ", pal);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let Some(status) = self.status.as_ref().filter(|s| s.has_location) else {
            frame.render_widget(
                Paragraph::new(" no location — the schedule needs one")
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

        let table_height = (events.len() + 1) as u16;
        let [table_area, chart_area] =
            Layout::vertical([Constraint::Length(table_height), Constraint::Min(0)]).areas(inner);

        let rows: Vec<Row<'_>> = events
            .iter()
            .enumerate()
            .map(|(i, event)| {
                let row = Row::new(vec![
                    format!(" {}", event.name),
                    event.hhmm(),
                    format!("{} K", event.kelvin),
                    relative(event.hour - now_hour),
                ]);
                if Some(i) == next {
                    row.style(Style::default().fg(pal.bg).bg(pal.accent).bold())
                } else if event.hour < now_hour {
                    row.style(Style::default().fg(pal.muted))
                } else {
                    row
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
            Row::new(vec![" event", "time", "kelvin", "when"])
                .style(Style::default().fg(pal.faint)),
        );
        frame.render_widget(table, table_area);

        if chart_area.height >= 5 {
            self.draw_chart(frame, chart_area, pal);
        }
    }

    /// Local midnight (unix) and the fractional local hour of "now".
    fn day_context(&self) -> (f64, f64) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let secs_into_day = (now as i64 + i64::from(self.offset_secs)).rem_euclid(86_400) as f64;
        (now - secs_into_day, secs_into_day / 3600.0)
    }

    /// The status strip under the wordmark: liveness, mode, local time, sun —
    /// and, right-aligned, the version and the active theme.
    fn draw_strip(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let mut spans = vec![Span::raw(" ")];
        match &self.status {
            Some(status) => {
                spans.push(Span::styled("● ", Style::default().fg(pal.ok)));
                spans.push(Span::styled("daemon", Style::default().fg(pal.muted)));
                spans.push(Span::styled("  ·  ", Style::default().fg(pal.faint)));
                let mode = if !status.enabled {
                    "OFF"
                } else if status.following {
                    "AUTO"
                } else {
                    "MANUAL"
                };
                spans.push(Span::styled(mode, Style::default().fg(pal.accent).bold()));
                spans.push(Span::styled("  ·  ", Style::default().fg(pal.faint)));
                spans.push(Span::styled(
                    self.local_hhmm(),
                    Style::default().fg(pal.muted),
                ));
                if status.has_location {
                    spans.push(Span::styled("  ·  ", Style::default().fg(pal.faint)));
                    spans.push(Span::styled(
                        format!("sun {:+.1}°", status.elevation),
                        Style::default().fg(pal.muted),
                    ));
                }
            }
            None => {
                spans.push(Span::styled("○ ", Style::default().fg(pal.err)));
                spans.push(Span::styled(
                    "daemon not running",
                    Style::default().fg(pal.err),
                ));
            }
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(
                    "v{} · theme {} ",
                    env!("CARGO_PKG_VERSION"),
                    THEMES[self.theme_index].name
                ),
                Style::default().fg(pal.faint),
            )))
            .alignment(Alignment::Right),
            area,
        );
    }

    fn local_hhmm(&self) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let day_secs = (now + i64::from(self.offset_secs)).rem_euclid(86_400);
        format!("{:02}:{:02}", day_secs / 3600, (day_secs % 3600) / 60)
    }

    /// The fallback for small terminals: no wordmark, no cards — just the
    /// status lines, the curve, and the keys.
    fn draw_compact(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let outer = card(" nightlightd ", pal);
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
        frame.render_widget(footer_line(0, pal), footer);
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
        let block = card(" now ", pal);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let Some(status) = &self.status else {
            frame.render_widget(
                Paragraph::new("\n daemon not running").style(Style::default().fg(pal.err)),
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
                .lines(vec![Line::from(format!("{}K", status.temperature))])
                .build(),
            big,
        );
    }

    /// Right card: where the sun is, where we are, and how far into the night
    /// the transition has come.
    fn draw_sun_card(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let block = card(" sun ", pal);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let Some(status) = self.status.as_ref().filter(|s| s.has_location) else {
            frame.render_widget(
                Paragraph::new("\n no location resolved").style(Style::default().fg(pal.muted)),
                inner,
            );
            return;
        };

        let [text, gauge] =
            Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(inner);

        let phase = sun_phase(status.elevation);
        let icon = match phase {
            "day" => "☀",
            "night" => "☾",
            _ => "◐",
        };
        let lat_hemisphere = if status.latitude >= 0.0 { "N" } else { "S" };
        let lon_hemisphere = if status.longitude >= 0.0 { "E" } else { "W" };
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled(format!(" {icon} "), Style::default().fg(pal.accent)),
                    Span::styled(
                        format!("{:+.1}°", status.elevation),
                        Style::default().bold(),
                    ),
                    Span::styled(format!("  {phase}"), Style::default().fg(pal.muted)),
                ]),
                Line::from(Span::styled(
                    format!(
                        "   {:.1}°{lat_hemisphere} {:.1}°{lon_hemisphere} · from the timezone",
                        status.latitude.abs(),
                        status.longitude.abs(),
                    ),
                    Style::default().fg(pal.muted),
                )),
                Line::from(Span::styled(
                    format!(
                        "   day {} K · night {} K",
                        status.day_temp, status.night_temp
                    ),
                    Style::default().fg(pal.muted),
                )),
            ]),
            text,
        );

        let ratio = ((status.elevation + 6.0) / 9.0).clamp(0.0, 1.0);
        frame.render_widget(
            LineGauge::default()
                .ratio(ratio)
                .label(Span::styled("☾⟷☀", Style::default().fg(pal.muted)))
                .filled_style(Style::default().fg(pal.accent))
                .unfilled_style(Style::default().fg(pal.faint)),
            gauge,
        );
    }

    fn draw_curve_card(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let block = card(" today ", pal);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.draw_chart(frame, inner, pal);
    }

    /// The day/night curve, from the same core maths the daemon runs: a faint
    /// vertical line and a dot mark "now". Falls back to a hint when no
    /// location is known.
    fn draw_chart(&self, frame: &mut Frame<'_>, area: Rect, pal: &Palette) {
        let Some(status) = self.status.as_ref().filter(|s| s.has_location) else {
            frame.render_widget(
                Paragraph::new(" no location — the curve needs one")
                    .style(Style::default().fg(pal.muted)),
                area,
            );
            return;
        };

        let (midnight, now_hour) = self.day_context();

        let kelvin_at = |hour: f64| -> f64 {
            let elevation =
                solar_elevation(status.latitude, status.longitude, midnight + hour * 3600.0);
            f64::from(target_temperature(
                elevation,
                status.day_temp,
                status.night_temp,
            ))
        };
        let points: Vec<(f64, f64)> = (0..=192)
            .map(|i| {
                let h = f64::from(i) / 8.0;
                (h, kelvin_at(h))
            })
            .collect();

        let night = f64::from(status.night_temp);
        let day = f64::from(status.day_temp);
        let pad = ((day - night) * 0.08).max(50.0);
        let now_line = [(now_hour, night - pad), (now_hour, day + pad)];
        let now_point = [(now_hour, kelvin_at(now_hour))];

        let datasets = vec![
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(pal.faint))
                .data(&now_line),
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(pal.accent))
                .data(&points),
            Dataset::default()
                .marker(Marker::Dot)
                .style(Style::default().fg(pal.text))
                .data(&now_point),
        ];
        let chart = Chart::new(datasets)
            .x_axis(
                Axis::default()
                    .bounds([0.0, 24.0])
                    .labels(["00", "06", "12", "18", "24"])
                    .style(Style::default().fg(pal.muted)),
            )
            .y_axis(
                Axis::default()
                    .bounds([night - pad, day + pad])
                    .labels([
                        format!("{} K", status.night_temp),
                        format!("{} K", status.day_temp),
                    ])
                    .style(Style::default().fg(pal.muted)),
            );
        frame.render_widget(chart, area);
    }
}

/// A rounded card with a bold accent title and muted borders — the shared look.
fn card<'a>(title: &'a str, pal: &Palette) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.muted))
        .title(Span::styled(title, Style::default().fg(pal.accent).bold()))
}

/// The key hints, styled as chips: the key on an accent background, the label
/// muted. The set follows the active tab.
fn footer_line(tab: usize, pal: &Palette) -> Paragraph<'static> {
    let chip = |key: &str, label: &str| {
        vec![
            Span::styled(
                format!(" {key} "),
                Style::default().fg(pal.bg).bg(pal.accent),
            ),
            Span::styled(format!(" {label}   "), Style::default().fg(pal.muted)),
        ]
    };
    let mut spans = vec![Span::raw(" ")];
    spans.extend(chip("⇥", "tab"));
    if tab == SETTINGS_TAB {
        spans.extend(chip("↑↓", "select"));
        spans.extend(chip("‹›", "adjust"));
        spans.extend(chip("⏎", "apply"));
    } else if tab == LOCATION_TAB {
        spans.extend(chip("⏎", "pick"));
        spans.extend(chip("c", "timezone"));
        spans.extend(chip("T", "theme"));
    } else {
        spans.extend(chip("t", "toggle"));
        spans.extend(chip("a", "auto"));
        spans.extend(chip("↑↓", "night temp"));
        spans.extend(chip("T", "theme"));
    }
    spans.extend(chip("q", "quit"));
    Paragraph::new(Line::from(spans))
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
