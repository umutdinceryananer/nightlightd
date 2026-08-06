//! The day/night curve: colour temperature across today, like f.lux's graph.
//!
//! Sampled from the same core maths the daemon uses — solar elevation at the
//! resolved location, run through the transition curve between the configured
//! day and night bounds — so the picture is exactly what the daemon will do.
//! A vertical marker shows the current time.
//!
//! The whole line is a handle (#45). Its two plateaus are the day and night
//! temperatures and drag vertically; its two ramps are the transition band's
//! bounds and drag sideways. Nothing here talks to the daemon: a drag returns
//! a proposal and the caller stages it behind Apply and Revert.

use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Color32, Pos2, Stroke};
use nightlightd_core::solar::solar_elevation;
use nightlightd_core::transition::{Band, target_temperature};

use crate::daemon::Status;

/// Height of the curve area in points.
const HEIGHT: f32 = 130.0;
/// Vertical breathing room so the line never touches the top or bottom edge.
const PAD: f32 = 12.0;
/// The narrowest a drag can pinch the band, in degrees of elevation. A
/// drag may move a bound, never invert the pair.
const MIN_BAND_WIDTH: f64 = 0.5;
/// The vertical axis, in kelvin, fixed rather than fitted to the current
/// bounds. A self-scaling axis draws every pair of bounds as the same
/// picture, which makes the night slider look inert and leaves a plateau
/// nothing to be dragged *to*. These ends match the panel's sliders.
const AXIS_MIN: f32 = 1500.0;
const AXIS_MAX: f32 = 6500.0;
/// The plateau drags' limits, the same window the two bound sliders use, so
/// the curve and the sliders cannot disagree about what is settable.
const DAY_FLOOR: u32 = 4000;
const NIGHT_CEIL: u32 = 4500;
/// How far from the line a pointer still counts as holding it, in points.
const GRAB: f32 = 12.0;

/// What a drag on the curve took hold of (#45), chosen when the gesture
/// starts and held until it ends, so the handle never changes under the
/// hand mid-drag.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    /// The upper plateau: the daytime temperature.
    DayLevel,
    /// The lower plateau: the night temperature.
    NightLevel,
    /// A ramp's upper half: the elevation at which full day begins.
    DayBound,
    /// A ramp's lower half: the elevation at which full night begins.
    NightBound,
}

/// What a drag is proposing. One value per gesture; the caller decides when
/// any of it reaches the daemon.
pub enum Edit {
    Band(Band),
    DayTemp(u32),
    NightTemp(u32),
}

/// Draws the curve. `status` supplies the location; `band`, `day_temp` and
/// `night_temp` come from the panel's live values so the shape follows a drag
/// before the daemon has been told. Shows a placeholder when no location is
/// known (the curve is meaningless without one). `offset_secs` is the local
/// UTC offset, used to place "now" and the hour axis on local time. `held`
/// carries the current gesture's handle across frames.
pub fn show(
    ui: &mut egui::Ui,
    status: Option<&Status>,
    band: Band,
    day_temp: u32,
    night_temp: u32,
    offset_secs: i32,
    held: &mut Option<Handle>,
) -> Option<Edit> {
    let Some(status) = status.filter(|s| s.has_location) else {
        ui.weak("Waiting for the daemon / location…");
        return None;
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
        target_temperature(elevation, band, day_temp, night_temp) as f32
    };

    let (response, painter) = ui.allocate_painter(
        egui::vec2(ui.available_width(), HEIGHT),
        egui::Sense::click_and_drag(),
    );
    let rect = response.rect;
    let plot_height = rect.height() - 2.0 * PAD;

    let to_x = |hour: f32| rect.left() + (hour / 24.0) * rect.width();
    let to_y = |kelvin: f32| {
        let frac = ((kelvin - AXIS_MIN) / (AXIS_MAX - AXIS_MIN)).clamp(0.0, 1.0);
        rect.bottom() - PAD - frac * plot_height
    };
    let to_kelvin = |y: f32| {
        let frac = ((rect.bottom() - PAD - y) / plot_height).clamp(0.0, 1.0);
        AXIS_MIN + frac * (AXIS_MAX - AXIS_MIN)
    };
    let hour_of_x = |x: f32| ((x - rect.left()) / rect.width() * 24.0).clamp(0.0, 24.0);
    // How far up the day/night span a temperature sits: 1 on the upper
    // plateau, 0 on the lower, between them on a ramp. Independent of the
    // drawing axis, so it stays the honest test for "is this a ramp".
    let night = night_temp as f32;
    let span = (day_temp as f32 - night).max(1.0);
    let frac_of = |kelvin: f32| ((kelvin - night) / span).clamp(0.0, 1.0);

    // The line's vertical extent in a column, so a near-vertical ramp is as
    // grabbable as a flat plateau.
    let hours_per_point = 24.0 / rect.width();
    let extent_at = |x: f32| {
        let hour = hour_of_x(x);
        let step = hours_per_point * 4.0;
        let a = to_y(kelvin_at((hour - step).max(0.0)));
        let b = to_y(kelvin_at((hour + step).min(24.0)));
        (a.min(b), a.max(b))
    };
    let handle_at = |p: Pos2| -> Option<Handle> {
        let (top, bottom) = extent_at(p.x);
        if p.y < top - GRAB || p.y > bottom + GRAB {
            return None;
        }
        // Where the curve is decides plateau or ramp; where the hand is
        // decides which bound of a ramp, so grabbing near a corner takes
        // the bound that corner belongs to.
        match frac_of(kelvin_at(hour_of_x(p.x))) {
            f if f > 0.97 => Some(Handle::DayLevel),
            f if f < 0.03 => Some(Handle::NightLevel),
            _ if frac_of(to_kelvin(p.y)) >= 0.5 => Some(Handle::DayBound),
            _ => Some(Handle::NightBound),
        }
    };

    let hovered = response
        .interact_pointer_pos()
        .or_else(|| response.hover_pos())
        .and_then(handle_at);
    if response.drag_started() {
        *held = hovered;
    }

    let mut edit = None;
    if let Some(handle) = *held {
        if let Some(p) = response.interact_pointer_pos() {
            edit = Some(match handle {
                Handle::DayLevel => Edit::DayTemp(
                    (to_kelvin(p.y).round() as u32).clamp(night_temp.max(DAY_FLOOR), 6500),
                ),
                Handle::NightLevel => Edit::NightTemp(
                    (to_kelvin(p.y).round() as u32).clamp(1500, day_temp.min(NIGHT_CEIL)),
                ),
                Handle::DayBound | Handle::NightBound => {
                    let hour = f64::from(hour_of_x(p.x));
                    let elevation = solar_elevation(
                        status.latitude,
                        status.longitude,
                        midnight + hour * 3600.0,
                    );
                    let mut next = band;
                    if handle == Handle::DayBound {
                        next.day_elevation = elevation.max(band.night_elevation + MIN_BAND_WIDTH);
                    } else {
                        next.night_elevation = elevation.min(band.day_elevation - MIN_BAND_WIDTH);
                    }
                    Edit::Band(next)
                }
            });
        }
        if response.drag_stopped() {
            *held = None;
        }
    }
    let active = held.or(hovered);
    if active.is_some() {
        ui.ctx().set_cursor_icon(if held.is_some() {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::Grab
        });
    }

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

    // The part under the pointer glows — the hint, with the grab cursor, that
    // the line is a handle. Only the stretch that would actually move.
    if let Some(handle) = active {
        let glow = Stroke::new(3.5, Color32::from_rgb(255, 205, 130));
        let mut run: Vec<Pos2> = Vec::new();
        for i in 0..=96 {
            let h = i as f32 * 0.25;
            let kelvin = kelvin_at(h);
            let frac = frac_of(kelvin);
            let belongs = match handle {
                Handle::DayLevel => frac > 0.98,
                Handle::NightLevel => frac < 0.02,
                Handle::DayBound | Handle::NightBound => (0.02..=0.98).contains(&frac),
            };
            if belongs {
                run.push(egui::pos2(to_x(h), to_y(kelvin)));
            } else {
                if run.len() > 1 {
                    painter.add(egui::Shape::line(std::mem::take(&mut run), glow));
                }
                run.clear();
            }
        }
        if run.len() > 1 {
            painter.add(egui::Shape::line(run, glow));
        }
        // What the glowing stretch is worth, spelled out where the eye
        // already is.
        let caption = match handle {
            Handle::DayLevel => format!("{day_temp} K"),
            Handle::NightLevel => format!("{night_temp} K"),
            Handle::DayBound => format!("full day above {:+.1}°", band.day_elevation),
            Handle::NightBound => format!("full night below {:+.1}°", band.night_elevation),
        };
        painter.text(
            egui::pos2(rect.left() + 6.0, rect.top() + 4.0),
            egui::Align2::LEFT_TOP,
            caption,
            egui::FontId::proportional(11.0),
            Color32::from_rgb(255, 205, 130),
        );
    }

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

    edit
}
