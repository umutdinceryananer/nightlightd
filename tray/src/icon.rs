//! The colour tray icons (GitHub #4): painted here, pixel by pixel, rather
//! than shipped as image files.
//!
//! Painted because the colour is the point. An image file can say "day" or
//! "night"; a painted icon can say how warm the screen is right now. It also
//! leaves nothing to install, so the `.deb`, the AUR package and a `cargo
//! install` all carry the same icons without a line of packaging.
//!
//! Every shape is a signed distance: for any point, how far it is from the
//! shape's edge, negative inside. Coordinates run from -1 to 1 across the
//! icon, `y` downwards. The half pixel either side of an edge becomes its
//! soft border, so an icon is drawn directly at each size instead of being
//! shrunk from a big one, which is where small icons go to mud.

use nightlightd_core::tray_icon::TrayIcon;

/// Red, green and blue, each from 0 to 255.
type Rgb = [f64; 3];

const fn hex(value: u32) -> Rgb {
    [
        ((value >> 16) & 0xff) as f64,
        ((value >> 8) & 0xff) as f64,
        (value & 0xff) as f64,
    ]
}

const WHITE: Rgb = hex(0xFFFFFF);
const BLACK: Rgb = hex(0x000000);
const GREY: Rgb = hex(0x9AA0A8);
const NAVY: Rgb = hex(0x262C5E);
const NAVY_DEEP: Rgb = hex(0x13183A);
const DIMMED_TOP: Rgb = hex(0x5C6068);
const DIMMED_BOTTOM: Rgb = hex(0x43464C);

/// The sizes offered to the panel, which takes the closest and scales it.
/// 16 to 48 are what panels ask for; 64 is a 32-pixel panel on a doubled
/// screen.
const SIZES: [i32; 6] = [16, 22, 24, 32, 48, 64];

/// At or under this many pixels a design spends its pixels differently:
/// fewer details and thicker strokes. This is where a panel icon actually
/// lives, and a drawing that only works large is a drawing nobody sees.
const SMALL: i32 = 24;

/// How warm the screen is, as an icon shows it.
///
/// More saturated on purpose than `core`'s `display_tint`, the colour every
/// other interface reads warmth from. That one is pale peach at 6500 K, which
/// is honest on a dark page and no use on a busy panel, and a panel icon that
/// does not read as coloured is the whole of GitHub #4.
const WARMTH: [(f64, Rgb); 6] = [
    (1700.0, hex(0xD63A2A)),
    (2700.0, hex(0xEE5A2A)),
    (3200.0, hex(0xFA7426)),
    (4000.0, hex(0xFF9426)),
    (5000.0, hex(0xFFB02E)),
    (6500.0, hex(0xFFC940)),
];

fn mix(from: Rgb, to: Rgb, amount: f64) -> Rgb {
    [0, 1, 2].map(|i| from[i] + (to[i] - from[i]) * amount)
}

fn tint(kelvin: u32) -> Rgb {
    let kelvin = f64::from(kelvin);
    WARMTH
        .windows(2)
        .find(|stops| kelvin <= stops[1].0)
        .map(|stops| {
            let along = (kelvin - stops[0].0) / (stops[1].0 - stops[0].0);
            mix(stops[0].1, stops[1].1, along.clamp(0.0, 1.0))
        })
        .unwrap_or(WARMTH[WARMTH.len() - 1].1)
}

/// What the icon has to say about the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// The filter is on and the screen is at this temperature.
    Warmed(u32),
    /// The filter is off, or nothing is running to filter anything.
    Off,
}

impl Look {
    /// The screen at `kelvin`, to the nearest hundred. A fade moves the
    /// temperature a few kelvin at a time, nobody can see that on a 22-pixel
    /// icon, and every difference the panel is told about costs it a redraw.
    pub fn warmed(kelvin: u32) -> Look {
        Look::Warmed(kelvin.saturating_add(50) / 100 * 100)
    }

    fn colour(self) -> Rgb {
        match self {
            Look::Warmed(kelvin) => tint(kelvin),
            Look::Off => GREY,
        }
    }
}

/// An outline, answering how far any point is from its edge.
enum Shape {
    Circle {
        x: f64,
        y: f64,
        r: f64,
    },
    RoundBox {
        x: f64,
        y: f64,
        half_width: f64,
        half_height: f64,
        r: f64,
    },
    Union(Vec<Shape>),
    Minus(Box<Shape>, Box<Shape>),
    /// The first shape, only where it is inside the second.
    Inside(Box<Shape>, Box<Shape>),
}

fn circle(x: f64, y: f64, r: f64) -> Shape {
    Shape::Circle { x, y, r }
}

fn round_box(x: f64, y: f64, half_width: f64, half_height: f64, r: f64) -> Shape {
    Shape::RoundBox {
        x,
        y,
        half_width,
        half_height,
        r,
    }
}

fn minus(shape: Shape, taken: Shape) -> Shape {
    Shape::Minus(Box::new(shape), Box::new(taken))
}

impl Shape {
    fn distance(&self, px: f64, py: f64) -> f64 {
        match self {
            Shape::Circle { x, y, r } => (px - x).hypot(py - y) - r,
            Shape::RoundBox {
                x,
                y,
                half_width,
                half_height,
                r,
            } => {
                let dx = (px - x).abs() - (half_width - r);
                let dy = (py - y).abs() - (half_height - r);
                dx.max(0.0).hypot(dy.max(0.0)) + dx.max(dy).min(0.0) - r
            }
            Shape::Union(parts) => parts
                .iter()
                .map(|part| part.distance(px, py))
                .fold(f64::INFINITY, f64::min),
            Shape::Minus(shape, taken) => shape.distance(px, py).max(-taken.distance(px, py)),
            Shape::Inside(shape, within) => shape.distance(px, py).max(within.distance(px, py)),
        }
    }
}

/// What fills a shape.
enum Paint {
    Flat(Rgb),
    /// Light at the top left, dark at the bottom right.
    Diagonal {
        light: Rgb,
        dark: Rgb,
        spread: f64,
    },
    /// From `top` at `from` to `bottom` at `to`.
    Vertical {
        top: Rgb,
        bottom: Rgb,
        from: f64,
        to: f64,
    },
}

impl Paint {
    fn at(&self, x: f64, y: f64) -> Rgb {
        match self {
            Paint::Flat(colour) => *colour,
            Paint::Diagonal {
                light,
                dark,
                spread,
            } => mix(
                *light,
                *dark,
                ((x + y + spread) / (2.0 * spread)).clamp(0.0, 1.0),
            ),
            Paint::Vertical {
                top,
                bottom,
                from,
                to,
            } => mix(*top, *bottom, ((y - from) / (to - from)).clamp(0.0, 1.0)),
        }
    }
}

/// One shape in one paint. An icon is a stack of these, bottom first.
struct Layer {
    shape: Shape,
    paint: Paint,
}

fn night_badge(look: Look, size: i32) -> Vec<Layer> {
    let colour = look.colour();
    let (inner, moon) = if size <= SMALL {
        // A thicker rim and a fatter crescent: at 16 pixels the thin ones
        // are a single row of blurred colour.
        (
            0.80,
            minus(circle(-0.08, 0.05, 0.60), circle(0.24, -0.20, 0.50)),
        )
    } else {
        (
            0.87,
            minus(circle(-0.08, 0.04, 0.62), circle(0.20, -0.16, 0.54)),
        )
    };
    // Stars go first when pixels run out; a star smaller than a pixel is
    // only a smudge in the crescent's way.
    let stars = if size <= 16 {
        circle(0.42, -0.30, 0.11)
    } else if size <= SMALL {
        Shape::Union(vec![circle(0.42, -0.32, 0.095), circle(0.56, 0.12, 0.075)])
    } else {
        Shape::Union(vec![
            circle(0.42, -0.30, 0.085),
            circle(0.62, 0.14, 0.06),
            circle(0.16, -0.60, 0.055),
        ])
    };
    let (top, bottom) = match look {
        Look::Warmed(_) => (NAVY, NAVY_DEEP),
        Look::Off => (DIMMED_TOP, DIMMED_BOTTOM),
    };
    vec![
        Layer {
            shape: circle(0.0, 0.0, 0.96),
            paint: Paint::Flat(mix(colour, BLACK, 0.15)),
        },
        Layer {
            shape: circle(0.0, 0.0, inner),
            paint: Paint::Vertical {
                top,
                bottom,
                from: -1.0,
                to: 1.0,
            },
        },
        Layer {
            shape: moon,
            paint: Paint::Diagonal {
                light: mix(colour, WHITE, 0.30),
                dark: mix(colour, BLACK, 0.08),
                spread: 1.2,
            },
        },
        Layer {
            shape: stars,
            paint: Paint::Flat(mix(colour, WHITE, 0.55)),
        },
    ]
}

/// How far toward night a temperature is: 0 in full daylight, 1 from 2700 K
/// down. Absolute rather than measured against the configured night, so that
/// someone who only ever warms to 4500 K gets a dusk, which is what their
/// screen looks like, and not a midnight.
fn nightfall(kelvin: u32) -> f64 {
    ((6500.0 - f64::from(kelvin)) / 3800.0).clamp(0.0, 1.0)
}

fn horizon(look: Look, size: i32) -> Vec<Layer> {
    let small = size <= SMALL;
    let (dusk, sky_top, sky_low, land, sun) = match look {
        Look::Warmed(kelvin) => {
            let dusk = nightfall(kelvin);
            (
                dusk,
                mix(hex(0x2F9BEA), hex(0x33358A), dusk),
                mix(hex(0xA8E0F7), hex(0xC8507A), dusk),
                mix(hex(0x3D9A5B), hex(0x1F2A4A), dusk),
                // Toward gold as it sets: the sky behind it turns orange and
                // rose, and a sun the same colour as its sky is not there.
                mix(tint(kelvin), hex(0xFFD45A), 0.55 * dusk),
            )
        }
        Look::Off => (
            0.8,
            hex(0x6A6F78),
            hex(0xB5B9C0),
            hex(0x555A62),
            hex(0xE2E4E8),
        ),
    };
    let sun_y = -0.24 + 0.64 * dusk;
    let radius = if small { 0.46 } else { 0.40 };
    let badge = || circle(0.0, 0.0, 0.96);

    let mut stack = vec![Layer {
        shape: badge(),
        paint: Paint::Vertical {
            top: sky_top,
            bottom: sky_low,
            from: -0.96,
            to: 0.6,
        },
    }];
    // The halo is one more thing a small icon cannot afford.
    if !small {
        stack.push(Layer {
            shape: circle(0.0, sun_y, radius + 0.09),
            paint: Paint::Flat(mix(mix(sky_top, sky_low, 0.6), mix(sun, WHITE, 0.2), 0.45)),
        });
    }
    stack.push(Layer {
        shape: circle(0.0, sun_y, radius),
        paint: Paint::Vertical {
            top: if small { sun } else { mix(sun, WHITE, 0.22) },
            bottom: sun,
            from: sun_y - radius,
            to: sun_y + radius,
        },
    });
    stack.push(Layer {
        shape: Shape::Inside(Box::new(circle(0.0, 2.34, 1.72)), Box::new(badge())),
        paint: Paint::Flat(land),
    });
    stack
}

fn screen(look: Look, size: i32) -> Vec<Layer> {
    let colour = look.colour();
    let small = size <= SMALL;
    let (frame, display, neck, base, moon) = if small {
        // The display takes everything it can; it is the part with the news.
        (
            round_box(0.0, -0.14, 0.98, 0.74, 0.14),
            round_box(0.0, -0.14, 0.84, 0.60, 0.06),
            round_box(0.0, 0.68, 0.20, 0.14, 0.01),
            round_box(0.0, 0.86, 0.56, 0.11, 0.06),
            minus(circle(-0.08, -0.13, 0.44), circle(0.14, -0.29, 0.36)),
        )
    } else {
        (
            round_box(0.0, -0.16, 0.96, 0.70, 0.17),
            round_box(0.0, -0.16, 0.81, 0.55, 0.08),
            round_box(0.0, 0.64, 0.17, 0.16, 0.02),
            round_box(0.0, 0.84, 0.50, 0.075, 0.07),
            minus(circle(-0.06, -0.15, 0.38), circle(0.12, -0.29, 0.33)),
        )
    };
    let (body, ink) = match look {
        Look::Warmed(_) => (hex(0x39404F), NAVY),
        Look::Off => (hex(0x62666E), mix(GREY, BLACK, 0.5)),
    };
    vec![
        Layer {
            shape: Shape::Union(vec![frame, neck, base]),
            paint: Paint::Vertical {
                top: mix(body, WHITE, 0.14),
                bottom: body,
                from: -1.0,
                to: 1.0,
            },
        },
        Layer {
            shape: display,
            paint: Paint::Diagonal {
                // Less highlight when small: a pale top half disappears on a
                // light panel.
                light: mix(colour, WHITE, if small { 0.16 } else { 0.38 }),
                dark: mix(colour, BLACK, 0.12),
                spread: 1.0,
            },
        },
        Layer {
            shape: moon,
            paint: Paint::Flat(ink),
        },
    ]
}

fn layers(style: TrayIcon, look: Look, size: i32) -> Vec<Layer> {
    match style {
        TrayIcon::Plain => Vec::new(),
        TrayIcon::NightBadge => night_badge(look, size),
        TrayIcon::Horizon => horizon(look, size),
        TrayIcon::Screen => screen(look, size),
    }
}

fn to_byte(value: f64) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

/// One pixel of a stack of layers, as alpha, red, green, blue.
fn pixel(layers: &[Layer], x: f64, y: f64, pixel_width: f64) -> [u8; 4] {
    let mut colour = [0.0; 3];
    let mut alpha = 0.0;
    for layer in layers {
        let coverage = (0.5 - layer.shape.distance(x, y) / pixel_width).clamp(0.0, 1.0);
        if coverage <= 0.0 {
            continue;
        }
        let underneath = alpha * (1.0 - coverage);
        let total = coverage + underneath;
        let paint = layer.paint.at(x, y);
        for (out, new) in colour.iter_mut().zip(paint) {
            *out = (new * coverage + *out * underneath) / total;
        }
        alpha = total;
    }
    let [red, green, blue] = colour.map(to_byte);
    [to_byte(alpha * 255.0), red, green, blue]
}

/// A whole icon, in the order StatusNotifierItem wants: ARGB, big-endian,
/// row by row from the top.
fn painted(layers: &[Layer], size: i32) -> ksni::Icon {
    let pixel_width = 2.0 / f64::from(size);
    let centre = |index: i32| (f64::from(index) + 0.5) * pixel_width - 1.0;
    let data = (0..size)
        .flat_map(|row| (0..size).map(move |col| (row, col)))
        .flat_map(|(row, col)| pixel(layers, centre(col), centre(row), pixel_width))
        .collect();
    ksni::Icon {
        width: size,
        height: size,
        data,
    }
}

/// The icon at every size, or nothing for the plain one, which the theme
/// draws from its name instead.
pub fn pixmaps(style: TrayIcon, look: Look) -> Vec<ksni::Icon> {
    if style == TrayIcon::Plain {
        return Vec::new();
    }
    SIZES
        .iter()
        .map(|&size| painted(&layers(style, look, size), size))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOKS: [Look; 3] = [Look::Warmed(6500), Look::Warmed(2700), Look::Off];

    fn colour_styles() -> impl Iterator<Item = TrayIcon> {
        TrayIcon::ALL
            .into_iter()
            .filter(|style| *style != TrayIcon::Plain)
    }

    /// The largest of `style`'s icons in `look`.
    fn largest(style: TrayIcon, look: Look) -> ksni::Icon {
        pixmaps(style, look).pop().expect("a colour icon has sizes")
    }

    /// `icon`'s pixel at (`x`, `y`) in the -1..1 square, as `[a, r, g, b]`.
    fn sample(icon: &ksni::Icon, x: f64, y: f64) -> [u8; 4] {
        let cell = |v: f64| {
            let scaled = (v + 1.0) / 2.0 * f64::from(icon.width);
            (scaled as i32).clamp(0, icon.width - 1)
        };
        let at = (cell(y) * icon.width + cell(x)) as usize * 4;
        [
            icon.data[at],
            icon.data[at + 1],
            icon.data[at + 2],
            icon.data[at + 3],
        ]
    }

    /// Inside the night badge's crescent at every size above [`SMALL`].
    const IN_THE_CRESCENT: (f64, f64) = (-0.45, 0.25);

    /// The plain icon is the theme's to draw. Handing the panel pixels as
    /// well would leave which one it shows up to the panel.
    #[test]
    fn the_plain_icon_paints_nothing() {
        for look in LOOKS {
            assert!(pixmaps(TrayIcon::Plain, look).is_empty());
        }
    }

    #[test]
    fn a_colour_icon_comes_in_every_size_with_every_byte() {
        for style in colour_styles() {
            for look in LOOKS {
                let icons = pixmaps(style, look);
                assert_eq!(icons.len(), SIZES.len());
                for (icon, size) in icons.iter().zip(SIZES) {
                    assert_eq!((icon.width, icon.height), (size, size));
                    assert_eq!(icon.data.len(), (size * size * 4) as usize);
                }
            }
        }
    }

    /// A panel is any colour at all, so what is not the icon must be clear.
    /// Alpha is the first byte; read in the wrong order this fails too.
    #[test]
    fn the_corners_are_clear_and_the_middle_is_not() {
        for style in colour_styles() {
            for look in LOOKS {
                let icon = largest(style, look);
                assert_eq!(sample(&icon, -1.0, -1.0)[0], 0, "{style:?}");
                assert_eq!(sample(&icon, 1.0, -1.0)[0], 0, "{style:?}");
                assert_eq!(sample(&icon, 0.0, 0.0)[0], 255, "{style:?}");
            }
        }
    }

    /// The feature, in one assertion. Green is the channel that falls as
    /// light warms; red stays near the top throughout.
    #[test]
    fn the_crescent_warms_with_the_screen() {
        let (x, y) = IN_THE_CRESCENT;
        let day = sample(&largest(TrayIcon::NightBadge, Look::Warmed(6500)), x, y);
        let night = sample(&largest(TrayIcon::NightBadge, Look::Warmed(2700)), x, y);
        assert!(night[2] < day[2], "day {day:?}, night {night:?}");
        assert!(night[1] > night[3], "night should be red, not blue");
    }

    #[test]
    fn a_filter_that_is_off_is_grey() {
        for style in colour_styles() {
            let icon = largest(style, Look::Off);
            for (x, y) in [(0.0, 0.0), IN_THE_CRESCENT, (0.0, -0.7)] {
                let [_, red, green, blue] = sample(&icon, x, y);
                let spread = red.max(green).max(blue) - red.min(green).min(blue);
                assert!(
                    spread < 24,
                    "{style:?} at ({x}, {y}) should carry no warmth: {red},{green},{blue}"
                );
            }
        }
    }

    /// Where the sun stands at noon is empty sky by night.
    #[test]
    fn the_sun_goes_down_on_the_horizon() {
        let (x, y) = (0.0, -0.24);
        let day = sample(&largest(TrayIcon::Horizon, Look::Warmed(6500)), x, y);
        let night = sample(&largest(TrayIcon::Horizon, Look::Warmed(2700)), x, y);
        assert!(day[2] > night[2] + 60, "day {day:?}, night {night:?}");
    }

    #[test]
    fn the_display_takes_the_colour_of_the_temperature() {
        let (x, y) = (-0.7, 0.2);
        let day = sample(&largest(TrayIcon::Screen, Look::Warmed(6500)), x, y);
        let night = sample(&largest(TrayIcon::Screen, Look::Warmed(2700)), x, y);
        assert!(night[2] < day[2], "day {day:?}, night {night:?}");
    }

    #[test]
    fn a_warm_setting_is_a_full_night_and_a_mild_one_is_a_dusk() {
        assert_eq!(nightfall(6500), 0.0);
        assert_eq!(nightfall(9000), 0.0);
        assert_eq!(nightfall(2700), 1.0);
        assert_eq!(nightfall(1700), 1.0);
        assert!((0.3..0.7).contains(&nightfall(4500)));
    }

    #[test]
    fn a_round_box_measures_from_its_edge() {
        let shape = round_box(0.0, 0.0, 0.5, 0.25, 0.05);
        assert!((shape.distance(0.0, 0.0) + 0.25).abs() < 1e-9);
        assert!((shape.distance(1.0, 0.0) - 0.5).abs() < 1e-9);
        // The rounded corner leaves the exact corner just outside.
        assert!(shape.distance(0.5, 0.25) > 0.0);
    }

    #[test]
    fn a_change_too_small_to_see_is_not_a_change() {
        assert_eq!(Look::warmed(3302), Look::warmed(3348));
        assert_ne!(Look::warmed(3348), Look::warmed(3351));
    }

    /// `--temp 9000` and `--temp 1000` are both legal; the table ending is
    /// not a reason for the icon to go black.
    #[test]
    fn the_colour_stops_at_the_ends_of_the_table() {
        assert_eq!(tint(1000), tint(1700));
        assert_eq!(tint(9000), tint(6500));
    }

    #[test]
    fn the_colour_runs_through_every_stop() {
        for (kelvin, colour) in WARMTH {
            assert_eq!(tint(kelvin as u32), colour);
        }
    }
}
