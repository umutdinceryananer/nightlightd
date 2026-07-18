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

mod daemon;
mod theme;

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
use ratatui::widgets::{Axis, Block, BorderType, Chart, Dataset, GraphType, LineGauge, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use tui_big_text::{BigText, PixelSize};

use crate::daemon::{Client, Status};
use crate::theme::{Palette, THEMES};

/// Bounds and step for the night-temperature keys, mirroring the panel.
const NIGHT_MIN: u32 = 1500;
const NIGHT_STEP: u32 = 100;

/// The figlet "slant" wordmark, the same one the README and the panel use.
const WORDMARK: &str = include_str!("wordmark.txt");

struct App {
    client: Client,
    status: Option<Status>,
    last_poll: Option<Instant>,
    offset_secs: i32,
    theme_index: usize,
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

    fn palette(&self) -> Palette {
        THEMES[self.theme_index].palette(self.status.as_ref().map(|s| s.temperature))
    }

    fn draw(&self, frame: &mut Frame<'_>) {
        let pal = self.palette();
        let area = frame.area();
        if area.width < 66 || area.height < 26 {
            self.draw_compact(frame, area, &pal);
            return;
        }

        let [wordmark, strip, cards, curve, footer] = Layout::vertical([
            Constraint::Length(6),
            Constraint::Length(2),
            Constraint::Length(8),
            Constraint::Min(9),
            Constraint::Length(1),
        ])
        .areas(area);

        frame.render_widget(
            Paragraph::new(WORDMARK.trim_end_matches('\n')).style(Style::default().fg(pal.accent)),
            wordmark,
        );
        self.draw_strip(frame, strip, &pal);

        let [now_card, sun_card] =
            Layout::horizontal([Constraint::Length(32), Constraint::Min(32)]).areas(cards);
        self.draw_now_card(frame, now_card, &pal);
        self.draw_sun_card(frame, sun_card, &pal);
        self.draw_curve_card(frame, curve, &pal);
        frame.render_widget(footer_line(&pal), footer);
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
        frame.render_widget(footer_line(pal), footer);
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

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let secs_into_day = (now as i64 + i64::from(self.offset_secs)).rem_euclid(86_400) as f64;
        let midnight = now - secs_into_day;
        let now_hour = secs_into_day / 3600.0;

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
                .style(Style::default().fg(Color::White))
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
/// muted.
fn footer_line(pal: &Palette) -> Paragraph<'static> {
    let chip = |key: &str, label: &str| {
        vec![
            Span::styled(
                format!(" {key} "),
                Style::default().fg(Color::Black).bg(pal.accent),
            ),
            Span::styled(format!(" {label}   "), Style::default().fg(pal.muted)),
        ]
    };
    let mut spans = vec![Span::raw(" ")];
    spans.extend(chip("t", "toggle"));
    spans.extend(chip("a", "auto"));
    spans.extend(chip("↑↓", "night temp"));
    spans.extend(chip("T", "theme"));
    spans.extend(chip("q", "quit"));
    Paragraph::new(Line::from(spans))
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
