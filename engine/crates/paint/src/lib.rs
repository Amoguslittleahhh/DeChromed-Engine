//! B9's display-list half (`display_list.rs`) and a real `<color>`
//! parser (`color.rs`) are started here. B10 (text shaping/fonts) is
//! started too -- see `layout::values`' character-width table. B11
//! (rasterization, `raster.rs`) covers solid-rectangle painting into a
//! real pixel buffer; text painting is still a placeholder there (no
//! glyph outlines exist yet -- see its own module docs). B12
//! (compositing, `compositor.rs`) is started: a real layer compositor
//! with translate/scale/opacity, no thread/GPU path yet. B13's Canvas 2D
//! graphics primitives (`canvas2d.rs`) are started too -- real path
//! fill/stroke and `ImageData` get/put, not yet wired to a `<canvas>`
//! DOM element or JS (no Track C yet); B13's WebGL/WebGPU half hasn't
//! started at all.

pub mod canvas2d;
pub mod color;
pub mod compositor;
pub mod display_list;
pub mod raster;

pub use canvas2d::{ImageData, Path2D, fill, get_image_data, put_image_data, stroke};
pub use color::Color;
pub use compositor::{Layer, composite_layers};
pub use display_list::{DisplayItem, DisplayList, build_display_list};
pub use raster::{Canvas, rasterize};
