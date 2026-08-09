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

use eframe::egui::{self, Pos2, Stroke};
use nightlightd_core::solar::solar_elevation;
use nightlightd_core::transition::{Band, target_temperature};

use crate::daemon::Status;
use crate::theme::Palette;

/// The shortest the curve is ever drawn. Above this it takes whatever
/// height the window has spare (#46's panel pass), so enlarging the window
/// grows the picture rather than the empty strip under the footer.
const MIN_HEIGHT: f32 = 120.0;
/// Vertical breathing room so the line never touches the top or bottom edge.
const PAD: f32 = 10.0;
/// A strip along the bottom that belongs to the hour labels and to nothing
/// else. The chart used to run all the way down and the hours were painted
/// over it, so at night — when the line sits on the floor — the two were
/// drawn on top of each other.
const AXIS_HEIGHT: f32 = 15.0;
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

/// Where a plateau drag lands the daytime temperature. Clamped to the same
/// window the panel's own slider offers, so the two controls for one value
/// can never disagree about what is settable, and never below the night
/// bound — the curve is a schedule, and a schedule that runs backwards is
/// not a picture of anything.
fn held_day_temp(kelvin: f32, night_temp: u32) -> u32 {
    (kelvin.round() as u32).clamp(night_temp.max(DAY_FLOOR), 6500)
}

/// The same for the lower plateau, held under the day bound.
fn held_night_temp(kelvin: f32, day_temp: u32) -> u32 {
    (kelvin.round() as u32).clamp(1500, day_temp.min(NIGHT_CEIL))
}

/// Where a ramp drag lands a transition bound: the solar elevation under the
/// pointer, stopped before it crosses its neighbour. The pointer can be
/// anywhere on a 24-hour axis, so without this a drag past noon would invert
/// the pair and the daemon would quietly fall back to the default — the
/// screen jumping to a band the hand never asked for.
fn held_band(band: Band, elevation: f64, day: bool) -> Band {
    let mut next = band;
    if day {
        next.day_elevation = elevation.max(band.night_elevation + MIN_BAND_WIDTH);
    } else {
        next.night_elevation = elevation.min(band.day_elevation - MIN_BAND_WIDTH);
    }
    next
}

/// Everything the curve draws itself from. Gathered into one value because
/// they arrive together and mean nothing apart: the panel's live band and
/// bounds, so the shape follows a hand before the daemon has been told, and
/// the palette the whole window is wearing this second.
pub struct View<'a> {
    pub status: Option<&'a Status>,
    pub band: Band,
    pub day_temp: u32,
    pub night_temp: u32,
    /// Local midnight as a unix time, and the hour of "now" within that day.
    /// Handed down rather than read here: the panel already works both out
    /// for the schedule, and a demo run needs "now" to be a clock it drives
    /// rather than the one on the wall.
    pub midnight: f64,
    pub now_hour: f32,
    pub pal: &'a Palette,
    /// The height the caller has spare for the chart, floored at
    /// [`MIN_HEIGHT`]. Measured from what the controls needed last frame.
    pub height: f32,
}

pub fn show(ui: &mut egui::Ui, view: View<'_>, held: &mut Option<Handle>) -> Option<Edit> {
    let View {
        status,
        band,
        day_temp,
        night_temp,
        midnight,
        now_hour,
        pal,
        height,
    } = view;
    let Some(status) = status.filter(|s| s.has_location) else {
        ui.weak("Waiting for the daemon / location…");
        return None;
    };

    // Kelvin at a given local hour today, from the same maths the daemon runs.
    let kelvin_at = |hour: f32| -> f32 {
        let t = midnight + f64::from(hour) * 3600.0;
        let elevation = solar_elevation(status.latitude, status.longitude, t);
        target_temperature(elevation, band, day_temp, night_temp) as f32
    };

    let (response, painter) = ui.allocate_painter(
        egui::vec2(ui.available_width(), height.max(MIN_HEIGHT)),
        egui::Sense::click_and_drag(),
    );
    let rect = response.rect;
    // The chart proper, with the hour strip cut off the bottom.
    let plot = egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, rect.max.y - AXIS_HEIGHT));
    let plot_height = plot.height() - 2.0 * PAD;

    let to_x = |hour: f32| plot.left() + (hour / 24.0) * plot.width();
    let to_y = |kelvin: f32| {
        let frac = ((kelvin - AXIS_MIN) / (AXIS_MAX - AXIS_MIN)).clamp(0.0, 1.0);
        plot.bottom() - PAD - frac * plot_height
    };
    let to_kelvin = |y: f32| {
        let frac = ((plot.bottom() - PAD - y) / plot_height).clamp(0.0, 1.0);
        AXIS_MIN + frac * (AXIS_MAX - AXIS_MIN)
    };
    let hour_of_x = |x: f32| ((x - plot.left()) / plot.width() * 24.0).clamp(0.0, 24.0);
    // How far up the day/night span a temperature sits: 1 on the upper
    // plateau, 0 on the lower, between them on a ramp. Independent of the
    // drawing axis, so it stays the honest test for "is this a ramp".
    let night = night_temp as f32;
    let span = (day_temp as f32 - night).max(1.0);
    let frac_of = |kelvin: f32| ((kelvin - night) / span).clamp(0.0, 1.0);

    // The line's vertical extent in a column, so a near-vertical ramp is as
    // grabbable as a flat plateau.
    let hours_per_point = 24.0 / plot.width();
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
                Handle::DayLevel => Edit::DayTemp(held_day_temp(to_kelvin(p.y), night_temp)),
                Handle::NightLevel => Edit::NightTemp(held_night_temp(to_kelvin(p.y), day_temp)),
                Handle::DayBound | Handle::NightBound => {
                    let hour = f64::from(hour_of_x(p.x));
                    let elevation = solar_elevation(
                        status.latitude,
                        status.longitude,
                        midnight + hour * 3600.0,
                    );
                    Edit::Band(held_band(band, elevation, handle == Handle::DayBound))
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

    painter.rect_filled(rect, 6.0, pal.bg);

    // Warm fill under the curve, one convex trapezoid per segment so a concave
    // curve never triangulates wrong.
    let samples: Vec<(f32, f32)> = (0..=48)
        .map(|i| {
            let h = i as f32 * 0.5;
            (h, kelvin_at(h))
        })
        .collect();
    // The fill is the accent at a whisper, so it warms with the line
    // rather than staying the one orange the panel used to be painted in.
    let fill = pal.accent.gamma_multiply(0.16);
    for pair in samples.windows(2) {
        let a = egui::pos2(to_x(pair[0].0), to_y(pair[0].1));
        let b = egui::pos2(to_x(pair[1].0), to_y(pair[1].1));
        let quad = vec![
            egui::pos2(a.x, plot.bottom()),
            a,
            b,
            egui::pos2(b.x, plot.bottom()),
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
    painter.add(egui::Shape::line(line, Stroke::new(2.0, pal.accent)));

    // The part under the pointer glows — the hint, with the grab cursor, that
    // the line is a handle. Only the stretch that would actually move.
    if let Some(handle) = active {
        let glow = Stroke::new(3.5, pal.text);
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
            pal.text,
        );
    }

    // "Now": a vertical marker and a dot on the line.
    let now_x = to_x(now_hour);
    painter.line_segment(
        [
            egui::pos2(now_x, plot.top()),
            egui::pos2(now_x, plot.bottom()),
        ],
        Stroke::new(1.0, pal.faint),
    );
    painter.circle_filled(egui::pos2(now_x, to_y(kelvin_at(now_hour))), 4.0, pal.text);

    // Hour ticks, edge-aligned so 0 and 24 are not clipped.
    for h in [0, 6, 12, 18, 24] {
        let align = match h {
            0 => egui::Align2::LEFT_BOTTOM,
            24 => egui::Align2::RIGHT_BOTTOM,
            _ => egui::Align2::CENTER_BOTTOM,
        };
        painter.text(
            egui::pos2(to_x(h as f32), rect.bottom() - 2.0),
            align,
            format!("{h:02}"),
            egui::FontId::proportional(10.0),
            pal.muted,
        );
    }

    edit
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A drag runs the pointer across the whole plot, including places that
    /// would invert the band. Wherever it lands, the pair stays ordered and
    /// the daemon never sees a band it would have to repair.
    #[test]
    fn a_ramp_drag_cannot_invert_the_band() {
        let band = Band::default();
        for step in -180..=180 {
            let elevation = f64::from(step) * 0.5;
            let moved_day = held_band(band, elevation, true);
            assert!(moved_day.day_elevation > moved_day.night_elevation);
            assert_eq!(moved_day.sane(), moved_day);
            let moved_night = held_band(band, elevation, false);
            assert!(moved_night.day_elevation > moved_night.night_elevation);
            assert_eq!(moved_night.sane(), moved_night);
        }
    }

    /// Inside its own range a drag is transparent: the bound lands exactly
    /// on the elevation under the pointer, no snapping, no rounding.
    #[test]
    fn a_ramp_drag_lands_where_the_pointer_is() {
        let band = Band::default();
        assert_eq!(held_band(band, -12.0, false).night_elevation, -12.0);
        assert_eq!(held_band(band, 8.0, true).day_elevation, 8.0);
        // And the bound it was not holding stays exactly where it was.
        assert_eq!(
            held_band(band, -12.0, false).day_elevation,
            band.day_elevation
        );
    }

    /// The plateaus obey the sliders' window, so dragging the curve can
    /// never reach a value the slider beside it refuses to show.
    #[test]
    fn plateau_drags_stay_inside_the_sliders_window() {
        assert_eq!(held_day_temp(9000.0, 4500), 6500);
        assert_eq!(held_day_temp(1000.0, 4500), 4500); // never under the night bound
        assert_eq!(held_day_temp(1000.0, 1700), DAY_FLOOR);
        assert_eq!(held_night_temp(9000.0, 6500), NIGHT_CEIL);
        assert_eq!(held_night_temp(100.0, 6500), 1500);
        assert_eq!(held_night_temp(9000.0, 4200), 4200); // never over the day bound
    }
}
