//! B11: a software rasterizer -- the real baseline both Blink (Skia) and
//! Gecko (WebRender) also bootstrap new platforms from before adding a
//! GPU path, per `ROADMAP.md`'s own B11 entry. Turns a [`DisplayList`]
//! into a real RGBA8 pixel buffer ([`Canvas`]), with genuine scanline
//! rectangle fill and alpha-over compositing (not a stand-in) -- the
//! numbers this module produces are pixel-accurate for what it draws.
//!
//! **What it draws:** `DisplayItem::FillRect` (solid rectangles) and, now,
//! `DisplayItem::DrawText` too -- real glyph rasterization, not a
//! placeholder: each `DrawText` item is shaped by `text::shape` (real
//! HarfBuzz-equivalent shaping) at `rect.height` as the font size (the
//! same convention `layout::flow` uses when it sets a word fragment's
//! `content_rect.height` to its own font size), then every shaped glyph's
//! real outline (`text::glyph_outline`, from `ttf-parser`) is filled with
//! the exact same nonzero-winding scanline fill `canvas2d::fill` already
//! implements for arbitrary paths -- text painting is not a separate,
//! special-cased renderer, it's this rasterizer's own real path-fill
//! algorithm applied to real glyph shapes.
//!
//! **Known gaps:** no anti-aliasing anywhere (both rectangle edges and
//! glyph edges are hard pixel boundaries); no clipping (nothing
//! establishes a clip region yet); no GPU path (the "software rasterizer
//! as the baseline" half of B11's own entry -- a GPU path was evaluated
//! and is deferred, see `ROADMAP.md`'s B11 entry for why); text painting
//! inherits `crates/text`'s own scoping limits (one embedded font, no
//! fallback, no hinting -- see that crate's module docs).

use crate::canvas2d::{self, Path2D};
use crate::color::Color;
use crate::display_list::{DisplayItem, DisplayList};
use layout::Rect;
use std::sync::OnceLock;

fn font() -> &'static text::Font {
    static FONT: OnceLock<text::Font> = OnceLock::new();
    FONT.get_or_init(text::Font::dejavu_sans)
}

/// An RGBA8 pixel buffer, row-major, 4 bytes per pixel.
#[derive(Debug, Clone)]
pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pixels: Vec<u8>,
}

impl Canvas {
    /// A new canvas, opaque white -- the same default background real
    /// browsers paint before anything else.
    pub fn new(width: usize, height: usize) -> Self {
        let mut pixels = vec![0u8; width * height * 4];
        for chunk in pixels.chunks_exact_mut(4) {
            chunk.copy_from_slice(&[255, 255, 255, 255]);
        }
        Canvas {
            width,
            height,
            pixels,
        }
    }

    pub fn get_pixel(&self, x: usize, y: usize) -> [u8; 4] {
        let i = (y * self.width + x) * 4;
        [
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ]
    }

    fn set_pixel(&mut self, x: usize, y: usize, color: [u8; 4]) {
        let i = (y * self.width + x) * 4;
        self.pixels[i..i + 4].copy_from_slice(&color);
    }

    /// A direct, unblended pixel write -- what `ImageData`'s `putImageData`
    /// means (it replaces pixels outright, it doesn't composite them; see
    /// `canvas2d`'s own module docs). Out-of-bounds coordinates are simply
    /// ignored, matching `putImageData`'s own clip-to-canvas behavior.
    pub fn put_pixel(&mut self, x: usize, y: usize, color: [u8; 4]) {
        if x >= self.width || y >= self.height {
            return;
        }
        self.set_pixel(x, y, color);
    }

    /// Real "source-over" alpha compositing (the standard Porter-Duff
    /// `over` operator), not a stand-in -- blends `color` onto whatever's
    /// already at `(x, y)`. Public so `canvas2d`'s path fill/stroke can
    /// reuse the same real blending math this module already uses for
    /// `FillRect`, instead of a second, subtly-different implementation.
    pub fn blend_pixel(&mut self, x: usize, y: usize, color: Color) {
        if x >= self.width || y >= self.height {
            return;
        }
        let dst = self.get_pixel(x, y);
        let src_a = color.a as f64 / 255.0;
        let dst_a = dst[3] as f64 / 255.0;
        let out_a = src_a + dst_a * (1.0 - src_a);
        if out_a <= 0.0 {
            self.set_pixel(x, y, [0, 0, 0, 0]);
            return;
        }
        let blend_channel = |src_c: u8, dst_c: u8| -> u8 {
            let s = src_c as f64 / 255.0;
            let d = dst_c as f64 / 255.0;
            let out = (s * src_a + d * dst_a * (1.0 - src_a)) / out_a;
            (out * 255.0).round().clamp(0.0, 255.0) as u8
        };
        self.set_pixel(
            x,
            y,
            [
                blend_channel(color.r, dst[0]),
                blend_channel(color.g, dst[1]),
                blend_channel(color.b, dst[2]),
                (out_a * 255.0).round().clamp(0.0, 255.0) as u8,
            ],
        );
    }

    /// B12: composites `layer` (another, already-rasterized canvas -- a
    /// compositor layer's own pixels) onto `self` at `(offset_x,
    /// offset_y)`, nearest-neighbor-scaled by `scale` and alpha-multiplied
    /// by `opacity`. This is the real per-pixel work behind
    /// [`crate::compositor::composite_layers`] -- separating it from
    /// [`rasterize`] is the actual architectural point of "layer
    /// promotion": once a layer's own pixels exist, moving or fading it
    /// (the common case for scrolling and `transform`/`opacity`
    /// animations) costs only this compositing pass, not a full
    /// re-rasterization of its display list.
    pub fn composite_over(
        &mut self,
        layer: &Canvas,
        offset_x: f64,
        offset_y: f64,
        scale: f64,
        opacity: f64,
    ) {
        if scale <= 0.0 || opacity <= 0.0 || layer.width == 0 || layer.height == 0 {
            return;
        }
        let opacity = opacity.min(1.0);
        let dst_w = ((layer.width as f64) * scale).round().max(0.0) as usize;
        let dst_h = ((layer.height as f64) * scale).round().max(0.0) as usize;
        let x0 = offset_x.round() as isize;
        let y0 = offset_y.round() as isize;
        for dy in 0..dst_h {
            let ty = y0 + dy as isize;
            if ty < 0 || ty as usize >= self.height {
                continue;
            }
            let sy = ((dy as f64) / scale).floor() as usize;
            if sy >= layer.height {
                continue;
            }
            for dx in 0..dst_w {
                let tx = x0 + dx as isize;
                if tx < 0 || tx as usize >= self.width {
                    continue;
                }
                let sx = ((dx as f64) / scale).floor() as usize;
                if sx >= layer.width {
                    continue;
                }
                let src = layer.get_pixel(sx, sy);
                let a = (src[3] as f64 / 255.0) * opacity;
                let color = Color {
                    r: src[0],
                    g: src[1],
                    b: src[2],
                    a: (a * 255.0).round().clamp(0.0, 255.0) as u8,
                };
                self.blend_pixel(tx as usize, ty as usize, color);
            }
        }
    }
}

pub fn rasterize(list: &DisplayList, width: usize, height: usize) -> Canvas {
    let mut canvas = Canvas::new(width, height);
    for item in &list.items {
        match item {
            DisplayItem::FillRect { rect, color } => fill_rect(&mut canvas, rect, *color),
            DisplayItem::DrawText { rect, text, color } => {
                draw_text(&mut canvas, rect, text, *color)
            }
        }
    }
    canvas
}

/// Real glyph rasterization for one `DrawText` item -- see module docs
/// for the shaping/outline/fill pipeline this drives.
fn draw_text(canvas: &mut Canvas, rect: &Rect, text: &str, color: Color) {
    let font_size_px = rect.height;
    if font_size_px <= 0.0 || text.is_empty() {
        return;
    }
    let font = font();
    let shaped = text::shape(font, text, font_size_px);
    let scale = font_size_px / font.units_per_em() as f64;
    let baseline_y = rect.y + font.ascender() as f64 * scale;
    let mut pen_x = rect.x;
    for glyph in &shaped.glyphs {
        if let Some(outline) = text::glyph_outline(font, glyph.glyph_id) {
            let mut path = Path2D::new();
            for contour in &outline.contours {
                let mut points = contour.iter();
                if let Some(&(fx, fy)) = points.next() {
                    path.move_to(
                        pen_x + glyph.x_offset + fx * scale,
                        baseline_y - glyph.y_offset - fy * scale,
                    );
                    for &(fx, fy) in points {
                        path.line_to(
                            pen_x + glyph.x_offset + fx * scale,
                            baseline_y - glyph.y_offset - fy * scale,
                        );
                    }
                    path.close_path();
                }
            }
            canvas2d::fill(canvas, &path, color);
        }
        pen_x += glyph.x_advance;
    }
}

fn fill_rect(canvas: &mut Canvas, rect: &Rect, color: Color) {
    let x0 = rect.x.max(0.0).round() as usize;
    let y0 = rect.y.max(0.0).round() as usize;
    let x1 = (rect.x + rect.width)
        .max(0.0)
        .round()
        .min(canvas.width as f64) as usize;
    let y1 = (rect.y + rect.height)
        .max(0.0)
        .round()
        .min(canvas.height as f64) as usize;
    for y in y0..y1 {
        for x in x0..x1 {
            canvas.blend_pixel(x, y, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display_list::DisplayList;

    #[test]
    fn opaque_fill_rect_paints_exact_pixels() {
        let list = DisplayList {
            items: vec![DisplayItem::FillRect {
                rect: Rect {
                    x: 2.0,
                    y: 2.0,
                    width: 3.0,
                    height: 3.0,
                },
                color: Color::rgb(255, 0, 0),
            }],
        };
        let canvas = rasterize(&list, 10, 10);
        assert_eq!(canvas.get_pixel(3, 3), [255, 0, 0, 255]);
        assert_eq!(canvas.get_pixel(1, 1), [255, 255, 255, 255]);
        assert_eq!(canvas.get_pixel(5, 5), [255, 255, 255, 255]);
    }

    #[test]
    fn translucent_fill_rect_blends_with_the_background() {
        let list = DisplayList {
            items: vec![DisplayItem::FillRect {
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                },
                color: Color {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 128,
                },
            }],
        };
        let canvas = rasterize(&list, 1, 1);
        // 50%-alpha black over opaque white -> roughly mid-gray, fully
        // opaque (white's own alpha=255 is fully covered by compositing).
        let px = canvas.get_pixel(0, 0);
        assert!((120..=136).contains(&px[0]));
        assert_eq!(px[3], 255);
    }

    #[test]
    fn fill_rect_partially_off_canvas_does_not_panic() {
        let list = DisplayList {
            items: vec![DisplayItem::FillRect {
                rect: Rect {
                    x: 8.0,
                    y: 8.0,
                    width: 10.0,
                    height: 10.0,
                },
                color: Color::rgb(0, 255, 0),
            }],
        };
        let canvas = rasterize(&list, 10, 10);
        assert_eq!(canvas.get_pixel(9, 9), [0, 255, 0, 255]);
    }

    #[test]
    fn draw_text_paints_real_glyph_pixels() {
        let list = DisplayList {
            items: vec![DisplayItem::DrawText {
                rect: Rect {
                    x: 2.0,
                    y: 2.0,
                    width: 60.0,
                    height: 20.0,
                },
                text: "M".to_string(),
                color: Color::rgb(0, 0, 0),
            }],
        };
        let canvas = rasterize(&list, 30, 30);
        // A 20px-tall "M" genuinely paints *some* non-background pixels
        // somewhere in its own rect -- not a placeholder/no-op.
        let mut painted_any = false;
        for y in 0..30 {
            for x in 0..30 {
                if canvas.get_pixel(x, y) != [255, 255, 255, 255] {
                    painted_any = true;
                }
            }
        }
        assert!(painted_any, "expected at least one non-background pixel");
    }

    #[test]
    fn draw_text_with_empty_string_does_not_panic_and_paints_nothing() {
        let list = DisplayList {
            items: vec![DisplayItem::DrawText {
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 5.0,
                    height: 5.0,
                },
                text: String::new(),
                color: Color::rgb(0, 0, 0),
            }],
        };
        let canvas = rasterize(&list, 10, 10);
        assert_eq!(canvas.get_pixel(2, 2), [255, 255, 255, 255]);
    }

    #[test]
    fn draw_text_off_canvas_does_not_panic() {
        let list = DisplayList {
            items: vec![DisplayItem::DrawText {
                rect: Rect {
                    x: 1000.0,
                    y: 1000.0,
                    width: 40.0,
                    height: 16.0,
                },
                text: "hello".to_string(),
                color: Color::rgb(0, 0, 0),
            }],
        };
        let canvas = rasterize(&list, 10, 10);
        assert_eq!(canvas.get_pixel(0, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn draw_text_with_zero_height_rect_does_not_panic() {
        let list = DisplayList {
            items: vec![DisplayItem::DrawText {
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 40.0,
                    height: 0.0,
                },
                text: "hello".to_string(),
                color: Color::rgb(0, 0, 0),
            }],
        };
        let canvas = rasterize(&list, 10, 10);
        assert_eq!(canvas.get_pixel(0, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn composite_over_translates_and_scales_a_layer() {
        let mut layer = Canvas::new(2, 2);
        layer.put_pixel(0, 0, [255, 0, 0, 255]);
        layer.put_pixel(1, 1, [0, 255, 0, 255]);
        let mut target = Canvas::new(10, 10);
        target.composite_over(&layer, 4.0, 4.0, 2.0, 1.0);
        // (0,0) of the layer, scaled 2x, offset by (4,4) -> a 2x2 block at
        // (4,4).
        assert_eq!(target.get_pixel(4, 4), [255, 0, 0, 255]);
        assert_eq!(target.get_pixel(5, 4), [255, 0, 0, 255]);
        // (1,1) of the layer -> a 2x2 block at (6,6).
        assert_eq!(target.get_pixel(6, 6), [0, 255, 0, 255]);
        // Untouched background elsewhere.
        assert_eq!(target.get_pixel(0, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn composite_over_multiplies_alpha_by_opacity() {
        let mut layer = Canvas::new(1, 1);
        layer.put_pixel(0, 0, [0, 0, 0, 255]);
        let mut target = Canvas::new(1, 1);
        target.composite_over(&layer, 0.0, 0.0, 1.0, 0.5);
        // Fully opaque black at 50% layer opacity, over opaque white ->
        // roughly mid-gray, same math as translucent_fill_rect's own test.
        let px = target.get_pixel(0, 0);
        assert!((120..=136).contains(&px[0]));
    }

    #[test]
    fn composite_over_with_zero_or_negative_scale_does_not_panic() {
        let layer = Canvas::new(2, 2);
        let mut target = Canvas::new(4, 4);
        target.composite_over(&layer, 0.0, 0.0, 0.0, 1.0);
        target.composite_over(&layer, 0.0, 0.0, -1.0, 1.0);
        target.composite_over(&layer, 0.0, 0.0, 1.0, 0.0);
        // Nothing panicked; canvas is still the untouched white background.
        assert_eq!(target.get_pixel(0, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn composite_over_off_canvas_offset_does_not_panic() {
        let mut layer = Canvas::new(2, 2);
        layer.put_pixel(0, 0, [255, 0, 0, 255]);
        let mut target = Canvas::new(4, 4);
        target.composite_over(&layer, 1000.0, -1000.0, 1.0, 1.0);
        assert_eq!(target.get_pixel(0, 0), [255, 255, 255, 255]);
    }
}
