//! The day/night curve: colour temperature across today, like f.lux's graph.
//!
//! Sampled from the same core maths the daemon uses — solar elevation at the
//! resolved location, run through the transition curve between the configured
//! day and night bounds — so the picture is exactly what the daemon will do.
//! A vertical marker shows the current time.

use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Color32, Pos2, Stroke};
use nightlightd_core::solar::solar_elevation;
use nightlightd_core::transition::target_temperature;

use crate::daemon::Status;

/// Height of the curve area in points.
const HEIGHT: f32 = 130.0;
/// Vertical breathing room so the line never touches the top or bottom edge.
const PAD: f32 = 12.0;

/// Draws the curve. `status` supplies the location; `day_temp`/`night_temp`
/// come from the panel's live slider values so the shape follows a drag before
/// the daemon has been told. Shows a placeholder when no location is known
/// (the curve is meaningless without one). `offset_secs` is the local UTC
/// offset, used to place "now" and the hour axis on local time.
pub fn show(
    ui: &mut egui::Ui,
    status: Option<&Status>,
    day_temp: u32,
    night_temp: u32,
    offset_secs: i32,
) {
    let Some(status) = status.filter(|s| s.has_location) else {
        ui.weak("Waiting for the daemon / location…");
        return;
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let secs_into_day = (now as i64 + i64::from(offset_secs)).rem_euclid(86_400) as f64;
    let midnight = now - secs_into_day;
    let now_hour = (secs_into_day / 3600.0) as f32;

    // Kelvin at a given local hour today, from the same maths the daemon runs.
    let kelvin_at = |hour: f32| -> f32 {
        let t = midnight + f64::from(hour) * 3600.0;
        let elevation = solar_elevation(status.latitude, status.longitude, t);
        target_temperature(elevation, day_temp, night_temp) as f32
    };

    let (response, painter) = ui.allocate_painter(
        egui::vec2(ui.available_width(), HEIGHT),
        egui::Sense::hover(),
    );
    let rect = response.rect;

    let to_x = |hour: f32| rect.left() + (hour / 24.0) * rect.width();
    let night = night_temp as f32;
    let day = day_temp as f32;
    let span = (day - night).max(1.0);
    let to_y = |kelvin: f32| {
        let frac = ((kelvin - night) / span).clamp(0.0, 1.0);
        rect.bottom() - PAD - frac * (rect.height() - 2.0 * PAD)
    };

    painter.rect_filled(rect, 6.0, Color32::from_gray(24));

    // Warm fill under the curve, one convex trapezoid per segment so a concave
    // curve never triangulates wrong.
    let samples: Vec<(f32, f32)> = (0..=48)
        .map(|i| {
            let h = i as f32 * 0.5;
            (h, kelvin_at(h))
        })
        .collect();
    let fill = Color32::from_rgba_unmultiplied(255, 150, 60, 32);
    for pair in samples.windows(2) {
        let a = egui::pos2(to_x(pair[0].0), to_y(pair[0].1));
        let b = egui::pos2(to_x(pair[1].0), to_y(pair[1].1));
        let quad = vec![
            egui::pos2(a.x, rect.bottom()),
            a,
            b,
            egui::pos2(b.x, rect.bottom()),
        ];
        painter.add(egui::Shape::convex_polygon(quad, fill, Stroke::NONE));
    }

    // The curve itself, warm orange.
    let line: Vec<Pos2> = (0..=96)
        .map(|i| {
            let h = i as f32 * 0.25;
            egui::pos2(to_x(h), to_y(kelvin_at(h)))
        })
        .collect();
    painter.add(egui::Shape::line(
        line,
        Stroke::new(2.0, Color32::from_rgb(255, 170, 90)),
    ));

    // "Now": a vertical marker and a dot on the line.
    let now_x = to_x(now_hour);
    painter.line_segment(
        [
            egui::pos2(now_x, rect.top()),
            egui::pos2(now_x, rect.bottom()),
        ],
        Stroke::new(1.0, Color32::from_white_alpha(70)),
    );
    painter.circle_filled(
        egui::pos2(now_x, to_y(kelvin_at(now_hour))),
        4.0,
        Color32::WHITE,
    );

    // Hour ticks, edge-aligned so 0 and 24 are not clipped.
    for h in [0, 6, 12, 18, 24] {
        let align = match h {
            0 => egui::Align2::LEFT_BOTTOM,
            24 => egui::Align2::RIGHT_BOTTOM,
            _ => egui::Align2::CENTER_BOTTOM,
        };
        painter.text(
            egui::pos2(to_x(h as f32), rect.bottom() - 3.0),
            align,
            format!("{h:02}"),
            egui::FontId::proportional(10.0),
            Color32::from_white_alpha(110),
        );
    }
}
