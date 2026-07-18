//! The TUI client (#35): a one-screen ratatui dashboard.
//!
//! A thin client like the tray and panel — no state beyond the last snapshot;
//! the daemon owns everything. One glanceable screen: the current temperature
//! and what drives it, where the sun is, the day/night curve drawn from the
//! same core maths the daemon runs, and a handful of keys. Deliberately not an
//! "application": no tabs, no views, no config editing beyond the night bound.

mod daemon;

use std::io;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nightlightd_core::solar::solar_elevation;
use nightlightd_core::transition::target_temperature;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::symbols::Marker;
use ratatui::text::Line;
use ratatui::widgets::{Axis, Block, Chart, Dataset, GraphType, Paragraph};
use ratatui::{DefaultTerminal, Frame};

use crate::daemon::{Client, Status};

/// The warm identity colour, matching the panel's curve.
const WARM: Color = Color::Rgb(255, 170, 90);
/// Bounds and step for the night-temperature keys, mirroring the panel.
const NIGHT_MIN: u32 = 1500;
const NIGHT_STEP: u32 = 100;

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
        let outer = Block::bordered().title(" nightlightd — colour temperature ");
        let inner = outer.inner(frame.area());
        frame.render_widget(outer, frame.area());

        let [header, chart, footer] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(6),
            Constraint::Length(1),
        ])
        .areas(inner);

        frame.render_widget(Paragraph::new(self.header_lines()), header);
        self.draw_curve(frame, chart);
        frame.render_widget(
            Paragraph::new(Line::from(" [t]oggle   [a]uto   [↑/↓] night temp   [q]uit"))
                .style(Style::default().fg(Color::DarkGray)),
            footer,
        );
    }

    /// The two status lines at the top: what is applied and why, and the sun.
    fn header_lines(&self) -> Vec<Line<'_>> {
        match &self.status {
            Some(status) => {
                let onoff = if status.enabled { "on" } else { "off" };
                let mut lines = vec![Line::from(format!(
                    " {} · {} K · {}",
                    onoff, status.temperature, status.source
                ))];
                if status.has_location {
                    lines.push(Line::from(format!(
                        " sun {:+.1}° ({}) at {:.1}°, {:.1}° · day {} K / night {} K",
                        status.elevation,
                        sun_phase(status.elevation),
                        status.latitude,
                        status.longitude,
                        status.day_temp,
                        status.night_temp,
                    )));
                }
                lines
            }
            None => vec![Line::from(" daemon not running").style(Style::default().fg(Color::Red))],
        }
    }

    /// The day/night curve, from the same core maths the daemon runs, with a
    /// dot at "now". Falls back to a hint when no location is known.
    fn draw_curve(&self, frame: &mut Frame<'_>, area: ratatui::layout::Rect) {
        let Some(status) = self.status.as_ref().filter(|s| s.has_location) else {
            frame.render_widget(
                Paragraph::new(" no location — the curve needs one")
                    .style(Style::default().fg(Color::DarkGray)),
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
        let now_point = [(now_hour, kelvin_at(now_hour))];

        let night = f64::from(status.night_temp);
        let day = f64::from(status.day_temp);
        let pad = ((day - night) * 0.08).max(50.0);

        let datasets = vec![
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
                    .style(Style::default().fg(Color::DarkGray)),
            )
            .y_axis(
                Axis::default()
                    .bounds([night - pad, day + pad])
                    .labels([
                        format!("{} K", status.night_temp),
                        format!("{} K", status.day_temp),
                    ])
                    .style(Style::default().fg(Color::DarkGray)),
            );
        frame.render_widget(chart, area);
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
