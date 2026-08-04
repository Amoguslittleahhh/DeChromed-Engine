//! A real (if small) CSS `<color>` parser -- the piece A6's cascade
//! module explicitly deferred ("genuinely correct value parsing
//! per-property... is its own large surface deferred to whichever
//! layout/paint phase first needs a given property's real typed value").
//! Paint (B9's display-list lowering) is that phase for `color`.
//!
//! Reference: <https://www.w3.org/TR/css-color-4/>.
//!
//! Implemented: a modest table of named colors (not the full CSS named-
//! color list, which has 148 entries), `#rgb`/`#rrggbb`/`#rrggbbaa`,
//! classic comma-separated `rgb()`/`rgba()` functional notation, and
//! `transparent`. `currentcolor` is deliberately *not* resolved here --
//! it's handled by the caller simply not overriding the inherited color
//! when this parser returns `None` for it, which is exactly the correct
//! fallback behavior for a value meaning "same as the inherited color".
//!
//! Known gaps: no `hsl()`/`hwb()`/`lab()`/`lch()`/`color()`, no CSS Color
//! 4's space-separated functional syntax (`rgb(0 0 0 / 50%)`), no
//! `%`-based rgb() channel values, and the named-color table only covers
//! commonly-used names -- an unrecognized name/format returns `None`
//! rather than guessing.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b, a: 255 }
    }

    pub const TRANSPARENT: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };
    pub const BLACK: Color = Color::rgb(0, 0, 0);
}

const NAMED_COLORS: &[(&str, Color)] = &[
    ("black", Color::rgb(0, 0, 0)),
    ("white", Color::rgb(255, 255, 255)),
    ("red", Color::rgb(255, 0, 0)),
    ("green", Color::rgb(0, 128, 0)),
    ("blue", Color::rgb(0, 0, 255)),
    ("yellow", Color::rgb(255, 255, 0)),
    ("cyan", Color::rgb(0, 255, 255)),
    ("aqua", Color::rgb(0, 255, 255)),
    ("magenta", Color::rgb(255, 0, 255)),
    ("fuchsia", Color::rgb(255, 0, 255)),
    ("gray", Color::rgb(128, 128, 128)),
    ("grey", Color::rgb(128, 128, 128)),
    ("silver", Color::rgb(192, 192, 192)),
    ("orange", Color::rgb(255, 165, 0)),
    ("purple", Color::rgb(128, 0, 128)),
    ("pink", Color::rgb(255, 192, 203)),
    ("brown", Color::rgb(165, 42, 42)),
    ("navy", Color::rgb(0, 0, 128)),
    ("teal", Color::rgb(0, 128, 128)),
    ("lime", Color::rgb(0, 255, 0)),
    ("maroon", Color::rgb(128, 0, 0)),
    ("olive", Color::rgb(128, 128, 0)),
    // `color`'s and `border-color`'s initial values, per `css::cascade`'s
    // property table -- resolved as plain black, an approximation of the
    // real "system UI text color" `canvastext` actually means.
    ("canvastext", Color::rgb(0, 0, 0)),
];

pub fn parse_color(value: &str) -> Option<Color> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("transparent") {
        return Some(Color::TRANSPARENT);
    }
    if let Some(hex) = value.strip_prefix('#') {
        return parse_hex(hex);
    }
    if let Some(inner) = value
        .strip_prefix("rgba(")
        .or_else(|| value.strip_prefix("rgb("))
    {
        return parse_rgb_function(inner.strip_suffix(')')?);
    }
    NAMED_COLORS
        .iter()
        .find(|(name, _)| value.eq_ignore_ascii_case(name))
        .map(|(_, c)| *c)
}

fn parse_hex(hex: &str) -> Option<Color> {
    let digit_pair = |s: &str| u8::from_str_radix(s, 16).ok();
    match hex.len() {
        3 => {
            let r = digit_pair(&hex[0..1].repeat(2))?;
            let g = digit_pair(&hex[1..2].repeat(2))?;
            let b = digit_pair(&hex[2..3].repeat(2))?;
            Some(Color::rgb(r, g, b))
        }
        6 => Some(Color::rgb(
            digit_pair(&hex[0..2])?,
            digit_pair(&hex[2..4])?,
            digit_pair(&hex[4..6])?,
        )),
        8 => Some(Color {
            r: digit_pair(&hex[0..2])?,
            g: digit_pair(&hex[2..4])?,
            b: digit_pair(&hex[4..6])?,
            a: digit_pair(&hex[6..8])?,
        }),
        _ => None,
    }
}

fn parse_rgb_function(inner: &str) -> Option<Color> {
    let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
    if parts.len() != 3 && parts.len() != 4 {
        return None;
    }
    let channel = |s: &str| s.parse::<f64>().ok().map(|n| n.clamp(0.0, 255.0) as u8);
    let r = channel(parts[0])?;
    let g = channel(parts[1])?;
    let b = channel(parts[2])?;
    let a = if parts.len() == 4 {
        parts[3]
            .parse::<f64>()
            .ok()
            .map(|n| (n.clamp(0.0, 1.0) * 255.0).round() as u8)?
    } else {
        255
    };
    Some(Color { r, g, b, a })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_colors() {
        assert_eq!(parse_color("red"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_color("RED"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_color("transparent"), Some(Color::TRANSPARENT));
        assert_eq!(parse_color("not-a-color"), None);
        assert_eq!(parse_color("currentcolor"), None);
    }

    #[test]
    fn hex_colors() {
        assert_eq!(parse_color("#f00"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(parse_color("#ff0000"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(
            parse_color("#ff000080"),
            Some(Color {
                r: 255,
                g: 0,
                b: 0,
                a: 128
            })
        );
    }

    #[test]
    fn rgb_function() {
        assert_eq!(parse_color("rgb(255, 0, 0)"), Some(Color::rgb(255, 0, 0)));
        assert_eq!(
            parse_color("rgba(255, 0, 0, 0.5)"),
            Some(Color {
                r: 255,
                g: 0,
                b: 0,
                a: 128
            })
        );
    }
}
