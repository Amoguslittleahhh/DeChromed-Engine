//! B11: a software rasterizer -- the real baseline both Blink (Skia) and
//! Gecko (WebRender) also bootstrap new platforms from before adding a
//! GPU path, per `ROADMAP.md`'s own B11 entry. Turns a [`DisplayList`]
//! into a real RGBA8 pixel buffer ([`Canvas`]), with genuine scanline
//! rectangle fill and alpha-over compositing (not a stand-in) -- the
//! numbers this module produces are pixel-accurate for what it draws.
//!
//! **What it draws:** `DisplayItem::FillRect` only. `DisplayItem::
//! DrawText` items are correctly positioned and colored by B9's display-
//! list lowering, but this rasterizer doesn't paint them -- there's no
//! font outline data or embedded bitmap glyph atlas yet (B10 only
//! improved *measurement*, via `layout::values`' character-width table,
//! not shaping/rendering), so drawing placeholder glyph shapes here would
//! overclaim what's actually implemented. Text regions are simply left
//! unpainted, a documented gap rather than a faked rendering.
//!
//! **Known gaps:** no anti-aliasing (rectangle edges are hard pixel
//! boundaries, since fill rects are always axis-aligned integers-after-
//! rounding); no clipping (nothing establishes a clip region yet); no
//! GPU path (the "software rasterizer as the baseline" half of B11's own
//! entry -- a GPU path is explicitly future work, not this landing).

use crate::color::Color;
use crate::display_list::{DisplayItem, DisplayList};
use layout::Rect;

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

    /// Real "source-over" alpha compositing (the standard Porter-Duff
    /// `over` operator), not a stand-in -- blends `color` onto whatever's
    /// already at `(x, y)`.
    fn blend_pixel(&mut self, x: usize, y: usize, color: Color) {
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
}

pub fn rasterize(list: &DisplayList, width: usize, height: usize) -> Canvas {
    let mut canvas = Canvas::new(width, height);
    for item in &list.items {
        if let DisplayItem::FillRect { rect, color } = item {
            fill_rect(&mut canvas, rect, *color);
        }
        // DrawText: documented gap, see module docs.
    }
    canvas
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
    fn draw_text_items_are_not_rasterized() {
        let list = DisplayList {
            items: vec![DisplayItem::DrawText {
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 5.0,
                    height: 5.0,
                },
                text: "hi".to_string(),
                color: Color::rgb(0, 0, 0),
            }],
        };
        let canvas = rasterize(&list, 10, 10);
        // Still the untouched white background -- documented gap, see
        // module docs.
        assert_eq!(canvas.get_pixel(2, 2), [255, 255, 255, 255]);
    }
}
