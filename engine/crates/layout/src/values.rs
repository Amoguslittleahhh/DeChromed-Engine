//! B1/B2: resolving computed-style strings into real pixel numbers -- the
//! "used value" resolution A6/A7 documented as out of scope until a real
//! layout phase existed to consume it against actual box geometry.
//!
//! Reference: <https://www.w3.org/TR/css-values-4/>
//!
//! Known gap: only the units/keywords layout actually needs are handled
//! (`px`/`em`/`rem`/`pt`/`%`, `auto`, the `font-size` keyword table,
//! `border-width`'s `thin`/`medium`/`thick`). Other absolute units (`cm`,
//! `in`, `mm`, `pc`, `q`), viewport units (`vw`/`vh`), and `calc()` aren't
//! implemented; an unrecognized length falls back to `0.0`/the caller's own
//! fallback rather than panicking -- a real UA would need much more of the
//! value grammar than this phase's scope covers, and that's stated plainly
//! here rather than left to be discovered by surprise.

/// CSS's `medium` `font-size` keyword resolves to this many pixels, the
/// same "1rem = 16px" default every real UA ships.
const DEFAULT_FONT_SIZE_PX: f64 = 16.0;

pub fn default_font_size_px() -> f64 {
    DEFAULT_FONT_SIZE_PX
}

/// Resolves a `<length>` to CSS pixels, or `None` if `value` isn't a length
/// this resolver understands.
pub fn parse_length_px(value: &str, font_size_px: f64, root_font_size_px: f64) -> Option<f64> {
    let value = value.trim();
    if value == "0" {
        return Some(0.0);
    }
    if let Some(n) = value.strip_suffix("px") {
        return n.trim().parse::<f64>().ok();
    }
    if let Some(n) = value.strip_suffix("rem") {
        return n.trim().parse::<f64>().ok().map(|n| n * root_font_size_px);
    }
    if let Some(n) = value.strip_suffix("em") {
        return n.trim().parse::<f64>().ok().map(|n| n * font_size_px);
    }
    if let Some(n) = value.strip_suffix("pt") {
        // 1pt = 1/72in, 1px = 1/96in (the CSS reference pixel).
        return n.trim().parse::<f64>().ok().map(|n| n * 96.0 / 72.0);
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LengthPercentageAuto {
    Length(f64),
    /// 0.0-100.0, as written (`50%` -> `50.0`).
    Percentage(f64),
    Auto,
}

/// Resolves a `<length-percentage>` or `auto` value; unrecognized input
/// (an unhandled unit, a keyword this resolver doesn't know) falls back to
/// `Auto` rather than panicking or silently becoming `0`.
pub fn parse_length_percentage_auto(
    value: &str,
    font_size_px: f64,
    root_font_size_px: f64,
) -> LengthPercentageAuto {
    let value = value.trim();
    if value == "auto" {
        return LengthPercentageAuto::Auto;
    }
    if let Some(n) = value.strip_suffix('%')
        && let Ok(n) = n.trim().parse::<f64>()
    {
        return LengthPercentageAuto::Percentage(n);
    }
    match parse_length_px(value, font_size_px, root_font_size_px) {
        Some(px) => LengthPercentageAuto::Length(px),
        None => LengthPercentageAuto::Auto,
    }
}

/// `border-width`'s keyword values plus real lengths; used for the single
/// (non-per-side) `border-width` longhand in `css::cascade`'s property
/// table.
pub fn parse_border_width_px(value: &str, font_size_px: f64, root_font_size_px: f64) -> f64 {
    match value.trim() {
        "thin" => 1.0,
        "medium" => 3.0,
        "thick" => 5.0,
        other => parse_length_px(other, font_size_px, root_font_size_px).unwrap_or(0.0),
    }
}

/// Resolves `font-size`'s absolute keywords (the CSS2.1 ratio table, all
/// relative to `medium` = 16px), `larger`/`smaller` (relative to the
/// *parent's* resolved font-size), percentages (also parent-relative), and
/// ordinary lengths.
pub fn resolve_font_size_px(value: &str, parent_font_size_px: f64, root_font_size_px: f64) -> f64 {
    match value.trim() {
        "xx-small" => DEFAULT_FONT_SIZE_PX * 3.0 / 5.0,
        "x-small" => DEFAULT_FONT_SIZE_PX * 3.0 / 4.0,
        "small" => DEFAULT_FONT_SIZE_PX * 8.0 / 9.0,
        "medium" => DEFAULT_FONT_SIZE_PX,
        "large" => DEFAULT_FONT_SIZE_PX * 6.0 / 5.0,
        "x-large" => DEFAULT_FONT_SIZE_PX * 3.0 / 2.0,
        "xx-large" => DEFAULT_FONT_SIZE_PX * 2.0,
        "larger" => parent_font_size_px * 1.2,
        "smaller" => parent_font_size_px / 1.2,
        other => {
            if let Some(n) = other.strip_suffix('%')
                && let Ok(n) = n.trim().parse::<f64>()
            {
                return parent_font_size_px * n / 100.0;
            }
            parse_length_px(other, parent_font_size_px, root_font_size_px)
                .unwrap_or(parent_font_size_px)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths() {
        assert_eq!(parse_length_px("10px", 16.0, 16.0), Some(10.0));
        assert_eq!(parse_length_px("2em", 16.0, 16.0), Some(32.0));
        assert_eq!(parse_length_px("2rem", 20.0, 16.0), Some(32.0));
        assert_eq!(parse_length_px("1pt", 16.0, 16.0), Some(96.0 / 72.0));
        assert_eq!(parse_length_px("0", 16.0, 16.0), Some(0.0));
        assert_eq!(parse_length_px("garbage", 16.0, 16.0), None);
    }

    #[test]
    fn length_percentage_auto() {
        assert_eq!(
            parse_length_percentage_auto("auto", 16.0, 16.0),
            LengthPercentageAuto::Auto
        );
        assert_eq!(
            parse_length_percentage_auto("50%", 16.0, 16.0),
            LengthPercentageAuto::Percentage(50.0)
        );
        assert_eq!(
            parse_length_percentage_auto("10px", 16.0, 16.0),
            LengthPercentageAuto::Length(10.0)
        );
        assert_eq!(
            parse_length_percentage_auto("nonsense", 16.0, 16.0),
            LengthPercentageAuto::Auto
        );
    }

    #[test]
    fn font_size_keywords() {
        assert_eq!(resolve_font_size_px("medium", 16.0, 16.0), 16.0);
        assert_eq!(resolve_font_size_px("large", 16.0, 16.0), 16.0 * 6.0 / 5.0);
        assert_eq!(resolve_font_size_px("larger", 20.0, 16.0), 24.0);
        assert_eq!(resolve_font_size_px("150%", 20.0, 16.0), 30.0);
        assert_eq!(resolve_font_size_px("18px", 20.0, 16.0), 18.0);
    }

    #[test]
    fn border_width_keywords() {
        assert_eq!(parse_border_width_px("thin", 16.0, 16.0), 1.0);
        assert_eq!(parse_border_width_px("medium", 16.0, 16.0), 3.0);
        assert_eq!(parse_border_width_px("thick", 16.0, 16.0), 5.0);
        assert_eq!(parse_border_width_px("7px", 16.0, 16.0), 7.0);
    }
}
