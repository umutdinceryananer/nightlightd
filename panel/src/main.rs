//! The control panel (#24): an f.lux-style slider to warm the screen by hand.
//!
//! A separate binary and crate, like the tray. It holds nothing the daemon
//! owns — dragging the slider sends `set_temperature`; "Back to automatic"
//! hands control to the sun again. Drawn with egui (pure Rust), whose canvas
//! will host the day/night curve in a later step.

mod daemon;

use eframe::egui;

use crate::daemon::Client;

/// The slider's ends, in kelvin. Below ~2000 K the screen goes deep orange;
/// 6500 K is neutral (no filter). Lower is warmer.
const WARMEST: u32 = 1500;
const NEUTRAL: u32 = 6500;

/// Where the slider starts before the user has touched it.
const START_KELVIN: u32 = 2800;

/// The panel's whole state: the daemon connection and the slider's value.
struct Panel {
    client: Client,
    kelvin: u32,
}

impl eframe::App for Panel {
    // eframe 0.35 wraps this in a CentralPanel itself, so we draw straight into
    // the provided `ui` instead of opening our own panel.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
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

        ui.heading("nightlightd");
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

        // egui is reactive, so ask for a repaint each second to keep the slider
        // tracking the sun even when the window sits idle.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs(1));
    }
}

fn main() -> eframe::Result<()> {
    let client = match Client::connect() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("nightlight-panel: cannot reach the session bus: {error}");
            std::process::exit(1);
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([380.0, 190.0])
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
            }))
        }),
    )
}
