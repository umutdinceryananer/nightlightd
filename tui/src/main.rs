//! The TUI client (#35): a one-screen ratatui dashboard.
//!
//! A thin client like the tray and panel — no state beyond the last snapshot;
//! the daemon owns everything. One glanceable screen, but a *designed* one:
//! the slant wordmark, a "now" card whose big temperature readout is tinted
//! with the actual colour of the screen, a sun card with a night⟷day gauge,
//! and the day/night curve from the same core maths the daemon runs.
//! Deliberately no tabs, views, or config editing beyond the night bound:
//! this is a remote control, not an application.

mod daemon;

use std::io;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nightlightd_core::color::temperature_to_rgb;
use nightlightd_core::solar::solar_elevation;
use nightlightd_core::transition::target_temperature;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Axis, Block, BorderType, Chart, Dataset, GraphType, LineGauge, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use tui_big_text::{BigText, PixelSize};

use crate::daemon::{Client, Status};

/// The warm identity colour, matching the panel's curve and the README art.
const WARM: Color = Color::Rgb(255, 170, 90);
/// Secondary chrome: borders, axes, hints.
const DIM: Color = Color::DarkGray;
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
}

fn main() -> io::Result<()> {
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
    };

    let mut terminal = ratatui::init();
    let result = app.run(&mut terminal);
    ratatui::restore();
    result
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

    /// Handles one keypress; returns `true` to quit. Every action invalidates
    /// the snapshot so the next frame shows the daemon's answer.
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

    fn draw(&self, frame: &mut Frame<'_>) {
        let area = frame.area();
        if area.width < 66 || area.height < 24 {
            self.draw_compact(frame, area);
            return;
        }

        let [wordmark, cards, curve, footer] = Layout::vertical([
            Constraint::Length(7),
            Constraint::Length(8),
            Constraint::Min(9),
            Constraint::Length(1),
        ])
        .areas(area);

        frame.render_widget(
            Paragraph::new(WORDMARK.trim_end_matches('\n')).style(Style::default().fg(WARM)),
            wordmark,
        );

        let [now_card, sun_card] =
            Layout::horizontal([Constraint::Length(32), Constraint::Min(32)]).areas(cards);
        self.draw_now_card(frame, now_card);
        self.draw_sun_card(frame, sun_card);
        self.draw_curve_card(frame, curve);
        frame.render_widget(footer_line(), footer);
    }

    /// The fallback for small terminals: no wordmark, no cards — just the
    /// status lines, the curve, and the keys.
    fn draw_compact(&self, frame: &mut Frame<'_>, area: Rect) {
        let outer = card(" nightlightd ");
        let inner = outer.inner(area);
        frame.render_widget(outer, area);
        let [header, chart, footer] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .areas(inner);
        frame.render_widget(Paragraph::new(self.compact_header()), header);
        self.draw_chart(frame, chart);
        frame.render_widget(footer_line(), footer);
    }

    fn compact_header(&self) -> Vec<Line<'_>> {
        match &self.status {
            Some(status) => {
                let onoff = if status.enabled { "on" } else { "off" };
                vec![
                    Line::from(format!(
                        " {} · {} K · {}",
                        onoff, status.temperature, status.source
                    )),
                    Line::from(format!(
                        " sun {:+.1}° ({}) · day {} K / night {} K",
                        status.elevation,
                        sun_phase(status.elevation),
                        status.day_temp,
                        status.night_temp,
                    )),
                ]
            }
            None => vec![Line::from(" daemon not running").red()],
        }
    }

    /// Left card: state badges and the big temperature readout, tinted with
    /// the actual colour the screen is filtered to right now.
    fn draw_now_card(&self, frame: &mut Frame<'_>, area: Rect) {
        let block = card(" now ");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let Some(status) = &self.status else {
            frame.render_widget(
                Paragraph::new("\n daemon not running").style(Style::default().fg(Color::Red)),
                inner,
            );
            return;
        };

        let [badges, big] =
            Layout::vertical([Constraint::Length(2), Constraint::Min(4)]).areas(inner);

        let (dot, dot_colour) = if status.enabled {
            ("●", Color::Green)
        } else {
            ("○", Color::Red)
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
                Span::styled("  ·  ", Style::default().fg(DIM)),
                Span::styled(mode, Style::default().fg(WARM).bold()),
            ])),
            badges,
        );

        // The number wears the tint the screen wears: white at 6500 K,
        // candle-orange at 2000 K. Dimmed when the filter is off.
        let (r, g, b) = temperature_to_rgb(status.temperature);
        let tint = if status.enabled {
            Color::Rgb(
                (r * 255.0).round() as u8,
                (g * 255.0).round() as u8,
                (b * 255.0).round() as u8,
            )
        } else {
            DIM
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
    fn draw_sun_card(&self, frame: &mut Frame<'_>, area: Rect) {
        let block = card(" sun ");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let Some(status) = self.status.as_ref().filter(|s| s.has_location) else {
            frame.render_widget(
                Paragraph::new("\n no location resolved").style(Style::default().fg(DIM)),
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
                    Span::styled(format!(" {icon} "), Style::default().fg(WARM)),
                    Span::styled(
                        format!("{:+.1}°", status.elevation),
                        Style::default().bold(),
                    ),
                    Span::styled(format!("  {phase}"), Style::default().fg(DIM)),
                ]),
                Line::from(Span::styled(
                    format!(
                        "   {:.1}°{lat_hemisphere} {:.1}°{lon_hemisphere} · from the timezone",
                        status.latitude.abs(),
                        status.longitude.abs(),
                    ),
                    Style::default().fg(DIM),
                )),
                Line::from(Span::styled(
                    format!(
                        "   day {} K · night {} K",
                        status.day_temp, status.night_temp
                    ),
                    Style::default().fg(DIM),
                )),
            ]),
            text,
        );

        // Where the transition stands: full night at -6°, full day at +3°.
        let ratio = ((status.elevation + 6.0) / 9.0).clamp(0.0, 1.0);
        frame.render_widget(
            LineGauge::default()
                .ratio(ratio)
                .label(Span::styled("☾⟷☀", Style::default().fg(DIM)))
                .filled_style(Style::default().fg(WARM))
                .unfilled_style(Style::default().fg(Color::Rgb(60, 60, 60))),
            gauge,
        );
    }

    fn draw_curve_card(&self, frame: &mut Frame<'_>, area: Rect) {
        let block = card(" today ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        self.draw_chart(frame, inner);
    }

    /// The day/night curve, from the same core maths the daemon runs: a dotted
    /// vertical line and a dot mark "now". Falls back to a hint when no
    /// location is known.
    fn draw_chart(&self, frame: &mut Frame<'_>, area: Rect) {
        let Some(status) = self.status.as_ref().filter(|s| s.has_location) else {
            frame.render_widget(
                Paragraph::new(" no location — the curve needs one")
                    .style(Style::default().fg(DIM)),
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
                .style(Style::default().fg(Color::Rgb(70, 70, 70)))
                .data(&now_line),
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(WARM))
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
                    .style(Style::default().fg(DIM)),
            )
            .y_axis(
                Axis::default()
                    .bounds([night - pad, day + pad])
                    .labels([
                        format!("{} K", status.night_temp),
                        format!("{} K", status.day_temp),
                    ])
                    .style(Style::default().fg(DIM)),
            );
        frame.render_widget(chart, area);
    }
}

/// A rounded, dim-bordered card with a bold warm title — the shared look.
fn card(title: &str) -> Block<'_> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(DIM))
        .title(Span::styled(title, Style::default().fg(WARM).bold()))
}

/// The key hints, styled as chips: the key on a warm background, the label dim.
fn footer_line() -> Paragraph<'static> {
    fn chip(key: &str, label: &str) -> Vec<Span<'static>> {
        vec![
            Span::styled(
                format!(" {key} "),
                Style::default().fg(Color::Black).bg(WARM),
            ),
            Span::styled(format!(" {label}   "), Style::default().fg(DIM)),
        ]
    }
    let mut spans = vec![Span::raw(" ")];
    spans.extend(chip("t", "toggle"));
    spans.extend(chip("a", "auto"));
    spans.extend(chip("↑↓", "night temp"));
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
