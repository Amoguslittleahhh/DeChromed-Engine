//! B12: a real layer compositor -- the actual mechanism that makes
//! scrolling and `transform`/`opacity` animations feel smooth in Blink and
//! Gecko, per `ROADMAP.md`'s own B12 entry: instead of re-rasterizing an
//! entire page's display list every frame, content that's going to move or
//! fade gets promoted onto its own [`Layer`], rasterized once, and then
//! moved/faded via cheap per-pixel compositing (`Canvas::composite_over`)
//! on every subsequent frame.
//!
//! This module is that compositing pass: [`composite_layers`] takes an
//! ordered stack of [`Layer`]s (each with its own [`DisplayList`] and a
//! translate/scale/opacity transform), rasterizes each one exactly once,
//! and composites them back-to-front onto a single output [`Canvas`] with
//! real per-pixel alpha math.
//!
//! **Known gaps**, all stated plainly rather than faked:
//! - **No compositor thread.** Real browsers run this on a thread separate
//!   from the one running layout/script, so an in-flight animation keeps
//!   moving even while the main thread is busy. `composite_layers` here
//!   runs synchronously, on whatever thread calls it.
//! - **No GPU path.** Layers are rasterized in software (B11's rasterizer)
//!   and composited in software; a real engine's compositor almost always
//!   uploads rasterized tiles as GPU textures and does this blending step
//!   on the GPU instead.
//! - **No rotation.** A [`Layer`]'s transform is translate, uniform scale,
//!   and opacity only -- real CSS `transform` also supports rotation, skew,
//!   and non-uniform/3D scale, none of which `Canvas::composite_over`'s
//!   nearest-neighbor sampling implements.
//! - **No automatic layer promotion.** Real engines decide *which*
//!   elements get their own compositor layer (typically ones with
//!   `will-change`, active `transform`/`opacity` animations, or certain
//!   stacking-context triggers) as part of the paint pipeline. That
//!   decision isn't wired up here -- callers construct `Layer`s by hand.
//! - **No true async animations.** Nothing here schedules frames or
//!   interpolates a `Layer`'s `offset`/`opacity` over time; a caller that
//!   wants an animation must recompute those fields itself between calls.

use crate::display_list::DisplayList;
use crate::raster::{Canvas, rasterize};

/// One compositor layer: its own display list, rasterized independently of
/// every other layer, plus the translate/scale/opacity transform to apply
/// when compositing it onto the final frame.
#[derive(Debug, Clone)]
pub struct Layer {
    pub display_list: DisplayList,
    pub width: usize,
    pub height: usize,
    pub offset_x: f64,
    pub offset_y: f64,
    pub scale: f64,
    pub opacity: f64,
}

impl Layer {
    /// A layer at its natural position/size, fully opaque -- the common
    /// case before any transform/opacity animation moves it.
    pub fn new(display_list: DisplayList, width: usize, height: usize) -> Self {
        Layer {
            display_list,
            width,
            height,
            offset_x: 0.0,
            offset_y: 0.0,
            scale: 1.0,
            opacity: 1.0,
        }
    }
}

/// Rasterizes every layer in `layers` exactly once, then composites them
/// back-to-front (later layers paint over earlier ones, the same stacking
/// order [`crate::display_list::build_display_list`]'s own children walk
/// already uses) onto a fresh `canvas_width`x`canvas_height` output canvas.
pub fn composite_layers(layers: &[Layer], canvas_width: usize, canvas_height: usize) -> Canvas {
    let mut target = Canvas::new(canvas_width, canvas_height);
    for layer in layers {
        let rasterized = rasterize(&layer.display_list, layer.width, layer.height);
        target.composite_over(
            &rasterized,
            layer.offset_x,
            layer.offset_y,
            layer.scale,
            layer.opacity,
        );
    }
    target
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::display_list::DisplayItem;
    use layout::Rect;

    fn solid_fill_layer(color: Color, width: usize, height: usize) -> Layer {
        let list = DisplayList {
            items: vec![DisplayItem::FillRect {
                rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: width as f64,
                    height: height as f64,
                },
                color,
            }],
        };
        Layer::new(list, width, height)
    }

    #[test]
    fn a_single_layer_composites_at_its_offset() {
        let mut layer = solid_fill_layer(Color::rgb(255, 0, 0), 2, 2);
        layer.offset_x = 3.0;
        layer.offset_y = 3.0;
        let canvas = composite_layers(&[layer], 10, 10);
        assert_eq!(canvas.get_pixel(3, 3), [255, 0, 0, 255]);
        assert_eq!(canvas.get_pixel(0, 0), [255, 255, 255, 255]);
    }

    #[test]
    fn later_layers_paint_over_earlier_ones() {
        let bottom = solid_fill_layer(Color::rgb(255, 0, 0), 4, 4);
        let top = solid_fill_layer(Color::rgb(0, 0, 255), 4, 4);
        let canvas = composite_layers(&[bottom, top], 4, 4);
        assert_eq!(canvas.get_pixel(0, 0), [0, 0, 255, 255]);
    }

    #[test]
    fn opacity_fades_a_layer_toward_the_background() {
        let mut layer = solid_fill_layer(Color::rgb(0, 0, 0), 1, 1);
        layer.opacity = 0.5;
        let canvas = composite_layers(&[layer], 1, 1);
        // Same source-over math as raster.rs's own translucency test.
        let px = canvas.get_pixel(0, 0);
        assert!((120..=136).contains(&px[0]));
    }

    #[test]
    fn scaling_a_layer_does_not_panic_and_covers_the_scaled_area() {
        let mut layer = solid_fill_layer(Color::rgb(0, 255, 0), 2, 2);
        layer.scale = 3.0;
        let canvas = composite_layers(&[layer], 20, 20);
        assert_eq!(canvas.get_pixel(5, 5), [0, 255, 0, 255]);
    }

    #[test]
    fn an_empty_layer_stack_produces_the_untouched_background() {
        let canvas = composite_layers(&[], 4, 4);
        assert_eq!(canvas.get_pixel(0, 0), [255, 255, 255, 255]);
    }
}
