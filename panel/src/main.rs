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
mod theme;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui;

use nightlightd_core::location::nearest_zone;
use nightlightd_core::schedule::{Milestone, day_length, hour_of, milestones};
use nightlightd_core::solar::solar_elevation;
use nightlightd_core::transition::{Band, phase, target_temperature};
use nightlightd_core::world::COASTLINE;

use crate::daemon::{Client, Status};

/// The manual slider's ends, in kelvin. Below ~2000 K the screen goes deep
/// orange; 6500 K is neutral (no filter); past it the screen goes bluish
/// rather than warm (#41). Neutral is therefore a place along this slider
/// now rather than its end — and it has to be, or someone holding a bluish
/// day would watch the reading say 6500 K while the screen wore 8000.
const WARMEST: u32 = nightlightd_core::color::UI_TEMPERATURE_RANGE.0;
const COOLEST: u32 = nightlightd_core::color::UI_TEMPERATURE_RANGE.1;

/// What the bounds are until the daemon says otherwise: the config's own
/// defaults, so a panel opened before the first status shows the shape most
/// people are actually running.
const DEFAULT_DAY: u32 = 6500;
const DEFAULT_NIGHT: u32 = 4500;

/// Except in a demo, which has no daemon to correct it — so what it starts
/// with is what the reel shows for its whole length.
///
/// The dashboard's demo has used a deep night since it existed. This one
/// kept the config default, which was fine while the curve's axis stopped at
/// neutral and a 6500/4500 day filled two fifths of the frame. On the fixed
/// 1500-10000 axis (#41) the same pair draws as a nearly flat line, and a
/// reel whose subject is taking hold of the curve needs a curve with a shape
/// in it. Matched to the dashboard's value so the two reels, shown side by
/// side in the README, are pictures of the same day.
const DEMO_NIGHT: u32 = 2600;

fn default_night(demo: bool) -> u32 {
    if demo { DEMO_NIGHT } else { DEFAULT_NIGHT }
}

/// Where the slider starts before the user has touched it.
const START_KELVIN: u32 = 2800;

/// The gamma slider's ends. Core accepts 0.1–10.0, but that is a safety clamp,
/// not a usable band — outside roughly this range the screen is illegible.
/// The same window the terminal dashboard offers.
const GAMMA_MIN: f64 = 0.5;
const GAMMA_MAX: f64 = 1.5;

/// The night-dim slider's ends: core's brightness floor (never black) to full.
const DIM_MIN: f64 = 0.1;
const DIM_MAX: f64 = 1.0;

/// The window's breathing room, on all four sides.
const MARGIN: f32 = 14.0;
/// The tabs, in order, and their indices. The same five the dashboard has and
/// in the same order, so the two interfaces are siblings rather than cousins.
const TABS: &[&str] = &["now", "today", "location", "outputs", "settings"];
const NOW_TAB: usize = 0;
const TODAY_TAB: usize = 1;
const LOCATION_TAB: usize = 2;
const OUTPUTS_TAB: usize = 3;
const SETTINGS_TAB: usize = 4;
/// What the links row takes, so it can be pinned to the bottom.
const FOOTER_HEIGHT: f32 = 26.0;
/// The air above and below the brand row. More than the window's own margin,
/// because the row is a header rather than the first item of a list.
const STRIP_AIR: f32 = 24.0;
/// The tab strip's height, and the air on each side of a tab's word.
const TAB_HEIGHT: f32 = 30.0;
const TAB_PADDING: f32 = 11.0;
/// The settings table's two fixed columns, so names and numbers line up down
/// the window instead of drifting with whatever text each row happens to
/// carry.
const LABEL_WIDTH: f32 = 78.0;
const READING_WIDTH: f32 = 62.0;
const ROW_HEIGHT: f32 = 20.0;
const BUTTON_HEIGHT: f32 = 26.0;
/// The theme picker, closed and open. Wide enough for the longest name with
/// room to spare, and no wider — eight short words do not want a rail across
/// the window.
const THEME_WIDTH: f32 = 130.0;
/// Where the chosen theme is remembered. Its own file, beside the daemon's
/// config but not inside it: which colours a window wears is the window's
/// business, and the dashboard keeps its choice under another name so a
/// change made here never reaches into a window that was not open.
const THEME_FILE: &str = "panel-theme";

/// One demo day, compressed — near enough the dashboard's 28 seconds that the
/// two reels read as the same day seen twice.
const DEMO_DAY_SECONDS: f64 = 30.0;

/// Where the demo stands when there is no daemon to say otherwise, and what
/// it calls the place.
const DEMO_LAT: f64 = 41.01;
const DEMO_LON: f64 = 28.98;
const DEMO_PLACE: &str = "Istanbul";

/// One step of the scripted tour. Actions rather than keystrokes: this window
/// is driven by a pointer, and a drag cannot honestly be written as a key.
#[derive(Clone, Copy)]
enum Act {
    /// Show a tab, as clicking its name would.
    Tab(usize),
    /// Hold the curve's ramp at a band, as a drag would — staged, not sent.
    Grab(f64, f64),
    /// Let the ramp go without applying, so the reel changes nothing.
    LetGo,
    /// Wear a theme.
    Theme(usize),
}

/// The tour, in seconds from the start of the run.
///
/// Deliberately not a full walk. The README's stills already show every tab
/// standing still; what a still cannot show is the day moving and a hand on
/// the curve. So the reel opens on nothing at all, takes hold of the ramp
/// early, lets go, and then leaves the sunset to play uninterrupted — the
/// day starts at noon and a 30-second day puts sunset near the eleventh
/// second, which is the one moment worth not talking over.
const DEMO_SCRIPT: &[(f64, Act)] = &[
    // The ramp, widened a step at a time from the default to -14: full night
    // lands deeper into dusk and the incline stretches visibly across the
    // chart. Staged, so Apply and Revert appear beside it.
    (4.5, Act::Grab(3.0, -7.5)),
    (5.0, Act::Grab(3.0, -9.0)),
    (5.5, Act::Grab(3.0, -10.5)),
    (6.0, Act::Grab(3.0, -12.0)),
    (6.5, Act::Grab(3.0, -13.5)),
    (7.0, Act::Grab(3.0, -14.0)),
    (9.0, Act::LetGo),
    // Sunset plays here, on the now tab, with nothing happening over it.
    (13.0, Act::Tab(TODAY_TAB)),
    (15.5, Act::Tab(LOCATION_TAB)),
    (18.0, Act::Tab(OUTPUTS_TAB)),
    // Back to the curve for the last third, so the palettes change over the
    // one picture that shows what a palette does — and dawn lands under them.
    (19.5, Act::Tab(NOW_TAB)),
    (22.0, Act::Theme(3)),
    (25.0, Act::Theme(0)),
];
/// The map's viewport. Antarctica is cropped away — nobody runs a night light
/// there — and the north is carried past Greenland.
const MAP_LAT_MIN: f64 = -60.0;
const MAP_LAT_MAX: f64 = 80.0;
/// What the caption row under the map needs, and the map's own floor.
const MAP_RESERVE: f32 = 34.0;
const MIN_MAP_HEIGHT: f32 = 130.0;
/// The shortest the chart is ever drawn; below this the day is unreadable.
const MIN_CURVE_HEIGHT: f32 = 110.0;

/// Project links for the footer.
const REPO_URL: &str = "https://github.com/umutdinceryananer/nightlightd";
const ISSUES_URL: &str = "https://github.com/umutdinceryananer/nightlightd/issues";

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
    /// The day/night values when the panel opened, for "Revert changes".
    orig_day: u32,
    orig_night: u32,
    /// The ramp-shaping knobs (GitHub #2): the gamma exponent and the night
    /// brightness factor, with the same opened-with/last-reported bookkeeping
    /// as the temperature bounds.
    gamma: f64,
    night_dim: f64,
    orig_gamma: f64,
    orig_night_dim: f64,
    start_at_login: bool,
    offset_secs: i32,
    /// Set by the single-instance `Present` call; the loop clears it and raises
    /// the window.
    focus: Arc<AtomicBool>,
    /// The last daemon snapshot and when it was taken. egui redraws every frame
    /// while interacting; polling on a timer keeps that from becoming a
    /// blocking D-Bus round trip per frame. `None` forces a fresh read.
    status: Option<Status>,
    /// The fade switch (#44), read through the additive `GetFade`; `None`
    /// against a daemon that is unreachable or too old to answer, and then
    /// the checkbox does not show. Not part of "Revert changes" — it is a
    /// behaviour switch, not a session's slider fiddling.
    fade: Option<bool>,
    /// Status unreadable but the daemon's name owned (#42): different
    /// versions, which deserves a different notice than silence.
    mismatch: bool,
    /// The transition band (#39) as the daemon last reported it, so the
    /// curve matches what the screen actually does; the default against a
    /// daemon that cannot answer.
    band: Band,
    /// A band proposed by dragging the curve's ramp (#45), not yet sent.
    /// The curve draws this while it exists; Apply sends it, Revert drops
    /// it and the curve snaps back to the daemon's band.
    staged_band: Option<Band>,
    /// Whether a plateau drag has moved the temperature bounds without
    /// sending them. The proposed values live in `day_temp`/`night_temp`
    /// like any slider edit, so the sliders mirror the drag; this only
    /// records that Apply has something to send and Revert something to
    /// put back.
    staged_temps: bool,
    /// Which part of the curve the current drag holds, carried across frames.
    curve_held: Option<curve::Handle>,
    last_poll: Option<Instant>,
    /// The day/night bounds as the daemon last reported them, plus whether a
    /// bound slider is mid-drag — so a change made elsewhere (another client, a
    /// daemon restart with a different config) is adopted instead of the panel
    /// showing stale sliders forever, without stomping an in-progress drag.
    daemon_day: u32,
    daemon_night: u32,
    daemon_gamma: f64,
    daemon_night_dim: f64,
    bounds_dragging: bool,
    /// The city the daemon's coordinates land in, with the coordinates it was
    /// resolved for. Looked up only when those move, because it reads the
    /// system's zone table off disk — and shown at all because the panel
    /// otherwise never says where it thinks you are, which is the one thing
    /// this project exists to get right.
    place: Option<(f64, f64, String)>,
    /// Which of the shared themes the window is wearing. Index rather than
    /// name because that is what the picker walks; the name is what gets
    /// written to disk, since indices are a release away from meaning
    /// something else.
    theme_index: usize,
    /// The demo clock, set by `--demo` and otherwise absent. Its presence is
    /// the only thing that makes this window synthesise a day rather than
    /// read one, and it is what mutes every write.
    demo: Option<Instant>,
    /// How far through the scripted tour we have got.
    demo_cursor: usize,
    /// The screens the daemon last wrote a ramp to, polled with the status.
    /// `None` is "could not ask", `Some(empty)` is "asked, nothing yet".
    outputs: Option<Vec<(u32, u16)>>,
    /// Which tab is showing.
    tab: usize,
    /// Which point of the world sits in the middle of the map, as fractions
    /// of the world's own width and height — `(0.5, 0.5)` is the map's own
    /// centre, where it starts. Held here rather than recomputed because it
    /// is the one thing about the map the user owns.
    map_center: egui::Vec2,
    /// How tall the chart was drawn last frame. Not computed from what the
    /// controls need — that arithmetic has to know every card's padding and
    /// every row that comes and goes, and forgetting one pushes the footer
    /// off the bottom of a window that cannot scroll. Instead the frame is
    /// laid out, the overflow is measured, and the chart gives exactly that
    /// back. Converges in one frame, and cannot be wrong about chrome it
    /// does not know about.
    curve_height: f32,
}

impl Panel {
    /// Sends a night-brightness change. The day bound rides along unchanged —
    /// daemon-owned state no panel widget edits — so it is read fresh here
    /// rather than echoed from the up-to-a-second-old cache, and when even a
    /// fresh read fails the send is skipped entirely: inventing a day bound
    /// of 1.0 would persist a value the user never chose.
    fn send_night_dim(&mut self, night: f64) {
        if let Some(fresh) = self.client.status() {
            self.client.set_brightness(fresh.day_brightness, night);
        }
        self.last_poll = None;
    }

    /// Where the daemon thinks it is, and what time it is there. Empty until
    /// a location is resolved, so the strip stays quiet rather than claiming
    /// a city it has not found.
    fn where_and_when(&self) -> String {
        let clock = {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let secs = (now as i64 + i64::from(self.offset_secs)).rem_euclid(86_400);
            format!("{:02}:{:02}", secs / 3600, (secs % 3600) / 60)
        };
        match self.place.as_ref() {
            Some((_, _, city)) => format!("{city} · {clock}"),
            None => clock,
        }
    }

    /// The state card's three pieces: the headline anyone opens the window
    /// for, the mode it is in, and a quieter line of why. The same words and
    /// the same precedence the tray's readout uses — off outranks everything,
    /// manual outranks the sun — and the phase comes from the band the daemon
    /// actually runs, never from a hardcoded pair.
    fn state_lines(&self) -> (String, String, String, Option<&'static str>) {
        let place = |s: &Status| {
            if s.has_location {
                phase(s.elevation, self.band).to_string()
            } else {
                "no location".to_string()
            }
        };
        match &self.status {
            Some(s) if !s.enabled => (
                format!("{} K", s.temperature),
                "OFF".into(),
                format!("the filter is off · {}", self.where_and_when()),
                None,
            ),
            Some(s) if !s.following => (
                format!("{} K", s.temperature),
                "MANUAL".into(),
                format!("held by hand · {}", self.where_and_when()),
                None,
            ),
            Some(s) => (
                format!("{} K", s.temperature),
                "AUTO".into(),
                format!("{} · {}", place(s), self.where_and_when()),
                // The little sky only appears where the sun is what is
                // driving the screen; beside "manual" it would be a picture
                // of something that is not happening.
                s.has_location.then(|| phase(s.elevation, self.band)),
            ),
            None if self.mismatch => (
                "update needed".into(),
                "".into(),
                "this panel and the daemon are different versions".into(),
                None,
            ),
            None => (
                "not running".into(),
                "".into(),
                "the daemon is not running, so nothing reaches the screen".into(),
                None,
            ),
        }
    }

    /// Everything the window draws, from the state card down to the links.
    /// Split out so the caller can put it inside a scroll area — a closure
    /// this long inline is unreadable, and the borrow checker is happier with
    /// one `&mut self` than with a capture.
    /// The window, in order: the chrome that is true everywhere, the state
    /// nobody should have to hunt for, the tabs, and the links. Only the tab
    /// content changes; the rest is the frame around it.
    fn body(&mut self, ui: &mut egui::Ui, pal: &theme::Palette, status: Option<Status>) {
        self.tab_keys(ui);
        self.top_strip(ui, pal);
        self.state_card(ui, pal);
        self.tab_bar(ui, pal);
        match self.tab {
            TODAY_TAB => self.today_tab(ui, pal),
            LOCATION_TAB => self.location_tab(ui, pal),
            OUTPUTS_TAB => self.outputs_tab(ui, pal),
            SETTINGS_TAB => self.settings_tab(ui, pal),
            _ => self.now_tab(ui, pal, status),
        }
        // The links sit at the bottom of the window rather than under the
        // content, so they do not wander up and down as tabs change height.
        let footer = ui.available_height() - FOOTER_HEIGHT;
        if footer > 0.0 {
            ui.add_space(footer);
        }
        self.footer(ui, pal);

        // How far the content ran past the window, or short of it. The chart
        // absorbs the difference next frame, which is one frame of lag on a
        // resize and nobody has ever seen it. Only while the chart is on
        // screen: on the other tabs there is nothing elastic to give.
        if self.tab == NOW_TAB {
            let slack = ui.max_rect().bottom() - ui.cursor().top();
            self.curve_height = (self.curve_height + slack).max(MIN_CURVE_HEIGHT);
        }
    }

    /// Tab and shift-tab walk the strip, as they do in the dashboard. The key
    /// is consumed so egui does not also move its own focus with it — and
    /// left alone entirely while a value field is being typed into, where
    /// tabbing out of the field is what tab is for.
    fn tab_keys(&mut self, ui: &mut egui::Ui) {
        // Only a widget being typed into may swallow the key. Both earlier
        // guards here were really "is anything focused at all" —
        // `egui_wants_keyboard_input` is that check under another name — and
        // egui focuses a widget the moment you press tab, so the second press
        // always found the door shut and stayed shut until a click cleared
        // the focus. `text_edit_focused` asks the question that was meant.
        if ui.ctx().text_edit_focused() {
            return;
        }
        let (forward, back) = ui.input_mut(|input| {
            (
                input.consume_key(egui::Modifiers::NONE, egui::Key::Tab),
                input.consume_key(egui::Modifiers::SHIFT, egui::Key::Tab),
            )
        });
        if !forward && !back {
            return;
        }
        if forward {
            self.tab = (self.tab + 1) % TABS.len();
        } else {
            self.tab = (self.tab + TABS.len() - 1) % TABS.len();
        }
    }

    /// Parks the keyboard on the tab strip and tells egui that this widget
    /// wants the tab key for itself.
    ///
    /// Consuming the event was never enough. egui reads Tab in
    /// `Memory::begin_pass`, before any application code runs, and applies
    /// the focus move while the widgets are being laid out — so the button
    /// beside the brand lit up with a focus ring for exactly one frame, over
    /// and over, which is the flicker. Handing the focus back afterwards is
    /// always a frame late. A focus lock is read *before* that queue is
    /// filled, so the move never happens at all.
    ///
    /// Skipped while a value field is being typed into, where tab belongs to
    /// the field.
    fn park_focus(&self, ui: &mut egui::Ui, id: egui::Id) {
        if ui.ctx().text_edit_focused() {
            return;
        }
        ui.memory_mut(|memory| {
            memory.request_focus(id);
            memory.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    tab: true,
                    ..Default::default()
                },
            );
        });
    }

    /// The tab strip: a rule across the window with the open tab's segment
    /// lit in the accent. Painted by hand rather than built from
    /// `selectable_label`, which fills the selected item with the selection
    /// colour — the accent, here — and left accent text on an accent ground.
    /// An underline says the same thing without a box, and the rule under
    /// the whole strip is what stops the row drifting loose between the state
    /// card above it and the content below.
    fn tab_bar(&mut self, ui: &mut egui::Ui, pal: &theme::Palette) {
        let font = egui::FontId::proportional(12.5);
        let (bar, bar_response) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), TAB_HEIGHT),
            egui::Sense::click(),
        );
        self.park_focus(ui, bar_response.id);
        let painter = ui.painter().clone();
        painter.hline(
            bar.x_range(),
            bar.bottom() - 1.0,
            egui::Stroke::new(1.0, pal.raised),
        );

        let mut left = bar.left();
        for (index, name) in TABS.iter().enumerate() {
            let text_width = painter
                .layout_no_wrap((*name).into(), font.clone(), egui::Color32::WHITE)
                .size()
                .x;
            let slot = egui::Rect::from_min_size(
                egui::pos2(left, bar.top()),
                egui::vec2(text_width + TAB_PADDING * 2.0, bar.height()),
            );
            left = slot.right();
            let response = ui.interact(slot, ui.id().with(("tab", index)), egui::Sense::click());
            if response.clicked() {
                self.tab = index;
            }
            let selected = self.tab == index;
            let colour = if selected || response.hovered() {
                pal.text
            } else {
                pal.muted
            };
            painter.text(
                slot.center() - egui::vec2(0.0, 1.0),
                egui::Align2::CENTER_CENTER,
                *name,
                font.clone(),
                colour,
            );
            if selected {
                // Sitting on the rule rather than above it, so the strip
                // reads as one line with a lit stretch.
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(slot.left() + 4.0, bar.bottom() - 2.0),
                        egui::vec2(slot.width() - 8.0, 2.0),
                    ),
                    1.0,
                    pal.accent,
                );
            }
        }
        ui.add_space(10.0);
    }

    /// Tab 2: the day's milestones — the same schedule the curve draws as a
    /// shape, written out as times and temperatures. Every row is a crossing
    /// the sun actually makes at this location today, computed by the core
    /// the daemon schedules on, so the table cannot drift from the curve
    /// beside it or from the screen.
    fn today_tab(&mut self, ui: &mut egui::Ui, pal: &theme::Palette) {
        let Some(status) = self.status.clone().filter(|s| s.has_location) else {
            card(ui, pal.surface, |ui| {
                ui.label(
                    egui::RichText::new(if self.status.is_some() {
                        "No location yet, so there is no schedule to derive."
                    } else {
                        "The daemon is not running, so there is no schedule to read."
                    })
                    .color(pal.muted),
                );
            });
            return;
        };
        let (midnight, now_hour) = self.day_context();
        let events = milestones(
            status.latitude,
            status.longitude,
            midnight,
            self.band,
            status.day_temp,
            status.night_temp,
        );
        card(ui, pal.surface, |ui| {
            for event in &events {
                // The next thing to happen is the one worth finding at a
                // glance; everything else is reference.
                let next = events
                    .iter()
                    .find(|e| e.hour > now_hour)
                    .is_some_and(|e| std::ptr::eq(e, event));
                let name = if next {
                    egui::RichText::new(event.name).color(pal.text).strong()
                } else {
                    egui::RichText::new(event.name).color(pal.muted)
                };
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(LABEL_WIDTH + 20.0, ROW_HEIGHT),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.label(name);
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(48.0, ROW_HEIGHT),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.label(
                                egui::RichText::new(event.hhmm())
                                    .monospace()
                                    .color(if next { pal.text } else { pal.muted }),
                            );
                        },
                    );
                    // The temperature in the colour it will be: the table
                    // reads as the same day the curve above it draws.
                    ui.allocate_ui_with_layout(
                        egui::vec2(62.0, ROW_HEIGHT),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.label(
                                egui::RichText::new(format!("{} K", event.kelvin))
                                    .monospace()
                                    .color(theme::display_tint(event.kelvin)),
                            );
                        },
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(relative(event.hour - now_hour))
                                .size(11.0)
                                .color(pal.faint),
                        );
                    });
                });
            }
            if events.is_empty() {
                ui.label(
                    egui::RichText::new("Polar day or night: the sun crosses nothing today.")
                        .color(pal.muted),
                );
            }
        });
        self.sun_card(ui, pal, &status, &events, midnight);
    }

    /// The day summarised, under the table that lists it.
    ///
    /// Deliberately not the table's rows again. The dashboard says this in a
    /// popup, where repeating sunrise and sunset is harmless because the table
    /// is not on screen; here the two sit an inch apart, and a card that
    /// echoes the one above it reads as padding. So this carries only what the
    /// table cannot: how long the day is, how that compares with yesterday,
    /// how high the sun gets, and when tomorrow starts.
    fn sun_card(
        &mut self,
        ui: &mut egui::Ui,
        pal: &theme::Palette,
        status: &Status,
        events: &[Milestone],
        midnight: f64,
    ) {
        let neighbour = |offset_days: f64| {
            milestones(
                status.latitude,
                status.longitude,
                midnight + offset_days * 86_400.0,
                self.band,
                status.day_temp,
                status.night_temp,
            )
        };
        card(ui, pal.surface, |ui| {
            ui.horizontal(|ui| {
                match day_length(events) {
                    Some(length) => {
                        ui.label(
                            egui::RichText::new(format!(
                                "{} of daylight",
                                hours_and_minutes(length)
                            ))
                            .size(15.0)
                            .strong()
                            .color(pal.text),
                        );
                        // Lengthening days take the warm accent and shortening
                        // ones the cool second hue, so which way the year is
                        // going is legible before the words are read.
                        if let Some(before) = day_length(&neighbour(-1.0)) {
                            let delta = length - before;
                            ui.add_space(2.0);
                            badge(
                                ui,
                                pal,
                                "",
                                &length_change(delta),
                                if delta >= 0.0 {
                                    pal.accent
                                } else {
                                    pal.accent2
                                },
                            );
                        }
                    }
                    // No crossings at all: say which of the two silences it is
                    // rather than leaving the card blank. The elevation is the
                    // only thing that can tell them apart.
                    None => {
                        ui.label(
                            egui::RichText::new(if status.elevation > 0.0 {
                                "The sun does not set today"
                            } else {
                                "The sun does not rise today"
                            })
                            .size(15.0)
                            .strong()
                            .color(pal.text),
                        );
                    }
                }
            });
            ui.add_space(6.0);
            // Wrapped, because four pills do not fit a narrow window on one
            // line and a clipped badge is worse than a second row.
            ui.horizontal_wrapped(|ui| {
                if let Some(noon) = hour_of(events, "solar noon") {
                    let elevation = solar_elevation(
                        status.latitude,
                        status.longitude,
                        midnight + noon * 3600.0,
                    );
                    badge(ui, pal, "noon", &format!("{elevation:+.1}°"), pal.accent2);
                }
                // Tomorrow's sunrise with the shift from today's, which is the
                // same fact as the day-length delta from the other end: it is
                // the one people set an alarm by.
                if let (Some(today_rise), Some(tomorrow)) = (
                    hour_of(events, "sunrise"),
                    neighbour(1.0).into_iter().find(|e| e.name == "sunrise"),
                ) {
                    let minutes = ((tomorrow.hour - today_rise) * 60.0).round() as i64;
                    badge(
                        ui,
                        pal,
                        "tomorrow",
                        &format!("{} ({minutes:+}m)", tomorrow.hhmm()),
                        pal.accent2,
                    );
                }
                badge(
                    ui,
                    pal,
                    "sun now",
                    &format!("{:+.1}°", status.elevation),
                    pal.accent2,
                );
            });
        });
    }

    /// Local midnight as a unix time, and the fractional hour of now — the
    /// two numbers every solar calculation here starts from.
    ///
    /// The hour is the one thing a demo run overrides. Everything that says
    /// "now" — the curve's marker, the schedule's next event, the relative
    /// times — reads it from here, so driving a compressed day is a matter of
    /// answering this question differently rather than of teaching each of
    /// them about the demo.
    fn day_context(&self) -> (f64, f64) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let secs_into_day = (now as i64 + i64::from(self.offset_secs)).rem_euclid(86_400) as f64;
        let hour = self.demo_hour().unwrap_or(secs_into_day / 3600.0);
        (now - secs_into_day, hour)
    }

    /// Where the demo's compressed day has got to, or [`None`] in a normal
    /// run. Starts at noon so a recording opens in daylight and walks into
    /// the evening, which is the half worth watching.
    fn demo_hour(&self) -> Option<f64> {
        self.demo
            .map(|start| (12.0 + start.elapsed().as_secs_f64() / DEMO_DAY_SECONDS * 24.0) % 24.0)
    }

    /// Stands in for the daemon during a demo run.
    ///
    /// The one thing this does *not* do is fake the picture: the elevation
    /// comes from `core::solar` at a real place and the temperature from
    /// `core::transition` through the band on screen, so a reel shows the
    /// arithmetic this program actually runs, sped up. An animation that
    /// merely looked like a sunset would be a lie told in the shop window.
    fn apply_demo(&mut self) {
        let Some(hour) = self.demo_hour() else {
            return;
        };
        // Answers the daemon would give, so no tab has to know it is in a
        // demo: the fade switch is on, nothing is out of date, and there are
        // two screens to list.
        self.fade = Some(true);
        self.mismatch = false;
        self.outputs = Some(vec![(78, 1024), (79, 1024)]);
        self.place = Some((DEMO_LAT, DEMO_LON, DEMO_PLACE.to_owned()));

        let (midnight, _) = self.day_context();
        let elevation = solar_elevation(DEMO_LAT, DEMO_LON, midnight + hour * 3600.0);
        // The band the window is showing, staged drag and all, so the curve
        // and the state card agree while the tour is dragging a ramp.
        let band = self.staged_band.unwrap_or(self.band);
        self.status = Some(Status {
            enabled: true,
            temperature: target_temperature(elevation, band, self.day_temp, self.night_temp),
            source: "demo".into(),
            elevation,
            has_location: true,
            latitude: DEMO_LAT,
            longitude: DEMO_LON,
            following: true,
            day_temp: self.day_temp,
            night_temp: self.night_temp,
            gamma: self.gamma,
            brightness: 1.0,
            day_brightness: 1.0,
            night_brightness: self.night_dim,
        });
    }

    /// Plays the scripted tour, one action per due entry.
    ///
    /// Actions rather than keystrokes, unlike the dashboard's reel: this
    /// window is driven by a pointer, and there is no honest way to script a
    /// drag as a keypress. What each action changes is exactly what a hand
    /// would have changed — the same staged band, the same tab index — so
    /// nothing here is a path the user cannot reach.
    fn run_demo_script(&mut self) {
        let Some(start) = self.demo else {
            return;
        };
        let elapsed = start.elapsed().as_secs_f64();
        while self.demo_cursor < DEMO_SCRIPT.len() {
            let (at, action) = DEMO_SCRIPT[self.demo_cursor];
            if elapsed < at {
                break;
            }
            match action {
                Act::Tab(index) => self.tab = index,
                Act::Grab(day, night) => {
                    self.staged_band = Some(Band {
                        day_elevation: day,
                        night_elevation: night,
                    });
                }
                // Let go without applying: the reel shows the gesture and the
                // guard, and leaves the recorder's own settings alone.
                Act::LetGo => self.staged_band = None,
                Act::Theme(index) => self.theme_index = index,
            }
            self.demo_cursor += 1;
        }
    }

    /// Tab 3: where the daemon thinks it is, and how to tell it otherwise.
    ///
    /// The map is coastline from `core::world`, a table committed to this
    /// repository rather than a dataset fetched at startup — a night light
    /// that phones out to learn where the shorelines are has the same defect
    /// as one that phones out to learn where *you* are, and the second is the
    /// entire reason this project exists. Clicking takes the coordinate under
    /// the pointer rather than snapping to any known place: the daemon
    /// accepts any pair, and a degree of error is a few minutes of sunset
    /// nobody notices. The name under the pointer is a caption, not a choice.
    fn location_tab(&mut self, ui: &mut egui::Ui, pal: &theme::Palette) {
        let known = self.status.as_ref().filter(|s| s.has_location).cloned();
        card(ui, pal.surface, |ui| match (&known, self.place.as_ref()) {
            (Some(status), place) => {
                ui.label(
                    egui::RichText::new(place.map_or("resolved", |(_, _, city)| city.as_str()))
                        .size(16.0)
                        .strong()
                        .color(pal.text),
                );
                ui.label(
                    egui::RichText::new(format_coords(status.latitude, status.longitude))
                        .size(11.0)
                        .color(pal.muted),
                );
            }
            (None, _) => {
                ui.label(
                    egui::RichText::new("No location")
                        .size(16.0)
                        .strong()
                        .color(pal.text),
                );
                ui.label(
                    egui::RichText::new(
                        "Without one there is no sunset to compute and nothing to follow.",
                    )
                    .size(11.0)
                    .color(pal.muted),
                );
            }
        });

        let mut pin = None;
        let mut clear = false;
        let mut center = self.map_center;
        card(ui, pal.surface, |ui| {
            // The map takes every point the card can spare, in both
            // directions, and the world is scaled to cover it rather than to
            // fit inside it. Fitting meant the frame's shape and the world's
            // had to agree, and when they disagreed the difference came out
            // as empty card — a band down one side, or a squat letterbox.
            let aspect =
                (std::f64::consts::TAU / (mercator(MAP_LAT_MAX) - mercator(MAP_LAT_MIN))) as f32;
            let viewport = egui::vec2(
                ui.available_width(),
                (ui.available_height() - MAP_RESERVE).max(MIN_MAP_HEIGHT),
            );
            let (rect, response) = ui.allocate_exact_size(viewport, egui::Sense::click_and_drag());
            let drag = if response.dragged() {
                response.drag_delta()
            } else {
                egui::Vec2::ZERO
            };
            let world;
            (world, center) = map_crop(rect.size(), aspect, center, drag);
            if response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            } else if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
            }

            // Everything past the frame's edge is cropped, not folded onto
            // it: a coastline clamped to the border would draw a shoreline
            // that is not there.
            let painter = ui.painter().with_clip_rect(rect);
            painter.rect_filled(rect, 6.0, pal.bg);

            let (top, bottom) = (mercator(MAP_LAT_MAX), mercator(MAP_LAT_MIN));
            let middle = rect.center();
            let to_screen = |lat: f64, lon: f64| {
                let u = ((lon + 180.0) / 360.0) as f32;
                let v = ((top - mercator(lat)) / (top - bottom)) as f32;
                egui::pos2(
                    middle.x + (u - center.x) * world.x,
                    middle.y + (v - center.y) * world.y,
                )
            };
            let to_world = |point: egui::Pos2| {
                let u = f64::from((center.x + (point.x - middle.x) / world.x).clamp(0.0, 1.0));
                let v = f64::from((center.y + (point.y - middle.y) / world.y).clamp(0.0, 1.0));
                (unmercator(top - v * (top - bottom)), u * 360.0 - 180.0)
            };

            // The coastlines, from the table in core. Each run is its own
            // stroke, and a run that leaves the cropped viewport is cut at
            // the edge rather than clamped — a clamped point would fold the
            // line back across the map as a false shoreline.
            let coast = egui::Stroke::new(1.0, pal.muted);
            for run in COASTLINE {
                let mut visible: Vec<egui::Pos2> = Vec::new();
                for &(longitude, latitude) in run.iter() {
                    let (lat, lon) = (f64::from(latitude), f64::from(longitude));
                    if (MAP_LAT_MIN..=MAP_LAT_MAX).contains(&lat) {
                        visible.push(to_screen(lat, lon));
                    } else if visible.len() > 1 {
                        painter.add(egui::Shape::line(std::mem::take(&mut visible), coast));
                    } else {
                        visible.clear();
                    }
                }
                if visible.len() > 1 {
                    painter.add(egui::Shape::line(visible, coast));
                }
            }

            // The crosshair: it follows the pointer over the map and comes to
            // rest on the pinned place when the pointer is elsewhere. One
            // pair of lines doing both jobs, rather than a fixed equator and
            // meridian that only ever pointed at the middle of the Atlantic.
            let hovered = response
                .hover_pos()
                .filter(|point| rect.contains(*point))
                .map(to_world);
            let aim = hovered.or_else(|| {
                known
                    .as_ref()
                    .map(|status| (status.latitude, status.longitude))
            });
            if let Some((lat, lon)) = aim {
                let point = to_screen(lat, lon);
                let hair = egui::Stroke::new(
                    1.0,
                    if hovered.is_some() {
                        pal.muted
                    } else {
                        pal.faint
                    },
                );
                painter.hline(rect.x_range(), point.y, hair);
                painter.vline(point.x, rect.y_range(), hair);
                painter.circle_stroke(point, 4.0, egui::Stroke::new(1.0, pal.text));
            }
            // Where the daemon actually is, whatever the pointer is doing.
            if let Some(status) = &known {
                let here = to_screen(status.latitude, status.longitude);
                painter.circle_filled(here, 4.0, pal.accent);
            }
            if response.clicked()
                && let Some(where_to) = response
                    .interact_pointer_pos()
                    .filter(|point| rect.contains(*point))
                    .map(to_world)
            {
                pin = Some(where_to);
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                // What the pointer is over, named — so a click is never a
                // guess about which country you just landed in.
                let caption = match hovered {
                    Some((lat, lon)) => nearest_zone(lat, lon).map_or_else(
                        || format_coords(lat, lon),
                        |(zone, _, _)| format!("{}  ·  {}", zone, format_coords(lat, lon)),
                    ),
                    None => "Drag to move the world · click to pin a place".to_string(),
                };
                ui.label(egui::RichText::new(caption).size(11.0).color(pal.muted));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Use the timezone").clicked() {
                        clear = true;
                    }
                });
            });
        });
        self.map_center = center;
        if let Some((lat, lon)) = pin {
            self.client.set_location(lat, lon);
            self.last_poll = None;
        }
        if clear {
            self.client.clear_location();
            self.last_poll = None;
        }
    }

    /// Tab 4: every screen the ramp is actually reaching.
    ///
    /// The daemon knows a CRTC number and a ramp size, and that is all it is
    /// told — the connector name on the back of the monitor belongs to a
    /// different X extension query, and asking for it is part of #34 rather
    /// than a free extra. So the table says CRTC, which is at least true. The
    /// applied temperature is repeated on every row on purpose: it is the
    /// same number for all of them today, and seeing it repeat is how the
    /// limitation reads as a fact rather than as an omission.
    fn outputs_tab(&mut self, ui: &mut egui::Ui, pal: &theme::Palette) {
        let applied = self.status.as_ref().map(|s| s.temperature);
        let Some(outputs) = self.outputs.as_ref().filter(|list| !list.is_empty()) else {
            card(ui, pal.surface, |ui| {
                ui.label(
                    egui::RichText::new(if self.outputs.is_some() {
                        "No screen has been written to yet."
                    } else {
                        "The daemon cannot be reached, so nothing is being written."
                    })
                    .color(pal.muted),
                );
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(
                        "A ramp is written on the first apply; outputs appear then.",
                    )
                    .size(11.0)
                    .color(pal.faint),
                );
            });
            return;
        };

        card(ui, pal.surface, |ui| {
            let row = |ui: &mut egui::Ui, name: egui::RichText, steps, kelvin| {
                ui.horizontal(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(LABEL_WIDTH + 20.0, ROW_HEIGHT),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.label(name);
                        },
                    );
                    for (text, width) in [(steps, 76.0), (kelvin, READING_WIDTH)] {
                        ui.allocate_ui_with_layout(
                            egui::vec2(width, ROW_HEIGHT),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.label(text);
                            },
                        );
                    }
                });
            };
            let heading = |text: &str| egui::RichText::new(text).size(11.0).color(pal.faint);
            row(
                ui,
                heading("output"),
                heading("gamma ramp"),
                heading("applied"),
            );
            for (crtc, steps) in outputs {
                row(
                    ui,
                    egui::RichText::new(format!("CRTC {crtc}")).color(pal.text),
                    egui::RichText::new(format!("{steps} steps"))
                        .monospace()
                        .color(pal.accent2),
                    match applied {
                        // The same tint the schedule uses, so a temperature
                        // looks like a temperature wherever it is printed.
                        Some(kelvin) => egui::RichText::new(format!("{kelvin} K"))
                            .monospace()
                            .color(theme::display_tint(kelvin)),
                        None => egui::RichText::new("—").monospace().color(pal.faint),
                    },
                );
            }
        });

        card(ui, pal.surface, |ui| {
            ui.label(
                egui::RichText::new(format!(
                    "{} wearing the same temperature.",
                    match outputs.len() {
                        1 => "One screen,".to_string(),
                        n => format!("{n} screens, all"),
                    }
                ))
                .color(pal.muted),
            );
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new("Per-screen control is #34, and this tab is where it lands.")
                    .size(11.0)
                    .color(pal.faint),
            );
        });
    }

    /// The window's thinnest layer: the name, and where and when the
    /// daemon thinks it is. Persistent, because it is true on every tab.
    fn top_strip(&mut self, ui: &mut egui::Ui, pal: &theme::Palette) {
        // The window's margin is already above; this is the rest of the air,
        // and the same amount goes underneath, so the row sits centred
        // between the top of the window and the card below it.
        ui.add_space(STRIP_AIR - MARGIN);
        let (mut power, mut restart, mut start) = (None, false, false);
        // A thin line of chrome. The name is small on purpose: the window is
        // already titled, and the loudest thing here should be what the
        // screen is doing, not what the program is called.
        ui.horizontal(|ui| {
            // The dashboard's wordmark, brought over rather than reinvented:
            // a moon, then the name in two tones. It reads as a mark without
            // the six figlet rows the panel used to carry, and at eleven grey
            // points the name had faded into the chrome beside a lit button.
            ui.add_space(2.0);
            sky_mark(ui, pal, pal.bg, "night");
            ui.add_space(6.0);
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label(
                egui::RichText::new("night")
                    .color(pal.text)
                    .size(14.0)
                    .strong(),
            );
            ui.label(
                egui::RichText::new("lightd")
                    .color(pal.accent)
                    .size(14.0)
                    .strong(),
            );
            // The one action that is true on every tab belongs in the one
            // strip that is on every tab. Turning the filter off is what
            // people reach for most; it should not be somewhere you navigate
            // to. The place and the clock moved down to the state card, where
            // they read as context for the phase rather than as chrome.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                match self.status.as_ref().map(|s| s.enabled) {
                    Some(on) => {
                        // The label promises a direction, so send that
                        // direction rather than a blind toggle: against a
                        // status gone stale a toggle would do the opposite of
                        // what the button said.
                        let label = if on { "Turn off" } else { "Turn on" };
                        if ui.button(label).clicked() {
                            power = Some(!on);
                        }
                    }
                    // No daemon to talk to. Either it is stale (#42) and a
                    // restart heals it, or it is simply not running (#43) and
                    // a client can at least ask for one.
                    None if self.mismatch => {
                        if ui.button("Restart the daemon").clicked() {
                            restart = true;
                        }
                    }
                    None => {
                        if ui.button("Start the daemon").clicked() {
                            start = true;
                        }
                    }
                }
            });
        });
        if let Some(on) = power {
            self.client.set_enabled(on);
            self.last_poll = None;
        }
        if restart {
            self.spawn_daemon(&["--user", "restart", "nightlightd"], false);
        }
        if start {
            self.spawn_daemon(&["--user", "start", "nightlightd"], true);
        }
        // Below the row, egui has already put a row's worth of spacing in, so
        // only the difference is added.
        ui.add_space(STRIP_AIR - ui.spacing().item_spacing.y);
    }

    /// What the screen is doing, above the tabs rather than inside one:
    /// whichever tab you are on, this is the question you came with.
    fn state_card(&mut self, ui: &mut egui::Ui, pal: &theme::Palette) {
        // The state card. The temperature is the headline because it is the
        // answer to the only question anyone opens this window with, and the
        // card is washed in that temperature's own colour so the block reads
        // as the thing it is announcing. The panel used to say this in nine
        // grey points in a corner, if at all.
        let (headline, mode, detail, sky) = self.state_lines();
        // A card with no daemon behind it is a notice, not a reading, and it
        // should not be wearing a temperature nothing is applying. The warn
        // ground is what makes "update needed" read as something to act on
        // rather than as another line of status.
        let ground = if self.status.is_some() {
            pal.hero
        } else {
            pal.warn_ground
        };
        // Big numbers, smaller words: "update needed" has to fit the card.
        let headline_size = if self.status.is_some() { 34.0 } else { 20.0 };
        card(ui, ground, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new(headline)
                        .size(headline_size)
                        .strong()
                        .color(pal.text),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        egui::RichText::new(mode)
                            .size(12.0)
                            .strong()
                            .color(pal.accent2),
                    );
                });
            });
            ui.horizontal(|ui| {
                if let Some(phase) = sky {
                    sky_mark(ui, pal, ground, phase);
                    ui.add_space(5.0);
                }
                ui.label(egui::RichText::new(detail).size(12.0).color(pal.muted));
            });
        });
    }

    /// Asks systemd for the daemon, falling back to the binary beside this
    /// one when there is no unit to start (a tarball install, a build tree).
    /// A thin client cannot be the daemon, but it can ask for one — the same
    /// bargain the tray struck in #43.
    fn spawn_daemon(&mut self, args: &[&str], fall_back: bool) {
        let unit = std::process::Command::new("systemctl").args(args).status();
        if fall_back && !unit.is_ok_and(|status| status.success()) {
            let _ = std::process::Command::new(sibling("nightlightd"))
                .arg("--daemon")
                .spawn();
        }
        self.last_poll = None;
    }

    /// Tab 1: the curve you can take hold of, and the override that
    /// overrules it.
    fn now_tab(&mut self, ui: &mut egui::Ui, pal: &theme::Palette, status: Option<Status>) {
        // The curve, in a card of its own that takes every point the controls
        // below do not want. It is the only thing here that grows, because it
        // is the only thing here that is worth more when it is bigger.
        let spare = self.curve_height;
        let (midnight, now_hour) = self.day_context();
        card(ui, pal.surface, |ui| {
            let shown_band = self.staged_band.unwrap_or(self.band);
            match curve::show(
                ui,
                curve::View {
                    status: status.as_ref(),
                    band: shown_band,
                    day_temp: self.day_temp,
                    night_temp: self.night_temp,
                    midnight,
                    now_hour: now_hour as f32,
                    pal,
                    height: spare,
                },
                &mut self.curve_held,
            ) {
                Some(curve::Edit::Band(next)) => self.staged_band = Some(next),
                Some(curve::Edit::DayTemp(kelvin)) => {
                    self.day_temp = kelvin;
                    self.staged_temps = true;
                }
                Some(curve::Edit::NightTemp(kelvin)) => {
                    self.night_temp = kelvin;
                    self.staged_temps = true;
                }
                None => {}
            }
            // The band in numbers (#48), or the decision an unsent drag is
            // waiting on. One row, in the curve's own card, because both are
            // about the shape directly above them.
            if self.staged_band.is_some() || self.staged_temps {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        if let Some(staged) = self.staged_band {
                            self.client
                                .set_band(staged.day_elevation, staged.night_elevation);
                            self.band = staged;
                            self.staged_band = None;
                        }
                        if self.staged_temps {
                            self.client.set_day_temp(self.day_temp);
                            self.client.set_night_temp(self.night_temp);
                            self.staged_temps = false;
                        }
                        self.last_poll = None;
                    }
                    // Back to what the daemon holds, not to what the panel
                    // opened with: this undoes the drag, not the session.
                    if ui.button("Revert").clicked() {
                        self.staged_band = None;
                        if self.staged_temps {
                            self.day_temp = self.daemon_day;
                            self.night_temp = self.daemon_night;
                            self.staged_temps = false;
                        }
                    }
                    let band = self.staged_band.unwrap_or(self.band);
                    ui.label(
                        egui::RichText::new(format!(
                            "{:+.1}° / {:+.1}°",
                            band.day_elevation, band.night_elevation
                        ))
                        .color(pal.muted)
                        .size(11.0),
                    );
                });
            } else if self.band != Band::default() {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "day above {:+.1}°, night below {:+.1}°",
                            self.band.day_elevation, self.band.night_elevation
                        ))
                        .color(pal.muted)
                        .size(11.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Default band").clicked() {
                            let fresh = Band::default();
                            self.client
                                .set_band(fresh.day_elevation, fresh.night_elevation);
                            self.band = fresh;
                            self.last_poll = None;
                        }
                    });
                });
            }
        });

        // Taking the screen off the sun for a while. Its own card, because it
        // is the one control here that overrules everything above it.
        let reading = format!("{} K", self.kelvin);
        let slider = egui::Slider::new(&mut self.kelvin, WARMEST..=COOLEST);
        let (moved, to_auto) = card(ui, pal.surface, |ui| {
            // Applied live only when the user actually moves it; the daemon
            // pins whatever the slider lands on and switches to manual.
            let moved = slider_row(ui, pal, "Hold at", reading, slider).changed();
            ui.label(
                egui::RichText::new("Pins the screen here and leaves the sun.")
                    .size(11.0)
                    .color(pal.muted),
            );
            ui.add_space(2.0);
            let mut to_auto = false;
            right_aligned(ui, |ui| {
                if ui.button("Back to automatic").clicked() {
                    to_auto = true;
                }
            });
            (moved, to_auto)
        });
        if moved {
            self.client.set_temperature(self.kelvin);
            // The cached snapshot is updated in place so the following-mode
            // mirror stops immediately instead of fighting the drag until the
            // next poll.
            if let Some(status) = &mut self.status {
                status.following = false;
                status.temperature = self.kelvin;
            }
        }
        if to_auto {
            self.client.follow_the_sun();
            self.last_poll = None;
        }
    }

    /// Tab 5: the four numbers that shape the curve, and the two
    /// switches about how the program behaves.
    fn settings_tab(&mut self, ui: &mut egui::Ui, pal: &theme::Palette) {
        // The two anchors that shape the curve. The ranges lean on each other
        // so the band cannot invert from here (the daemon clamps regardless),
        // and a change is sent — and hence written to the config file — once
        // per release, not once per drag frame.
        let day_min = self.night_temp.max(4000);
        let night_max = self.day_temp.min(4500);
        // The day bound runs past neutral into a bluish daytime (#41), as far
        // as core says a control should offer. The night bound does not
        // follow it up there: the whole point of the lower bound is to be
        // warmer than the upper one.
        let (night_min, day_max) = nightlightd_core::color::UI_TEMPERATURE_RANGE;
        // `update_while_editing(false)` on every slider: typing into the value
        // field must commit once, on Enter or focus loss — not per keystroke,
        // where a half-typed "75" would already have sent 7.
        let mut revert = false;
        let (day, night, gamma, dim) = card(ui, pal.surface, |ui| {
            let day = slider_row(
                ui,
                pal,
                "Daytime",
                format!("{} K", self.day_temp),
                egui::Slider::new(&mut self.day_temp, day_min..=day_max),
            );
            if day.drag_stopped() || (day.changed() && !day.dragged()) {
                self.client.set_day_temp(self.day_temp);
                // The slider sends on release, so whatever a plateau drag had
                // staged has now gone out by another door.
                self.staged_temps = false;
                self.last_poll = None;
            }
            let night = slider_row(
                ui,
                pal,
                "Nighttime",
                format!("{} K", self.night_temp),
                egui::Slider::new(&mut self.night_temp, night_min..=night_max),
            );
            if night.drag_stopped() || (night.changed() && !night.dragged()) {
                self.client.set_night_temp(self.night_temp);
                self.staged_temps = false;
                self.last_poll = None;
            }

            // The two ramp-shaping knobs (GitHub #2), sent on release like the
            // bounds above. Gamma bends the curve's midtones all day; night dim
            // lowers the ceiling after dark, easing on the same solar curve as the
            // temperature. The day-brightness bound stays whatever the daemon
            // holds — no interface exposes it, only the config file.
            // Display formatting only — `fixed_decimals` would round the backing
            // value itself on the first frame, marking the slider changed and
            // sending a value the user never chose (a config gamma of 0.925 must
            // not become 0.93 on disk just because the panel opened).
            let gamma = slider_row(
                ui,
                pal,
                "Gamma",
                format!("{:.2}", self.gamma),
                egui::Slider::new(&mut self.gamma, GAMMA_MIN..=GAMMA_MAX)
                    .custom_formatter(|v, _| format!("{v:.2}")),
            );
            // The format guard on both shaped knobs: clicking into the value field
            // and away again re-commits the display-rounded text (0.925 renders as
            // "0.92" and would come back as such). A difference the display cannot
            // even show is formatter residue, not a user choice — never sent.
            if (gamma.drag_stopped() || (gamma.changed() && !gamma.dragged()))
                && format!("{:.2}", self.gamma) != format!("{:.2}", self.daemon_gamma)
            {
                self.client.set_gamma(self.gamma);
                self.last_poll = None;
            }
            let dim = slider_row(
                ui,
                pal,
                "Night dim",
                format!("{:.0}%", self.night_dim * 100.0),
                egui::Slider::new(&mut self.night_dim, DIM_MIN..=DIM_MAX)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                    .custom_parser(|s| {
                        // "90" and "90%" mean ninety percent; "0.9" — the unit the
                        // config file uses — means the same. The slider's window
                        // makes the two readings unambiguous.
                        let v: f64 = s.trim().trim_end_matches('%').trim().parse().ok()?;
                        Some(if v > DIM_MAX { v / 100.0 } else { v })
                    }),
            );
            if (dim.drag_stopped() || (dim.changed() && !dim.dragged()))
                && format!("{:.0}%", self.night_dim * 100.0)
                    != format!("{:.0}%", self.daemon_night_dim * 100.0)
            {
                self.send_night_dim(self.night_dim);
            }
            // Back to the values the panel opened with, undoing this session's
            // slider fiddling. Right-aligned under the table it undoes, and quiet:
            // it is an escape hatch, not one of the four things you came to do.
            ui.add_space(2.0);
            right_aligned(ui, |ui| {
                if ui.button("Revert changes").clicked() {
                    revert = true;
                }
            });
            (day, night, gamma, dim)
        });
        self.bounds_dragging = day.dragged() || night.dragged() || gamma.dragged() || dim.dragged();

        // Nothing to undo before the first sync (the origs would be
        // compile-time defaults, not the user's values), and the shaped knobs
        // are only sent when this session actually moved them — an untouched
        // knob is not the panel's to rewrite, even with the value it believes
        // is current.
        if revert && self.anchors_synced {
            self.day_temp = self.orig_day;
            self.night_temp = self.orig_night;
            self.client.set_day_temp(self.day_temp);
            self.client.set_night_temp(self.night_temp);
            // Touched means either the UI copy moved off its seed or the
            // daemon's reported value moved off the opening one — the second
            // catches a knob dragged exactly onto the window edge, where the
            // clamped UI copy lands back on its seed while the daemon changed.
            let gamma_seed = self.orig_gamma.clamp(GAMMA_MIN, GAMMA_MAX);
            if self.gamma != gamma_seed || self.daemon_gamma != self.orig_gamma {
                // The verbatim orig, not the displayed clamp: a hand-written
                // out-of-window gamma comes back exactly as written.
                self.client.set_gamma(self.orig_gamma);
                self.gamma = gamma_seed;
            }
            let dim_seed = self.orig_night_dim.clamp(DIM_MIN, DIM_MAX);
            if self.night_dim != dim_seed || self.daemon_night_dim != self.orig_night_dim {
                self.send_night_dim(self.orig_night_dim);
                self.night_dim = dim_seed;
            }
            self.last_poll = None;
        }
        // What the window looks like, under what the screen looks like. The
        // same eight themes the dashboard carries, from the same table — a
        // name here and a name there have to mean one palette, or "nord" is
        // two different programs.
        let mut chosen = None;
        card(ui, pal.surface, |ui| {
            let applied = self.status.as_ref().map(|s| s.temperature);
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(LABEL_WIDTH, ROW_HEIGHT),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(egui::RichText::new("Theme").color(pal.muted));
                    },
                );
                egui::ComboBox::from_id_salt("theme")
                    .width(THEME_WIDTH)
                    .selected_text(
                        egui::RichText::new(theme::THEMES[self.theme_index].name).color(pal.accent),
                    )
                    .show_ui(ui, |ui| {
                        ui.set_min_width(THEME_WIDTH);
                        ui.spacing_mut().item_spacing.y = 4.0;
                        for (index, _) in theme::THEMES.iter().enumerate() {
                            if theme_swatch(ui, index, index == self.theme_index, applied).clicked()
                            {
                                chosen = Some(index);
                                ui.close();
                            }
                        }
                    });
            });
        });
        if let Some(index) = chosen {
            self.theme_index = index;
            remember_theme(index);
        }

        // The two switches share a card and a row. They are the only things
        // here that are neither a temperature nor a time — settings about how
        // the program behaves rather than about what the screen looks like —
        // so they read better as a pair than as two loose lines under the
        // last card.
        card(ui, pal.surface, |ui| {
            // Two even columns across the card, so the pair lines up with the
            // full-width rows above instead of huddling at the left edge.
            ui.horizontal(|ui| {
                let gap = ui.spacing().item_spacing.x;
                let half = ((ui.available_width() - gap) / 2.0).max(80.0);
                let column = |ui: &mut egui::Ui, add: &mut dyn FnMut(&mut egui::Ui)| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(half, ROW_HEIGHT),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| add(ui),
                    );
                };
                // The fade switch (#44) shows only when the daemon can answer
                // for it; sent optimistically, the next poll confirms.
                let mut fade_changed = None;
                if let Some(fade) = self.fade {
                    let mut checked = fade;
                    column(ui, &mut |ui| {
                        if ui.checkbox(&mut checked, "Fade transitions").changed() {
                            fade_changed = Some(checked);
                        }
                    });
                }
                if let Some(checked) = fade_changed {
                    self.client.set_fade(checked);
                    self.fade = Some(checked);
                    self.last_poll = None;
                }
                // Enables/disables the daemon's systemd user service, then
                // re-reads the real state — if systemctl failed (no unit
                // installed), the box must snap back instead of showing a
                // success that never happened.
                let mut login = self.start_at_login;
                let mut toggled = false;
                column(ui, &mut |ui| {
                    toggled = ui.checkbox(&mut login, "Start at login").changed();
                });
                if toggled {
                    self.start_at_login = login;
                    autostart::set(self.start_at_login);
                    self.start_at_login = autostart::enabled();
                }
            });
        });

        // Where all of the above ends up. Every knob on this tab is persisted
        // by the daemon the moment it is released, so the file is not an
        // alternative to the panel — it is the same settings, readable and
        // editable by hand, and the panel that hides it is asking to be
        // trusted about state it never showed.
        card(ui, pal.surface, |ui| {
            let path = config_path();
            ui.horizontal(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(LABEL_WIDTH, ROW_HEIGHT),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(egui::RichText::new("Config").color(pal.muted));
                    },
                );
                // Shortened for the window, copied in full: a path you cannot
                // paste is decoration.
                ui.label(
                    egui::RichText::new(tilde(&path))
                        .monospace()
                        .size(11.0)
                        .color(pal.text),
                );
                right_aligned(ui, |ui| {
                    if ui.button("Copy").clicked() {
                        ui.ctx().copy_text(path.clone());
                    }
                });
            });
            ui.add_space(2.0);
            ui.label(
                egui::RichText::new("Changes here are written to that file as you make them.")
                    .size(11.0)
                    .color(pal.faint),
            );
        });
    }

    /// The links, and the version they belong to.
    fn footer(&mut self, ui: &mut egui::Ui, pal: &theme::Palette) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(concat!("v", env!("CARGO_PKG_VERSION"))).color(pal.faint));
            ui.label(egui::RichText::new("·").color(pal.faint));
            ui.hyperlink_to("View on GitHub", REPO_URL);
            ui.label(egui::RichText::new("·").color(pal.faint));
            ui.hyperlink_to("Give feedback", ISSUES_URL);
        });
    }
}

/// Dresses egui in the palette. Sliders, buttons and checkboxes all read
/// their colours from `Visuals`, so setting it here is what makes the whole
/// window follow the screen rather than only the pieces we paint by hand.
fn paint(ui: &mut egui::Ui, pal: &theme::Palette) {
    let mut visuals = ui.visuals().clone();
    let visuals = &mut visuals;
    visuals.panel_fill = pal.bg;
    visuals.window_fill = pal.surface;
    visuals.extreme_bg_color = pal.bg;
    visuals.faint_bg_color = pal.surface;
    visuals.override_text_color = Some(pal.text);
    visuals.hyperlink_color = pal.accent2;
    visuals.selection.bg_fill = pal.accent;
    // The four widget states, lifting toward the accent as the pointer
    // arrives: a rail at rest, a face under the hand, the accent when held.
    visuals.widgets.noninteractive.bg_fill = pal.surface;
    visuals.widgets.noninteractive.weak_bg_fill = pal.surface;
    visuals.widgets.inactive.bg_fill = pal.raised;
    visuals.widgets.inactive.weak_bg_fill = pal.raised;
    visuals.widgets.hovered.bg_fill = pal.muted;
    visuals.widgets.hovered.weak_bg_fill = pal.muted;
    visuals.widgets.active.bg_fill = pal.accent;
    visuals.widgets.active.weak_bg_fill = pal.accent;
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
    ] {
        widget.corner_radius = 4.into();
    }
    // Borders belong to the surfaces, not to outlines: the raised shade is
    // what separates things, exactly as it does in the dashboard.
    // A separator is drawn with the non-interactive stroke, so this cannot
    // be NONE without the rules vanishing with it — quiet, not absent.
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, pal.raised);
    visuals.widgets.inactive.bg_stroke = egui::Stroke::NONE;
    visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, pal.accent);
    visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, pal.accent);
    // The context carries it to the window chrome and the next frame; the ui
    // gets it now, so the temperature that just arrived is already worn.
    ui.ctx().set_visuals(visuals.clone());
    *ui.visuals_mut() = visuals.clone();
    // Rows sat on top of each other at egui's default four points; a panel
    // read at arm's length wants more air than a debug overlay does.
    let spacing = &mut ui.style_mut().spacing;
    spacing.item_spacing = egui::vec2(8.0, 8.0);
    spacing.button_padding = egui::vec2(10.0, 5.0);
}

/// Mercator, and its inverse.
///
/// The obvious projection for a rectangle is the flat one — longitude across,
/// latitude down — and that is what this map drew first. The trouble is its
/// shape: the inhabited latitudes are about two and a half times wider than
/// they are tall, which is a letterbox rather than a world. Mercator stretches
/// the far latitudes, so the same crop comes out nearer five to three, and it
/// is the projection everyone has already seen. Its famous distortion costs us
/// nothing here: this map is for pointing at the town you live in, not for
/// comparing Greenland to Africa.
fn mercator(latitude: f64) -> f64 {
    (std::f64::consts::FRAC_PI_4 + latitude.to_radians() / 2.0)
        .tan()
        .ln()
}

fn unmercator(y: f64) -> f64 {
    (2.0 * y.exp().atan() - std::f64::consts::FRAC_PI_2).to_degrees()
}

/// How large the world is drawn inside a viewport, and which point of it sits
/// at the viewport's middle, after a drag of `drag` points.
///
/// Two rules that look independent — fill the frame completely, never stretch
/// the world — have exactly one solution together: scale until the tighter
/// side is covered and let the other side run off the edge. So the frame's
/// own shape decides which way the world is cropped. A wide window loses the
/// poles; a tall one loses the far east and west. What ran off is not gone,
/// it is dragged back into view, which is why this returns a centre and not
/// just a size.
///
/// The centre is kept in world fractions rather than pixels so that resizing
/// the window leaves the same *place* in the middle instead of the same
/// offset, and it is clamped to keep the world covering the frame: pan to the
/// edge and it stops there, because a gap at the border is the thing this
/// whole arrangement exists to prevent. On the axis with nothing to spare the
/// clamp collapses to a point, so that axis simply does not pan.
fn map_crop(
    frame: egui::Vec2,
    aspect: f32,
    center: egui::Vec2,
    drag: egui::Vec2,
) -> (egui::Vec2, egui::Vec2) {
    let height = (frame.x / aspect).max(frame.y).max(1.0);
    let world = egui::vec2(height * aspect, height);
    let half = egui::vec2(
        (frame.x / world.x).min(1.0) / 2.0,
        (frame.y / world.y).min(1.0) / 2.0,
    );
    let center = egui::vec2(
        (center.x - drag.x / world.x).clamp(half.x, 1.0 - half.x),
        (center.y - drag.y / world.y).clamp(half.y, 1.0 - half.y),
    );
    (world, center)
}

/// Where the daemon keeps its config, by the same XDG derivation the daemon
/// itself uses. Falls back to the literal `~/.config/...` when neither
/// variable is set, which is a session broken well past this program's
/// business — a path that is merely wrong reads better here than an empty
/// row that suggests there is no file at all.
fn config_path() -> String {
    nightlightd_core::paths::config_file("config.toml")
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "~/.config/nightlightd/config.toml".into())
}

/// The theme this window last wore, or `live` when it has never been asked.
/// Anything unreadable, empty, or naming a theme that no longer exists lands
/// on the default without complaint — the file is a convenience, and a night
/// light must not fail to open because somebody typed into it.
fn remembered_theme() -> usize {
    nightlightd_core::paths::config_file(THEME_FILE)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|name| theme::index_of(name.trim()))
        .unwrap_or(0)
}

/// Remembers the choice by name rather than by index — an index is one
/// release away from meaning a different theme. Failure is silence: the
/// colours still apply for this session, they just will not survive the
/// window closing, and that is not worth interrupting anyone over.
fn remember_theme(index: usize) {
    let Some(name) = theme::THEMES.get(index).map(|theme| theme.name) else {
        return;
    };
    let Some(path) = nightlightd_core::paths::config_file(THEME_FILE) else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, name);
}

/// The same path with the home directory written as `~`. The panel is a few
/// hundred points wide and `/home/somebody` spends a third of that saying
/// nothing the reader does not already know.
fn tilde(path: &str) -> String {
    let home = std::env::var_os("HOME").map(|home| home.to_string_lossy().into_owned());
    shorten_home(path, home.as_deref())
}

/// The substitution itself, taking the home directory rather than reading it,
/// so it can be tested against strings instead of against whichever account
/// happens to be running the tests.
///
/// The prefix has to end on a path boundary. `/home/umut` is a prefix of
/// `/home/umutdincer` by text and of nothing at all by filesystem, and the
/// difference between those two is a nonsense path shown with confidence.
fn shorten_home(path: &str, home: Option<&str>) -> String {
    let home = home
        .map(|h| h.trim_end_matches('/'))
        .filter(|h| !h.is_empty());
    match home {
        Some(home) if path == home => "~".to_owned(),
        Some(home) if path.starts_with(home) && path[home.len()..].starts_with('/') => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_owned(),
    }
}

/// "41.0°N 29.0°E" for a signed pair, the same shape the dashboard prints.
fn format_coords(latitude: f64, longitude: f64) -> String {
    format!(
        "{:.1}°{} {:.1}°{}",
        latitude.abs(),
        if latitude >= 0.0 { "N" } else { "S" },
        longitude.abs(),
        if longitude >= 0.0 { "E" } else { "W" },
    )
}

/// "in 2h 05m" / "3h 12m ago" / "now" for a signed hour delta, so a row says
/// how far off it is without the reader doing arithmetic against the clock.
fn relative(delta_hours: f64) -> String {
    let minutes = (delta_hours * 60.0).round() as i64;
    if minutes.abs() < 1 {
        return "now".into();
    }
    let (hours, rest) = (minutes.abs() / 60, minutes.abs() % 60);
    let span = if hours > 0 {
        format!("{hours}h {rest:02}m")
    } else {
        format!("{rest}m")
    };
    if minutes > 0 {
        format!("in {span}")
    } else {
        format!("{span} ago")
    }
}

/// A little sky beside the phase word: a rayed sun by day, a crescent at
/// night, a sun sitting on the horizon through the transition. Drawn rather
/// than typed, because the crescent is a disc with a bite taken out of it in
/// the card's own colour and no font can be relied on for a moon.
///
/// The idiom is the dashboard's `sky_art` at a fourteenth of the size: same
/// three scenes, same reason — a word tells you the phase, a shape lets you
/// see it without reading.
fn sky_mark(ui: &mut egui::Ui, pal: &theme::Palette, ground: egui::Color32, phase: &str) {
    let size = 14.0;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let painter = ui.painter();
    let centre = rect.center();
    match phase {
        "night" => {
            // A disc, then the same disc again in the card's ground, shifted
            // up and to the right: what is left is the crescent.
            painter.circle_filled(centre, size * 0.42, pal.accent);
            painter.circle_filled(
                centre + egui::vec2(size * 0.24, -size * 0.18),
                size * 0.36,
                ground,
            );
        }
        "day" => {
            painter.circle_filled(centre, size * 0.26, pal.accent);
            for step in 0..8 {
                let angle = std::f32::consts::TAU * step as f32 / 8.0;
                let dir = egui::vec2(angle.cos(), angle.sin());
                painter.line_segment(
                    [centre + dir * size * 0.36, centre + dir * size * 0.49],
                    egui::Stroke::new(1.2, pal.accent),
                );
            }
        }
        // The transition: half a sun over the line it is crossing, which is
        // the whole of what the band means.
        _ => {
            let horizon = centre.y + size * 0.20;
            painter.circle_filled(
                egui::pos2(centre.x, horizon - size * 0.10),
                size * 0.30,
                pal.accent,
            );
            painter.line_segment(
                [
                    egui::pos2(rect.left(), horizon),
                    egui::pos2(rect.right(), horizon),
                ],
                egui::Stroke::new(1.2, pal.muted),
            );
        }
    }
}

/// A card: a raised surface with real padding and a soft corner. The panel's
/// groups used to be marked by a horizontal rule and a guess; this makes each
/// one an object you can point at. The dashboard does the same thing with the
/// same reasoning — elevation instead of frames.
fn card<R>(ui: &mut egui::Ui, fill: egui::Color32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let out = egui::Frame::new()
        .fill(fill)
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            // A Frame shrinks to whatever its content used, so a card whose
            // rows do not happen to reach the edge comes out narrower than
            // the card above it. Cards are the window's columns; they are all
            // one width or they are not cards.
            ui.set_width(ui.available_width());
            add(ui)
        });
    ui.add_space(8.0);
    out.inner
}

/// A pill: a quiet name and a lit value on a raised ground.
///
/// The day's small facts are all the same shape — a word and a number — and a
/// run of them only reads as a run if they are drawn the same. Loose text
/// separated by middots reads as one long sentence that happens to contain
/// digits; these read as things you can pick out one at a time.
fn badge(
    ui: &mut egui::Ui,
    pal: &theme::Palette,
    name: &str,
    value: &str,
    tint: egui::Color32,
) -> egui::Response {
    egui::Frame::new()
        .fill(pal.raised)
        .corner_radius(9)
        .inner_margin(egui::Margin::symmetric(9, 4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                if !name.is_empty() {
                    ui.label(egui::RichText::new(name).size(10.0).color(pal.muted));
                }
                ui.label(egui::RichText::new(value).size(11.0).strong().color(tint));
            });
        })
        .response
}

/// One row of the theme list, wearing the colours it is offering: that
/// theme's own ground, its own accent, its name.
///
/// A dropdown of eight plain words would have you choose a palette by reading
/// about it. Each row here *is* the palette, so opening the list is already
/// seeing the answer — and the closed row costs one line, which is what a
/// settings table can spare.
fn theme_swatch(
    ui: &mut egui::Ui,
    index: usize,
    selected: bool,
    applied: Option<u32>,
) -> egui::Response {
    let own = theme::Palette::of(index, applied);
    let name = theme::THEMES[index].name;
    let response = egui::Frame::new()
        .fill(own.surface)
        // The chosen one is ringed in its own accent. A tick would say the
        // same thing in a language the list is not speaking.
        .stroke(egui::Stroke::new(
            1.5,
            if selected {
                own.accent
            } else {
                egui::Color32::TRANSPARENT
            },
        ))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            // A Frame shrinks to its content, which would leave eight ragged
            // pills of different widths; the list reads as a list when every
            // row is the same block of colour.
            ui.set_width(ui.available_width());
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
            ui.label(
                egui::RichText::new(name)
                    .size(12.0)
                    .strong()
                    .color(own.accent),
            );
        })
        .response
        .interact(egui::Sense::click());
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// "14h 32m" for a span of hours. Hours and minutes because that is how long
/// a day is said out loud; a decimal would be shorter and would have to be
/// converted in the reader's head before it meant anything.
fn hours_and_minutes(hours: f64) -> String {
    let minutes = (hours * 60.0).round().max(0.0) as i64;
    format!("{}h {:02}m", minutes / 60, minutes % 60)
}

/// "3m 12s longer", against yesterday. Seconds because at the solstices the
/// difference is a handful of them, and a figure that reads "0m longer" for a
/// fortnight either side of midsummer is worse than no figure.
fn length_change(delta_hours: f64) -> String {
    let seconds = (delta_hours.abs() * 3600.0).round() as i64;
    if seconds == 0 {
        return "the same as yesterday".to_owned();
    }
    let word = if delta_hours > 0.0 {
        "longer"
    } else {
        "shorter"
    };
    match (seconds / 60, seconds % 60) {
        (0, s) => format!("{s}s {word} than yesterday"),
        (m, s) => format!("{m}m {s:02}s {word} than yesterday"),
    }
}

/// A row that hugs the right edge. `Ui::with_layout` on its own takes every
/// point of height the parent has left — inside a scroll area that is the
/// whole viewport, which swelled a card to the height of the window.
/// Allocating one row's worth first is what keeps it a row.
fn right_aligned<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), BUTTON_HEIGHT),
        egui::Layout::right_to_left(egui::Align::Center),
        add,
    )
    .inner
}

/// One control, on one line: name, rail, reading. The four settings used to
/// take two lines each — a label row and a full-width rail — which read as
/// eight unrelated things stacked up. Fixed columns on both ends make them a
/// table, so the eye runs down the names and down the numbers.
fn slider_row<'a>(
    ui: &mut egui::Ui,
    pal: &theme::Palette,
    label: &str,
    reading: String,
    slider: egui::Slider<'a>,
) -> egui::Response {
    let mut response = None;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(LABEL_WIDTH, ROW_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(egui::RichText::new(label).color(pal.muted));
            },
        );
        let gap = ui.spacing().item_spacing.x;
        let rail = (ui.available_width() - READING_WIDTH - gap).max(40.0);
        // A Slider allocates `spacing.slider_width` for its rail and ignores
        // whatever size it was added at, so the width has to be set here.
        ui.spacing_mut().slider_width = rail;
        response = Some(ui.add(slider.show_value(false).update_while_editing(false)));
        ui.allocate_ui_with_layout(
            egui::vec2(READING_WIDTH, ROW_HEIGHT),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.label(egui::RichText::new(reading).color(pal.text).monospace());
            },
        );
    });
    response.expect("the horizontal layout always runs")
}

impl eframe::App for Panel {
    // eframe 0.35 wraps this in a CentralPanel itself, so we draw straight into
    // the provided `ui` instead of opening our own panel.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // A second launch asked us to come forward.
        if self.focus.swap(false, Ordering::Relaxed) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Focus);
        }

        // A demo run answers its own questions and asks the daemon none of
        // them, every frame rather than once a second: the compressed day
        // moves too fast for a one-second cache to look like anything but a
        // stutter.
        if self.demo.is_some() {
            self.apply_demo();
            self.run_demo_script();
        }
        // Refresh the snapshot at most once a second (or when an action just
        // invalidated it); every frame in between reuses the cache.
        else if self
            .last_poll
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(1))
        {
            self.status = self.client.status();
            self.fade = self.client.fade();
            self.outputs = self.client.outputs();
            self.band = self
                .client
                .band()
                .map(|(day, night)| {
                    Band {
                        day_elevation: day,
                        night_elevation: night,
                    }
                    .sane()
                })
                .unwrap_or_default();
            match self.status.as_ref().filter(|s| s.has_location) {
                Some(s) => {
                    let moved = self.place.as_ref().is_none_or(|(lat, lon, _)| {
                        (lat - s.latitude).abs() > 0.01 || (lon - s.longitude).abs() > 0.01
                    });
                    if moved {
                        self.place = nearest_zone(s.latitude, s.longitude).map(|(zone, _, _)| {
                            let city = zone.rsplit('/').next().unwrap_or(&zone).replace('_', " ");
                            (s.latitude, s.longitude, city)
                        });
                    }
                }
                None => self.place = None,
            }
            self.mismatch = self.status.is_none() && self.client.daemon_on_bus();
            if self.mismatch {
                // Maybe this process is simply older than the file it came
                // from; one silent relaunch answers that. Still mismatched
                // afterwards means the disk needs the user.
                relaunch_once();
            }
            self.last_poll = Some(Instant::now());
        }
        let status = self.status.clone();

        // In automatic mode the slider mirrors the live sun-based temperature,
        // so it drifts down as the sun sets and snaps back after "Back to
        // automatic". The first drag (below) switches the daemon to manual,
        // `following` goes false, and this stops — leaving the slider to the
        // user. When following, the slider already shows this value, so writing
        // it again is a no-op and never fights a drag.
        if let Some(status) = &status
            && status.following
        {
            self.kelvin = status.temperature.clamp(WARMEST, COOLEST);
        }

        // Seed the day/night sliders from the daemon once; after that they are
        // the source of truth (each change is sent and persisted).
        if !self.anchors_synced
            && let Some(status) = &status
        {
            self.day_temp = status.day_temp;
            self.night_temp = status.night_temp;
            self.orig_day = status.day_temp;
            self.orig_night = status.night_temp;
            self.daemon_day = status.day_temp;
            self.daemon_night = status.night_temp;
            // The UI copies live inside the sliders' windows (egui silently
            // clamps the backing value there anyway); orig_* and daemon_*
            // keep the verbatim values, so a hand-written config value
            // outside the window is never the panel's to rewrite.
            self.gamma = status.gamma.clamp(GAMMA_MIN, GAMMA_MAX);
            self.night_dim = status.night_brightness.clamp(DIM_MIN, DIM_MAX);
            self.orig_gamma = status.gamma;
            self.orig_night_dim = status.night_brightness;
            self.daemon_gamma = status.gamma;
            self.daemon_night_dim = status.night_brightness;
            // Seed the warm slider from what is actually applied, so a panel
            // opened during a manual override shows the truth instead of the
            // compile-time default (the following-mode mirror only covers auto).
            self.kelvin = status.temperature.clamp(WARMEST, COOLEST);
            self.anchors_synced = true;
        }

        // Adopt bounds changed elsewhere (another client, a daemon restart with
        // a different config); our own sends update daemon_* via the poll that
        // follows them, so this only fires on genuinely external changes.
        if self.anchors_synced
            && !self.bounds_dragging
            && !self.staged_temps
            && self.curve_held.is_none()
            && let Some(status) = &status
            && (status.day_temp != self.daemon_day
                || status.night_temp != self.daemon_night
                || status.gamma != self.daemon_gamma
                || status.night_brightness != self.daemon_night_dim)
        {
            self.day_temp = status.day_temp;
            self.night_temp = status.night_temp;
            self.daemon_day = status.day_temp;
            self.daemon_night = status.night_temp;
            self.gamma = status.gamma.clamp(GAMMA_MIN, GAMMA_MAX);
            self.night_dim = status.night_brightness.clamp(DIM_MIN, DIM_MAX);
            self.daemon_gamma = status.gamma;
            self.daemon_night_dim = status.night_brightness;
        }

        // The window wears the chosen theme — and on the default one, what the
        // screen is doing: every tone mixed from the applied temperature, the
        // same table and the same arithmetic the dashboard runs. Applied
        // before anything is drawn, so widgets pick it up on the frame the
        // temperature changes.
        let pal = theme::Palette::of(
            self.theme_index,
            self.status.as_ref().map(|s| s.temperature),
        );
        paint(ui, &pal);

        // eframe hands us a Ui with no margin at all — its own documentation
        // says so and tells you to wrap it — which is why every label sat
        // flush against the window edge and every value on the right was
        // clipped by it. One inset child, and the whole window breathes.
        let mut inset = ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect().shrink(MARGIN)));
        let ui = &mut inset;

        // No scroll area: the window is a fixed stack of cards with exactly
        // one elastic member, the curve. Everything else has a height it
        // always wants, so the smallest window that fits them is a number we
        // can compute — and it is the minimum the window enforces below. A
        // panel that scrolls is a panel whose minimum size was guessed.
        self.body(ui, &pal, status);

        // egui is reactive: it draws when something asks it to, and an idle
        // panel asking once a second is exactly right — the sun does not move
        // faster than that and a settings window has no business burning a
        // core to prove it.
        //
        // A demo run is the one case where that is wrong. Its day is
        // compressed into half a minute, so once a second is thirty frames
        // for a whole sunset, and the reel comes out as a slideshow. There it
        // asks for every frame the display will give.
        if self.demo.is_some() {
            ui.ctx().request_repaint();
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs(1));
        }
    }
}

/// The path to a sibling binary — the daemon next to this panel — for the
/// case where systemd has no unit to start. Falls back to the bare name so
/// `PATH` still gets a look in.
fn sibling(name: &str) -> std::path::PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .unwrap_or_else(|| std::path::PathBuf::from(name))
}

/// The one self-repair a stale client can do (#42): replace this process
/// with whatever its own path holds on disk now. After an update the
/// running copy is old while the file is new, and this heals that with
/// nobody watching. Guarded to a single attempt — when the disk copy is
/// just as old, exec would loop forever otherwise. `exec` only returns on
/// failure, and every failure path falls through to the visible notice.
fn relaunch_once() {
    use std::os::unix::process::CommandExt;
    use std::sync::atomic::{AtomicBool, Ordering};
    static TRIED: AtomicBool = AtomicBool::new(false);
    if TRIED.swap(true, Ordering::SeqCst) || std::env::var_os("NIGHTLIGHT_RELAUNCHED").is_some() {
        return;
    }
    let mut args = std::env::args_os();
    let Some(argv0) = args.next() else {
        return;
    };
    let _ = std::process::Command::new(argv0)
        .args(args)
        .env("NIGHTLIGHT_RELAUNCHED", "1")
        .exec();
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

/// What the command line asked the window to be.
struct Options {
    /// Which tab to open on. A flag rather than a click, so the showcase can
    /// photograph each tab without anything driving a mouse.
    tab: usize,
    /// Run the scripted tour on a compressed day, reading no daemon and
    /// writing to none.
    demo: bool,
}

/// Parses the command line, or answers it and stops.
///
/// The panel parsed nothing at all until now, so any flag handed to it simply
/// opened the window — `--version` included. A packager or a bug report runs
/// that first, and a mistyped option deserves a complaint rather than a
/// window. [`None`] means the argument was the whole of the job.
fn parse_args() -> Option<Options> {
    let usage = format!(
        "usage: nightlight-panel [--version] [--help] [--tab <{}>] [--demo]

The settings window: the day/night curve, the schedule, the map, the
outputs and the knobs. Everything is set inside it; these options only
choose how it opens.",
        TABS.join(", ")
    );
    let mut options = Options {
        tab: NOW_TAB,
        demo: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--version" | "-V" => {
                println!("nightlight-panel {}", env!("CARGO_PKG_VERSION"));
                return None;
            }
            "--help" | "-h" => {
                println!("{usage}");
                return None;
            }
            "--demo" => options.demo = true,
            "--tab" => {
                let Some(name) = args.next() else {
                    eprintln!("{usage}");
                    std::process::exit(2);
                };
                let Some(index) = TABS.iter().position(|title| **title == name) else {
                    eprintln!("nightlight-panel: unknown tab {name:?}\n\n{usage}");
                    std::process::exit(2);
                };
                options.tab = index;
            }
            other => {
                eprintln!("nightlight-panel: unknown option {other:?}\n\n{usage}");
                std::process::exit(2);
            }
        }
    }
    Some(options)
}

fn main() -> eframe::Result<()> {
    let Some(args) = parse_args() else {
        return Ok(());
    };
    // Single instance: if a panel is already open, ask it to come forward and
    // exit instead of opening a second window.
    let focus = Arc::new(AtomicBool::new(false));
    let _lock = match single::acquire(Arc::clone(&focus)) {
        Some(connection) => connection,
        None => return Ok(()),
    };

    let mut client = match Client::connect() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("nightlight-panel: cannot reach the session bus: {error}");
            std::process::exit(1);
        }
    };
    // A reel must not touch the settings of whoever is recording it. Muted at
    // the door rather than checked at each send, so no future write can
    // forget to ask.
    if args.demo {
        client.mute();
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            // Sized for the tallest tab rather than for all of them stacked:
            // splitting the window into tabs took roughly a third of its
            // height away, and what is left over on the shorter tabs is dead
            // space nobody asked for.
            .with_inner_size([470.0, 600.0])
            // Resizable, with a floor that keeps the curve readable: the
            // window used to be nailed to one size and left a hand of dead
            // space under the footer at that size.
            // The floor is the tallest tab at its tightest, and that is no
            // longer the now tab: settings carries four cards — the four
            // bounds, the theme, the two switches, the config path — and
            // wants nearly the whole opening height. Below this something is
            // cut off, and with no scroll area to fall back on, cut off means
            // gone. It has to be raised whenever a card is added to settings.
            .with_min_inner_size([390.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "nightlightd",
        options,
        Box::new(|_cc| {
            Ok(Box::new(Panel {
                client,
                kelvin: START_KELVIN,
                day_temp: DEFAULT_DAY,
                night_temp: default_night(args.demo),
                anchors_synced: false,
                orig_day: DEFAULT_DAY,
                orig_night: default_night(args.demo),
                gamma: 1.0,
                night_dim: 1.0,
                orig_gamma: 1.0,
                orig_night_dim: 1.0,
                start_at_login: autostart::enabled(),
                offset_secs: local_offset_seconds(),
                focus: Arc::clone(&focus),
                status: None,
                fade: None,
                mismatch: false,
                band: Band::default(),
                staged_band: None,
                staged_temps: false,
                curve_held: None,
                last_poll: None,
                daemon_day: 6500,
                daemon_night: 4500,
                daemon_gamma: 1.0,
                daemon_night_dim: 1.0,
                bounds_dragging: false,
                place: None,
                theme_index: remembered_theme(),
                outputs: None,
                tab: args.tab,
                demo: args.demo.then(Instant::now),
                demo_cursor: 0,
                map_center: egui::vec2(0.5, 0.5),
                curve_height: 160.0,
            }))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The schedule's fourth column has to read without arithmetic: a row is
    /// ahead, behind, or happening.
    #[test]
    fn relative_reads_forwards_and_backwards() {
        assert_eq!(relative(0.0), "now");
        assert_eq!(relative(2.0), "in 2h 00m");
        assert_eq!(relative(-2.5), "2h 30m ago");
        assert_eq!(relative(0.25), "in 15m");
        assert_eq!(relative(-0.25), "15m ago");
        // Under half a minute either way is not worth a number.
        assert_eq!(relative(0.005), "now");
    }

    /// The day's two readings, at the sizes they actually take: a length said
    /// the way a day is said out loud, and a change that stays useful when it
    /// shrinks to seconds — which is exactly when someone is watching for it.
    #[test]
    fn the_day_reads_in_hours_and_its_change_in_seconds() {
        assert_eq!(hours_and_minutes(14.956), "14h 57m");
        assert_eq!(hours_and_minutes(9.0), "9h 00m");
        assert_eq!(hours_and_minutes(0.0), "0h 00m");

        assert_eq!(
            length_change(192.0 / 3600.0),
            "3m 12s longer than yesterday"
        );
        assert_eq!(
            length_change(-192.0 / 3600.0),
            "3m 12s shorter than yesterday"
        );
        // Around the solstices the change is seconds; "0m 12s" is arithmetic
        // showing through, and a rounded-away change is not a change at all.
        assert_eq!(length_change(12.0 / 3600.0), "12s longer than yesterday");
        assert_eq!(length_change(0.0), "the same as yesterday");
        assert_eq!(length_change(0.00001), "the same as yesterday");
    }

    /// The home directory is shortened only when it is genuinely the path's
    /// parent. Matching on text alone turns a neighbour's directory into a
    /// path that does not exist, shown as if it did.
    #[test]
    fn only_a_real_home_prefix_becomes_a_tilde() {
        let home = Some("/home/umut");
        assert_eq!(shorten_home("/home/umut/.config/x", home), "~/.config/x");
        assert_eq!(shorten_home("/home/umut", home), "~");
        assert_eq!(
            shorten_home("/home/umutdincer/x", home),
            "/home/umutdincer/x"
        );
        assert_eq!(shorten_home("/etc/x", home), "/etc/x");
        // A trailing slash on HOME must not eat the one that follows it, and
        // an unset or empty HOME leaves the path exactly as it was.
        assert_eq!(shorten_home("/home/umut/x", Some("/home/umut/")), "~/x");
        assert_eq!(shorten_home("/home/umut/x", Some("")), "/home/umut/x");
        assert_eq!(shorten_home("/home/umut/x", None), "/home/umut/x");
    }

    /// The map's contract, at both shapes a window can be: the world covers
    /// the frame edge to edge, and its own proportions survive. Everything
    /// else about the map follows from these two holding at once.
    #[test]
    fn the_world_fills_the_frame_without_stretching() {
        let aspect = 1.67;
        let still = egui::Vec2::ZERO;
        for frame in [egui::vec2(600.0, 200.0), egui::vec2(200.0, 400.0)] {
            let (world, _) = map_crop(frame, aspect, egui::vec2(0.5, 0.5), still);
            assert!(
                world.x >= frame.x,
                "{world:?} leaves a gap beside {frame:?}"
            );
            assert!(world.y >= frame.y, "{world:?} leaves a gap under {frame:?}");
            assert!(
                (world.x / world.y - aspect).abs() < 1e-3,
                "{world:?} is not the world's shape"
            );
        }
    }

    /// Dragging moves the crop the way the hand went, stops at the world's
    /// edge rather than opening a gap, and does nothing at all on the axis
    /// that had no room to give.
    #[test]
    fn the_crop_pans_within_the_world_and_no_further() {
        let aspect = 1.67;
        // Wider than the world: the width is spent exactly, so only the
        // vertical can move.
        let frame = egui::vec2(600.0, 200.0);
        let middle = egui::vec2(0.5, 0.5);
        let (world, up) = map_crop(frame, aspect, middle, egui::vec2(-40.0, -40.0));
        assert_eq!(up.x, 0.5, "there is no width to pan into");
        assert!(up.y > 0.5, "dragging up should reveal what is below");

        // Far past the edge, twice, and it comes to rest in the same place.
        let shove = egui::vec2(0.0, -10_000.0);
        let (_, once) = map_crop(frame, aspect, middle, shove);
        let (_, twice) = map_crop(frame, aspect, once, shove);
        assert_eq!(once, twice, "the pan should stop at the edge");
        let bottom = once.y + (frame.y / world.y) / 2.0;
        assert!((bottom - 1.0).abs() < 1e-4, "the world's edge is the stop");
    }

    /// The centre is a place, not an offset: resize the window and whatever
    /// was in the middle of the map is still in the middle of the map.
    #[test]
    fn resizing_keeps_the_same_place_in_the_middle() {
        let aspect = 1.67;
        let still = egui::Vec2::ZERO;
        let (_, aimed) = map_crop(
            egui::vec2(800.0, 400.0),
            aspect,
            egui::vec2(0.5, 0.5),
            egui::vec2(0.0, -30.0),
        );
        assert!(aimed.y > 0.5, "the drag has to have moved something");
        let (_, resized) = map_crop(egui::vec2(830.0, 400.0), aspect, aimed, still);
        assert!((resized.y - aimed.y).abs() < 1e-6, "the middle drifted");
    }
}
