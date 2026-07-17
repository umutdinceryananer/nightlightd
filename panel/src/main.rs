//! The control panel (#24): an f.lux-style slider to warm the screen by hand.
//!
//! A separate binary and crate, like the tray. It holds nothing the daemon
//! owns — dragging the slider sends `set_temperature`; "Back to automatic"
//! hands control to the sun again. Drawn with egui (pure Rust), whose canvas
//! will host the day/night curve in a later step.

mod autostart;
mod curve;
mod daemon;
mod single;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui;

use crate::daemon::Client;

/// The slider's ends, in kelvin. Below ~2000 K the screen goes deep orange;
/// 6500 K is neutral (no filter). Lower is warmer.
const WARMEST: u32 = 1500;
const NEUTRAL: u32 = 6500;

/// Where the slider starts before the user has touched it.
const START_KELVIN: u32 = 2800;

/// The figlet "slant" wordmark, the same one the README uses, embedded at
/// compile time so there is nothing to escape.
const WORDMARK: &str = include_str!("wordmark.txt");

/// The panel's whole state: the daemon connection, the manual-warm slider, the
/// day/night curve anchors, the start-at-login flag, and the local UTC offset.
struct Panel {
    client: Client,
    kelvin: u32,
    day_temp: u32,
    night_temp: u32,
    /// Whether the anchors have been seeded from the daemon yet (once, at the
    /// first status we receive).
    anchors_synced: bool,
    start_at_login: bool,
    offset_secs: i32,
    /// Set by the single-instance `Present` call; the loop clears it and raises
    /// the window.
    focus: Arc<AtomicBool>,
}

impl eframe::App for Panel {
    // eframe 0.35 wraps this in a CentralPanel itself, so we draw straight into
    // the provided `ui` instead of opening our own panel.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // A second launch asked us to come forward.
        if self.focus.swap(false, Ordering::Relaxed) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        let status = self.client.status();

        // In automatic mode the slider mirrors the live sun-based temperature,
        // so it drifts down as the sun sets and snaps back after "Back to
        // automatic". The first drag (below) switches the daemon to manual,
        // `following` goes false, and this stops — leaving the slider to the
        // user. When following, the slider already shows this value, so writing
        // it again is a no-op and never fights a drag.
        if let Some(status) = &status
            && status.following
        {
            self.kelvin = status.temperature.clamp(WARMEST, NEUTRAL);
        }

        // Seed the day/night sliders from the daemon once; after that they are
        // the source of truth (each change is sent and persisted).
        if !self.anchors_synced
            && let Some(status) = &status
        {
            self.day_temp = status.day_temp;
            self.night_temp = status.night_temp;
            self.anchors_synced = true;
        }

        ui.add(
            egui::Label::new(
                egui::RichText::new(WORDMARK.trim_end_matches('\n'))
                    .monospace()
                    .size(10.0)
                    .color(egui::Color32::from_rgb(255, 170, 90)),
            )
            .wrap_mode(egui::TextWrapMode::Extend),
        );
        ui.add_space(8.0);
        curve::show(ui, status.as_ref(), self.offset_secs);
        ui.add_space(10.0);

        // The two anchors that shape the curve; the daemon persists each change.
        if ui
            .add(
                egui::Slider::new(&mut self.day_temp, 4000..=6500)
                    .suffix(" K")
                    .text("Daytime"),
            )
            .changed()
        {
            self.client.set_day_temp(self.day_temp);
        }
        if ui
            .add(
                egui::Slider::new(&mut self.night_temp, 1500..=4500)
                    .suffix(" K")
                    .text("Nighttime"),
            )
            .changed()
        {
            self.client.set_night_temp(self.night_temp);
        }

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(8.0);
        ui.label("Warm the screen by hand — drag left for warmer:");
        ui.add_space(4.0);

        let slider = egui::Slider::new(&mut self.kelvin, WARMEST..=NEUTRAL).suffix(" K");
        // Apply live only when the user actually moves it; the daemon pins
        // whatever the slider lands on and switches to manual.
        if ui.add(slider).changed() {
            self.client.set_temperature(self.kelvin);
        }

        ui.add_space(12.0);
        if ui.button("Back to automatic").clicked() {
            self.client.follow_the_sun();
        }

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        // Enables/disables the daemon's systemd user service. The state was read
        // once at startup; a change writes it through immediately.
        if ui
            .checkbox(&mut self.start_at_login, "Start at login")
            .changed()
        {
            autostart::set(self.start_at_login);
        }

        // egui is reactive, so ask for a repaint each second to keep the slider
        // tracking the sun even when the window sits idle.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs(1));
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

fn main() -> eframe::Result<()> {
    // Single instance: if a panel is already open, ask it to come forward and
    // exit instead of opening a second window.
    let focus = Arc::new(AtomicBool::new(false));
    let _lock = match single::acquire(Arc::clone(&focus)) {
        Some(connection) => connection,
        None => return Ok(()),
    };

    let client = match Client::connect() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("nightlight-panel: cannot reach the session bus: {error}");
            std::process::exit(1);
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([460.0, 520.0])
            .with_resizable(false),
        ..Default::default()
    };
    eframe::run_native(
        "nightlightd",
        options,
        Box::new(|_cc| {
            Ok(Box::new(Panel {
                client,
                kelvin: START_KELVIN,
                day_temp: 6500,
                night_temp: 4500,
                anchors_synced: false,
                start_at_login: autostart::enabled(),
                offset_secs: local_offset_seconds(),
                focus: Arc::clone(&focus),
            }))
        }),
    )
}
