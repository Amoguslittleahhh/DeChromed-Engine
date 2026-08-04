//! B13 (the `<canvas>` half): a real Canvas 2D graphics-primitive layer --
//! path construction, real scanline polygon fill (nonzero winding rule,
//! matching `CanvasRenderingContext2D.fill()`'s default), basic line
//! stroking, and `ImageData` get/put -- built on top of B11's [`Canvas`]
//! pixel buffer and its real Porter-Duff blending.
//!
//! **This is not wired to `<canvas>` the DOM element or to JavaScript.**
//! There's no Track C (script/runtime) yet, so nothing calls
//! `getContext("2d")` and dispatches into this module today -- it exists
//! as the real graphics primitives a future JS binding would call into,
//! the same relationship `paint::raster` already has to CSS painting.
//!
//! **Implemented:** `Path2D`-equivalent path building (`move_to`/
//! `line_to`/`close_path`/`rect`), `fill()` via a real scanline polygon
//! rasterizer using the nonzero winding rule (so a path with a
//! clockwise outer subpath and a counter-clockwise inner one renders a
//! real hole, e.g. a ring/donut shape -- not just "fill each subpath
//! independently"), `stroke()` via Bresenham line rasterization (real
//! per-pixel line drawing, not a stand-in), and `ImageData`-equivalent
//! `get_image_data`/`put_image_data` for direct pixel access.
//!
//! **Known gaps**, stated plainly:
//! - No bezier/quadratic curves (`bezierCurveTo`/`quadraticCurveTo`/`arc`)
//!   -- paths are polylines only.
//! - No anti-aliasing anywhere (hard pixel edges, same as B11's rect fill).
//! - `stroke()`'s line width is approximated by rasterizing several
//!   parallel Bresenham lines offset along the segment's normal, not real
//!   mitered/capped polygon stroking -- thick diagonal strokes will look
//!   rougher than a real implementation's.
//! - No `fillText`/`strokeText` (needs real font shaping, B10's own gap),
//!   no gradients or patterns, no clipping regions, and no per-context 2D
//!   transform (`ctx.translate`/`scale`/`rotate` -- distinct from B12's
//!   compositor-layer transforms).
//! - Only `source-over` compositing (the module's fill/stroke always
//!   alpha-blend) -- `globalCompositeOperation`'s other modes aren't
//!   implemented.

use crate::color::Color;
use crate::raster::Canvas;

/// A path built from straight-line subpaths -- the real subset of the
/// `Path2D`/`CanvasRenderingContext2D` path API this module implements.
#[derive(Debug, Clone, Default)]
pub struct Path2D {
    subpaths: Vec<Vec<(f64, f64)>>,
}

impl Path2D {
    pub fn new() -> Self {
        Path2D::default()
    }

    /// Starts a new subpath at `(x, y)`, matching `moveTo`'s real
    /// semantics: it does not connect to whatever subpath came before.
    pub fn move_to(&mut self, x: f64, y: f64) {
        self.subpaths.push(vec![(x, y)]);
    }

    /// Appends a point to the current subpath (starting one at `(x, y)`
    /// itself if `move_to` was never called, the same fallback real
    /// canvas contexts use).
    pub fn line_to(&mut self, x: f64, y: f64) {
        match self.subpaths.last_mut() {
            Some(sub) => sub.push((x, y)),
            None => self.subpaths.push(vec![(x, y)]),
        }
    }

    /// Closes the current subpath by connecting its last point back to
    /// its first.
    pub fn close_path(&mut self) {
        if let Some(sub) = self.subpaths.last_mut()
            && let Some(&first) = sub.first()
        {
            sub.push(first);
        }
    }

    /// A closed rectangular subpath, matching `rect()`.
    pub fn rect(&mut self, x: f64, y: f64, width: f64, height: f64) {
        self.move_to(x, y);
        self.line_to(x + width, y);
        self.line_to(x + width, y + height);
        self.line_to(x, y + height);
        self.close_path();
    }

    /// All edges across every subpath, each tagged with its winding
    /// direction (`+1` for a downward-going edge, `-1` for upward) --
    /// what the nonzero-winding scanline fill in [`fill`] needs.
    fn edges(&self) -> Vec<(f64, f64, f64, f64, i32)> {
        let mut edges = Vec::new();
        for sub in &self.subpaths {
            if sub.len() < 2 {
                continue;
            }
            for window in sub.windows(2) {
                let (x0, y0) = window[0];
                let (x1, y1) = window[1];
                if y0 == y1 {
                    continue;
                }
                let dir = if y1 > y0 { 1 } else { -1 };
                edges.push((x0, y0, x1, y1, dir));
            }
            // Implicitly close the subpath for filling purposes (canvas
            // fill() always treats open subpaths as closed).
            let (fx, fy) = sub[0];
            let (lx, ly) = sub[sub.len() - 1];
            if (fx, fy) != (lx, ly) && ly != fy {
                let dir = if fy > ly { 1 } else { -1 };
                edges.push((lx, ly, fx, fy, dir));
            }
        }
        edges
    }
}

/// Real scanline polygon fill using the nonzero winding rule: for every
/// pixel row, every path edge crossing that row contributes a signed
/// x-intersection, intersections are sorted, and a running winding-number
/// accumulator determines which spans between them are "inside" (nonzero)
/// vs. "outside" (zero) -- the same rule `CanvasRenderingContext2D.fill()`
/// defaults to, and what correctly renders a donut/ring shape (an outer
/// subpath wound one way, an inner one wound the other) as a real hole
/// rather than two independently-filled disks.
pub fn fill(canvas: &mut Canvas, path: &Path2D, color: Color) {
    let edges = path.edges();
    if edges.is_empty() {
        return;
    }
    let y_min = edges
        .iter()
        .flat_map(|e| [e.1, e.3])
        .fold(f64::INFINITY, f64::min)
        .max(0.0)
        .floor() as usize;
    let y_max = edges
        .iter()
        .flat_map(|e| [e.1, e.3])
        .fold(f64::NEG_INFINITY, f64::max)
        .min(canvas.height as f64)
        .ceil() as usize;

    for y in y_min..y_max.min(canvas.height) {
        let scan_y = y as f64 + 0.5;
        let mut crossings: Vec<(f64, i32)> = edges
            .iter()
            .filter(|&&(_, y0, _, y1, _)| {
                (y0 <= scan_y && scan_y < y1) || (y1 <= scan_y && scan_y < y0)
            })
            .map(|&(x0, y0, x1, y1, dir)| {
                let t = (scan_y - y0) / (y1 - y0);
                (x0 + t * (x1 - x0), dir)
            })
            .collect();
        crossings.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

        let mut winding = 0;
        for pair in crossings.windows(2) {
            winding += pair[0].1;
            if winding != 0 {
                let x0 = pair[0].0.max(0.0).round() as usize;
                let x1 = pair[1].0.min(canvas.width as f64).round() as usize;
                for x in x0..x1.min(canvas.width) {
                    canvas.blend_pixel(x, y, color);
                }
            }
        }
    }
}

/// Real per-pixel line rasterization (Bresenham's algorithm) for every
/// segment in every subpath, approximating `line_width` by rasterizing
/// `line_width.round()` parallel lines offset along the segment's normal
/// -- see module docs for exactly how this differs from real mitered
/// polygon stroking.
pub fn stroke(canvas: &mut Canvas, path: &Path2D, color: Color, line_width: f64) {
    let half_width = (line_width.max(1.0) / 2.0).round() as i64;
    for sub in &path.subpaths {
        for window in sub.windows(2) {
            let (x0, y0) = window[0];
            let (x1, y1) = window[1];
            let dx = x1 - x0;
            let dy = y1 - y0;
            let len = (dx * dx + dy * dy).sqrt();
            let (nx, ny) = if len > 0.0 {
                (-dy / len, dx / len)
            } else {
                (0.0, 0.0)
            };
            for offset in -half_width..=half_width {
                let ox = nx * offset as f64;
                let oy = ny * offset as f64;
                draw_line(canvas, x0 + ox, y0 + oy, x1 + ox, y1 + oy, color);
            }
        }
    }
}

fn draw_line(canvas: &mut Canvas, x0: f64, y0: f64, x1: f64, y1: f64, color: Color) {
    let (mut x0, mut y0) = (x0.round() as i64, y0.round() as i64);
    let (x1, y1) = (x1.round() as i64, y1.round() as i64);
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        if x0 >= 0 && y0 >= 0 {
            canvas.blend_pixel(x0 as usize, y0 as usize, color);
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

/// A rectangular RGBA8 pixel snapshot -- the real subset of `ImageData`
/// this module implements (no `colorSpace`, no typed-array-backed `data`
/// view, just the plain bytes).
#[derive(Debug, Clone, PartialEq)]
pub struct ImageData {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

/// Matches `getImageData(x, y, width, height)`: a direct pixel snapshot,
/// clipped to `canvas`'s own bounds (out-of-bounds rows/columns come back
/// as transparent black, matching real `getImageData`'s own clipping
/// behavior rather than panicking).
pub fn get_image_data(canvas: &Canvas, x: i64, y: i64, width: usize, height: usize) -> ImageData {
    let mut data = vec![0u8; width * height * 4];
    for row in 0..height {
        let sy = y + row as i64;
        if sy < 0 || sy as usize >= canvas.height {
            continue;
        }
        for col in 0..width {
            let sx = x + col as i64;
            if sx < 0 || sx as usize >= canvas.width {
                continue;
            }
            let px = canvas.get_pixel(sx as usize, sy as usize);
            let i = (row * width + col) * 4;
            data[i..i + 4].copy_from_slice(&px);
        }
    }
    ImageData {
        width,
        height,
        data,
    }
}

/// Matches `putImageData(imageData, x, y)`: a direct, unblended pixel
/// write (real `putImageData` replaces pixels outright, it does not
/// composite them), clipped to `canvas`'s own bounds.
pub fn put_image_data(canvas: &mut Canvas, image: &ImageData, x: i64, y: i64) {
    for row in 0..image.height {
        let ty = y + row as i64;
        if ty < 0 {
            continue;
        }
        for col in 0..image.width {
            let tx = x + col as i64;
            if tx < 0 {
                continue;
            }
            let i = (row * image.width + col) * 4;
            if i + 4 > image.data.len() {
                continue;
            }
            let px = [
                image.data[i],
                image.data[i + 1],
                image.data[i + 2],
                image.data[i + 3],
            ];
            canvas.put_pixel(tx as usize, ty as usize, px);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_rectangular_path_paints_the_interior() {
        let mut canvas = Canvas::new(10, 10);
        let mut path = Path2D::new();
        path.rect(2.0, 2.0, 4.0, 4.0);
        fill(&mut canvas, &path, Color::rgb(255, 0, 0));
        assert_eq!(canvas.get_pixel(4, 4), [255, 0, 0, 255]);
        assert_eq!(canvas.get_pixel(0, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn fill_with_nonzero_winding_renders_a_hole() {
        let mut canvas = Canvas::new(20, 20);
        let mut path = Path2D::new();
        // Outer square, clockwise.
        path.move_to(2.0, 2.0);
        path.line_to(18.0, 2.0);
        path.line_to(18.0, 18.0);
        path.line_to(2.0, 18.0);
        path.close_path();
        // Inner square, counter-clockwise (opposite winding) -> a hole
        // under the nonzero rule.
        path.move_to(8.0, 8.0);
        path.line_to(8.0, 12.0);
        path.line_to(12.0, 12.0);
        path.line_to(12.0, 8.0);
        path.close_path();
        fill(&mut canvas, &path, Color::rgb(0, 0, 255));
        // Inside the outer ring but outside the inner hole: filled.
        assert_eq!(canvas.get_pixel(4, 4), [0, 0, 255, 255]);
        // Inside the inner hole: not filled.
        assert_eq!(canvas.get_pixel(10, 10), [255, 255, 255, 255]);
    }

    #[test]
    fn fill_with_an_empty_path_does_not_panic() {
        let mut canvas = Canvas::new(5, 5);
        let path = Path2D::new();
        fill(&mut canvas, &path, Color::rgb(0, 0, 0));
        assert_eq!(canvas.get_pixel(0, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn stroke_draws_a_diagonal_line() {
        let mut canvas = Canvas::new(10, 10);
        let mut path = Path2D::new();
        path.move_to(0.0, 0.0);
        path.line_to(9.0, 9.0);
        stroke(&mut canvas, &path, Color::rgb(0, 128, 0), 1.0);
        assert_eq!(canvas.get_pixel(0, 0), [0, 128, 0, 255]);
        assert_eq!(canvas.get_pixel(9, 9), [0, 128, 0, 255]);
    }

    #[test]
    fn stroke_off_canvas_does_not_panic() {
        let mut canvas = Canvas::new(4, 4);
        let mut path = Path2D::new();
        path.move_to(-10.0, -10.0);
        path.line_to(100.0, 100.0);
        stroke(&mut canvas, &path, Color::rgb(0, 0, 0), 3.0);
    }

    #[test]
    fn get_and_put_image_data_round_trips() {
        let mut canvas = Canvas::new(4, 4);
        canvas.put_pixel(1, 1, [10, 20, 30, 255]);
        let snapshot = get_image_data(&canvas, 0, 0, 4, 4);
        let mut target = Canvas::new(4, 4);
        put_image_data(&mut target, &snapshot, 0, 0);
        assert_eq!(target.get_pixel(1, 1), [10, 20, 30, 255]);
    }

    #[test]
    fn get_image_data_out_of_bounds_region_is_transparent_black() {
        let canvas = Canvas::new(2, 2);
        let snapshot = get_image_data(&canvas, -5, -5, 3, 3);
        assert_eq!(snapshot.data[0..4], [0, 0, 0, 0]);
    }

    #[test]
    fn put_image_data_with_negative_offset_does_not_panic() {
        let mut canvas = Canvas::new(4, 4);
        let image = ImageData {
            width: 2,
            height: 2,
            data: vec![1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255],
        };
        put_image_data(&mut canvas, &image, -1, -1);
    }
}
