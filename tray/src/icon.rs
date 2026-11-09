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

/// Which icon the tray wears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// The icon theme's own monochrome symbol, and the default: nobody's
    /// panel changes unless they ask it to.
    Plain,
    /// A navy disc holding a crescent in the screen's colour.
    NightBadge,
}

impl Style {
    /// Every style, in menu order.
    pub const ALL: [Style; 2] = [Style::Plain, Style::NightBadge];

    /// What the menu calls it.
    pub fn label(self) -> &'static str {
        match self {
            Style::Plain => "Plain",
            Style::NightBadge => "Night badge",
        }
    }

    /// What the choice is remembered as. Separate from the label so that
    /// rewording the menu cannot forget anybody's choice.
    pub fn name(self) -> &'static str {
        match self {
            Style::Plain => "plain",
            Style::NightBadge => "night-badge",
        }
    }

    pub fn from_name(name: &str) -> Option<Style> {
        Style::ALL.into_iter().find(|style| style.name() == name)
    }

    /// Where it sits in the menu.
    pub fn index(self) -> usize {
        Style::ALL
            .iter()
            .position(|style| *style == self)
            .unwrap_or(0)
    }
}

/// An outline, answering how far any point is from its edge.
enum Shape {
    Circle { x: f64, y: f64, r: f64 },
    Union(Vec<Shape>),
    Minus(Box<Shape>, Box<Shape>),
}

fn circle(x: f64, y: f64, r: f64) -> Shape {
    Shape::Circle { x, y, r }
}

fn minus(shape: Shape, taken: Shape) -> Shape {
    Shape::Minus(Box::new(shape), Box::new(taken))
}

impl Shape {
    fn distance(&self, px: f64, py: f64) -> f64 {
        match self {
            Shape::Circle { x, y, r } => (px - x).hypot(py - y) - r,
            Shape::Union(parts) => parts
                .iter()
                .map(|part| part.distance(px, py))
                .fold(f64::INFINITY, f64::min),
            Shape::Minus(shape, taken) => shape.distance(px, py).max(-taken.distance(px, py)),
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
            Paint::Diagonal { light, dark } => {
                mix(*light, *dark, ((x + y + 1.2) / 2.4).clamp(0.0, 1.0))
            }
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
            },
        },
        Layer {
            shape: stars,
            paint: Paint::Flat(mix(colour, WHITE, 0.55)),
        },
    ]
}

fn layers(style: Style, look: Look, size: i32) -> Vec<Layer> {
    match style {
        Style::Plain => Vec::new(),
        Style::NightBadge => night_badge(look, size),
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
pub fn pixmaps(style: Style, look: Look) -> Vec<ksni::Icon> {
    if style == Style::Plain {
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

    fn colour_styles() -> impl Iterator<Item = Style> {
        Style::ALL
            .into_iter()
            .filter(|style| *style != Style::Plain)
    }

    /// The largest of `style`'s icons in `look`.
    fn largest(style: Style, look: Look) -> ksni::Icon {
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

    #[test]
    fn every_choice_can_be_read_back_from_its_name() {
        for style in Style::ALL {
            assert_eq!(Style::from_name(style.name()), Some(style));
        }
    }

    /// A remembered name that no longer exists, or that somebody typed into
    /// the file, is no choice at all rather than an error.
    #[test]
    fn a_name_nobody_knows_is_no_choice() {
        assert_eq!(Style::from_name("sparkles"), None);
        assert_eq!(Style::from_name(""), None);
    }

    #[test]
    fn the_menu_position_of_a_style_is_where_it_is_listed() {
        for (position, style) in Style::ALL.into_iter().enumerate() {
            assert_eq!(style.index(), position);
        }
    }

    /// The plain icon is the theme's to draw. Handing the panel pixels as
    /// well would leave which one it shows up to the panel.
    #[test]
    fn the_plain_icon_paints_nothing() {
        for look in LOOKS {
            assert!(pixmaps(Style::Plain, look).is_empty());
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
        let day = sample(&largest(Style::NightBadge, Look::Warmed(6500)), x, y);
        let night = sample(&largest(Style::NightBadge, Look::Warmed(2700)), x, y);
        assert!(night[2] < day[2], "day {day:?}, night {night:?}");
        assert!(night[1] > night[3], "night should be red, not blue");
    }

    #[test]
    fn a_filter_that_is_off_is_grey() {
        let (x, y) = IN_THE_CRESCENT;
        let [_, red, green, blue] = sample(&largest(Style::NightBadge, Look::Off), x, y);
        let spread = red.max(green).max(blue) - red.min(green).min(blue);
        assert!(
            spread < 24,
            "off should carry no warmth: {red},{green},{blue}"
        );
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
