//! Real text shaping and glyph outline extraction, shared between
//! `layout` (which needs real advance widths to measure text) and `paint`
//! (which needs real glyph outlines to actually paint it) -- living in its
//! own crate specifically so both can depend on it without `paint`
//! depending on `layout` in the wrong direction (`paint` already depends
//! on `layout` for `Rect`/`Fragment`, so `layout` can't depend back on
//! `paint`).
//!
//! **Real technology, not a hand-rolled approximation:** shaping is
//! [`rustybuzz`], a complete Rust port of HarfBuzz -- the same shaping
//! engine real browsers (via HarfBuzz itself) and other production Rust
//! text stacks (e.g. `resvg`, Servo's own font stack) actually use. Glyph
//! outlines come from [`ttf-parser`], a widely-used, spec-conformant
//! TrueType/OpenType parser. This replaces the earlier B10 landing's flat
//! per-character-table width *approximation* with genuine shaping: real
//! kerning and ligatures for the embedded font's script coverage, driven
//! by the font's own GSUB/GPOS tables rather than guessed ratios.
//!
//! **The one font this engine ships:** `assets/fonts/DejaVuSans.ttf`
//! (embedded via `include_bytes!`, see that directory's own `README.md`
//! for its license). There's no font matching/fallback chain, no
//! `@font-face` loading, and no variable-font support -- every glyph this
//! crate shapes or outlines comes from this one embedded font, which is
//! still a real, substantial step up from a synthetic per-character width
//! table: it's an actual font's actual hinting-free outline and metric
//! data, not an approximation of "some" font.
//!
//! **Known gaps:** only whatever scripts DejaVu Sans itself covers shape
//! correctly (no complex-script-specific fallback fonts for scripts it
//! doesn't cover); no font fallback when a codepoint is missing from the
//! embedded font (rustybuzz still shapes it, typically to a `.notdef`
//! glyph, rather than falling back to a different font); no hinting
//! (outlines are used at their native resolution, scaled, with no
//! grid-fitting).

use ttf_parser::OutlineBuilder;

/// The one embedded font this crate ships -- see module docs.
pub static FONT_BYTES: &[u8] = include_bytes!("../../../assets/fonts/DejaVuSans.ttf");

/// A parsed, shapeable font -- a thin wrapper around a [`rustybuzz::Face`]
/// (which itself derefs to a [`ttf_parser::Face`] for outline/metric
/// access).
pub struct Font {
    face: rustybuzz::Face<'static>,
}

impl Font {
    /// The embedded DejaVu Sans font -- see module docs for why this is
    /// the only font available today.
    pub fn dejavu_sans() -> Font {
        Font {
            face: rustybuzz::Face::from_slice(FONT_BYTES, 0)
                .expect("the embedded DejaVu Sans font must parse"),
        }
    }

    pub fn units_per_em(&self) -> i32 {
        self.face.units_per_em()
    }

    /// The font's own real ascent, in font units above the baseline --
    /// what a real renderer needs to place a shaped run's baseline
    /// correctly within its line box (rather than guessing a fixed
    /// fraction of the font size).
    pub fn ascender(&self) -> i16 {
        self.face.ascender()
    }
}

/// A process-lifetime-cached [`Font::dejavu_sans`] -- shaping/outline
/// extraction against the embedded font is real, non-trivial work
/// (parsing the whole TTF table set), and both `layout` (measurement)
/// and `paint` (rendering) need it, so this is the one place that parsing
/// happens rather than each consumer crate hand-rolling its own
/// `OnceLock` around a second call to [`Font::dejavu_sans`].
pub fn default_font() -> &'static Font {
    static FONT: std::sync::OnceLock<Font> = std::sync::OnceLock::new();
    FONT.get_or_init(Font::dejavu_sans)
}

/// One shaped glyph: which glyph (not which character -- shaping can
/// merge/reorder/substitute characters into glyphs, e.g. ligatures), and
/// its real advance/offset in CSS pixels at the font size shaping was
/// requested at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapedGlyph {
    pub glyph_id: u16,
    pub x_advance: f64,
    pub x_offset: f64,
    pub y_offset: f64,
}

/// The real result of shaping a run of text: its glyphs, in visual
/// (post-shaping) order, and their total advance width.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShapedRun {
    pub glyphs: Vec<ShapedGlyph>,
    pub width_px: f64,
}

/// Real HarfBuzz-equivalent shaping (via `rustybuzz`) of `text` at
/// `font_size_px`, using `font`'s own GSUB/GPOS tables for kerning and
/// ligature substitution -- not an approximation. `guess_segment_
/// properties()` infers `text`'s script/language/direction the same way
/// a caller with no better information would; a caller that already knows
/// these (e.g. from CSS `direction`) should prefer `shape_with_direction`.
pub fn shape(font: &Font, text: &str, font_size_px: f64) -> ShapedRun {
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    shape_buffer(font, buffer, font_size_px)
}

/// Like [`shape`], but with an explicit direction (`ltr` maps to
/// left-to-right, anything else to right-to-left) instead of guessing one
/// from the text -- what a caller that already knows the CSS `direction`
/// in effect (B8) should use, since shaping RTL text left-to-right (or
/// vice versa) produces the wrong glyph order/mirroring for
/// direction-sensitive scripts.
pub fn shape_with_direction(font: &Font, text: &str, font_size_px: f64, ltr: bool) -> ShapedRun {
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.guess_segment_properties();
    buffer.set_direction(if ltr {
        rustybuzz::Direction::LeftToRight
    } else {
        rustybuzz::Direction::RightToLeft
    });
    shape_buffer(font, buffer, font_size_px)
}

fn shape_buffer(font: &Font, buffer: rustybuzz::UnicodeBuffer, font_size_px: f64) -> ShapedRun {
    let output = rustybuzz::shape(&font.face, &[], buffer);
    let scale = font_size_px / font.units_per_em() as f64;
    let infos = output.glyph_infos();
    let positions = output.glyph_positions();
    let mut glyphs = Vec::with_capacity(infos.len());
    let mut width_px = 0.0;
    for (info, pos) in infos.iter().zip(positions.iter()) {
        let x_advance = pos.x_advance as f64 * scale;
        glyphs.push(ShapedGlyph {
            glyph_id: info.glyph_id as u16,
            x_advance,
            x_offset: pos.x_offset as f64 * scale,
            y_offset: pos.y_offset as f64 * scale,
        });
        width_px += x_advance;
    }
    ShapedRun { glyphs, width_px }
}

/// A real glyph outline: one or more closed contours, each a polyline in
/// **font units** (not yet scaled to a font size -- see [`Font::
/// units_per_em`]), with curves (`ttf-parser` reports both quadratic
/// TrueType and cubic CFF/OpenType curves) flattened to straight
/// segments. Every contour uses the nonzero winding rule, matching both
/// TrueType's own real rasterization convention and
/// `paint::canvas2d::fill`'s existing nonzero-winding scanline fill --
/// so a glyph outline can be filled with that same real algorithm with no
/// special-casing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GlyphOutline {
    pub contours: Vec<Vec<(f64, f64)>>,
}

/// Real outline extraction (via `ttf-parser`) for `glyph_id` in `font`,
/// flattening any curves along the way. `None` if the font has no outline
/// for this glyph (e.g. `.notdef`, or a genuinely empty glyph like space).
///
/// `scale` is the same `font_size_px / font.units_per_em()` factor a
/// caller is about to apply to these (still font-unit) coordinates when
/// actually rendering them -- it's used only to pick how finely each
/// curve gets flattened (see [`segments_for`]), so a tiny glyph doesn't
/// pay for detail no pixel will ever show, and a huge one doesn't come
/// out visibly faceted. Pass `1.0` if the eventual render scale isn't
/// known yet; that reproduces this function's original fixed-8-segment
/// behavior at ordinary body-text scales (font-unit deviation happens to
/// land in the same range `segments_for` would pick for a ~16px glyph).
pub fn glyph_outline(font: &Font, glyph_id: u16, scale: f64) -> Option<GlyphOutline> {
    let mut collector = OutlineCollector {
        scale,
        ..OutlineCollector::default()
    };
    font.face
        .outline_glyph(ttf_parser::GlyphId(glyph_id), &mut collector)?;
    collector.finish();
    Some(GlyphOutline {
        contours: collector.contours,
    })
}

/// No single flattened curve segment should span more than this many
/// *device* pixels (post-scale) -- the real tolerance a scale-aware
/// flattener should target, replacing a single fixed subdivision count
/// that was either too coarse (visibly faceted rounded letterforms like
/// `o`/`e` at large sizes) or wastefully fine (tiny glyphs subdividing
/// curves no pixel could ever resolve) at every size but one.
const MAX_PX_PER_CURVE_SEGMENT: f64 = 3.0;
/// Clamped so a degenerate (near-zero-length or absurdly long) curve still
/// gets a sane, bounded amount of work.
const MIN_CURVE_SEGMENTS: usize = 2;
const MAX_CURVE_SEGMENTS: usize = 32;

/// Picks a curve's subdivision count from its rough on-screen (device
/// pixel) chord length at `scale`, instead of one constant used at every
/// font size.
fn segments_for(p0: (f64, f64), p_end: (f64, f64), scale: f64) -> usize {
    let device_px_length = ((p_end.0 - p0.0).hypot(p_end.1 - p0.1) * scale).abs();
    let segments = (device_px_length / MAX_PX_PER_CURVE_SEGMENT).ceil() as usize;
    segments.clamp(MIN_CURVE_SEGMENTS, MAX_CURVE_SEGMENTS)
}

#[derive(Default)]
struct OutlineCollector {
    contours: Vec<Vec<(f64, f64)>>,
    current: Vec<(f64, f64)>,
    start: (f64, f64),
    last: (f64, f64),
    scale: f64,
}

impl OutlineCollector {
    fn finish(&mut self) {
        if !self.current.is_empty() {
            self.contours.push(std::mem::take(&mut self.current));
        }
    }
}

impl OutlineBuilder for OutlineCollector {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish();
        let p = (x as f64, y as f64);
        self.current.push(p);
        self.start = p;
        self.last = p;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = (x as f64, y as f64);
        self.current.push(p);
        self.last = p;
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let p0 = self.last;
        let p1 = (x1 as f64, y1 as f64);
        let p2 = (x as f64, y as f64);
        let segments = segments_for(p0, p2, self.scale);
        for i in 1..=segments {
            let t = i as f64 / segments as f64;
            let mt = 1.0 - t;
            let px = mt * mt * p0.0 + 2.0 * mt * t * p1.0 + t * t * p2.0;
            let py = mt * mt * p0.1 + 2.0 * mt * t * p1.1 + t * t * p2.1;
            self.current.push((px, py));
        }
        self.last = p2;
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let p0 = self.last;
        let p1 = (x1 as f64, y1 as f64);
        let p2 = (x2 as f64, y2 as f64);
        let p3 = (x as f64, y as f64);
        let segments = segments_for(p0, p3, self.scale);
        for i in 1..=segments {
            let t = i as f64 / segments as f64;
            let mt = 1.0 - t;
            let px = mt * mt * mt * p0.0
                + 3.0 * mt * mt * t * p1.0
                + 3.0 * mt * t * t * p2.0
                + t * t * t * p3.0;
            let py = mt * mt * mt * p0.1
                + 3.0 * mt * mt * t * p1.1
                + 3.0 * mt * t * t * p2.1
                + t * t * t * p3.1;
            self.current.push((px, py));
        }
        self.last = p3;
    }

    fn close(&mut self) {
        if !self.current.is_empty() {
            self.current.push(self.start);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_font_parses() {
        let font = Font::dejavu_sans();
        assert!(font.units_per_em() > 0);
    }

    #[test]
    fn shaping_empty_text_produces_no_glyphs() {
        let font = Font::dejavu_sans();
        let run = shape(&font, "", 16.0);
        assert!(run.glyphs.is_empty());
        assert_eq!(run.width_px, 0.0);
    }

    #[test]
    fn shaping_produces_one_glyph_per_character_for_plain_latin_text() {
        let font = Font::dejavu_sans();
        let run = shape(&font, "hi", 16.0);
        assert_eq!(run.glyphs.len(), 2);
        assert!(run.width_px > 0.0);
    }

    #[test]
    fn wider_characters_shape_to_a_wider_run() {
        let font = Font::dejavu_sans();
        let narrow = shape(&font, "iiii", 16.0);
        let wide = shape(&font, "mmmm", 16.0);
        // A real font's own metrics, not a guessed ratio: DejaVu Sans's
        // "m" is genuinely wider than its "i".
        assert!(wide.width_px > narrow.width_px);
    }

    #[test]
    fn shaping_scales_linearly_with_font_size() {
        let font = Font::dejavu_sans();
        let at_16 = shape(&font, "hello", 16.0);
        let at_32 = shape(&font, "hello", 32.0);
        assert!((at_32.width_px - at_16.width_px * 2.0).abs() < 0.01);
    }

    #[test]
    fn rtl_direction_does_not_panic_and_still_advances() {
        let font = Font::dejavu_sans();
        let run = shape_with_direction(&font, "hello", 16.0, false);
        assert_eq!(run.glyphs.len(), 5);
        assert!(run.width_px > 0.0);
    }

    #[test]
    fn a_visible_glyph_has_a_nonempty_outline() {
        let font = Font::dejavu_sans();
        let run = shape(&font, "o", 16.0);
        let outline =
            glyph_outline(&font, run.glyphs[0].glyph_id, 1.0).expect("'o' has an outline");
        assert!(!outline.contours.is_empty());
        // "o" is a real letterform with a hole -- two contours (outer +
        // inner), same nonzero-winding-hole story as canvas2d's own donut
        // test.
        assert_eq!(outline.contours.len(), 2);
    }

    #[test]
    fn curve_flattening_gets_finer_at_a_larger_render_scale() {
        // A real scale-aware flattener should subdivide a curve more at a
        // larger scale (more device pixels for the same font-unit curve
        // to potentially look faceted over) and less at a tiny one --
        // not the same fixed segment count regardless of how big the
        // glyph will actually be rendered.
        let font = Font::dejavu_sans();
        let run = shape(&font, "o", 16.0);
        let glyph_id = run.glyphs[0].glyph_id;
        let tiny = glyph_outline(&font, glyph_id, 0.01).unwrap();
        let huge = glyph_outline(&font, glyph_id, 50.0).unwrap();
        let tiny_points: usize = tiny.contours.iter().map(|c| c.len()).sum();
        let huge_points: usize = huge.contours.iter().map(|c| c.len()).sum();
        assert!(
            huge_points > tiny_points,
            "expected more flattened points at a larger scale: tiny={tiny_points} huge={huge_points}"
        );
    }

    #[test]
    fn a_space_glyph_has_no_outline() {
        let font = Font::dejavu_sans();
        let run = shape(&font, " ", 16.0);
        let outline = glyph_outline(&font, run.glyphs[0].glyph_id, 1.0);
        assert!(outline.is_none_or(|o| o.contours.is_empty()));
    }

    #[test]
    fn every_ascii_printable_character_shapes_without_panicking() {
        let font = Font::dejavu_sans();
        for c in (0x20u8..0x7f).map(char::from) {
            let run = shape(&font, &c.to_string(), 16.0);
            for glyph in &run.glyphs {
                let _ = glyph_outline(&font, glyph.glyph_id, 1.0);
            }
        }
    }
}
