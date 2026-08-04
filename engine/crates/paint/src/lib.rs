//! B9's display-list half (`display_list.rs`) and a real `<color>`
//! parser (`color.rs`) are started here. B10 (text shaping/fonts) is
//! started too -- see `layout::values`' character-width table. B11
//! (rasterization, `raster.rs`) covers solid-rectangle painting into a
//! real pixel buffer; text painting is still a placeholder there (no
//! glyph outlines exist yet -- see its own module docs). B12
//! (compositing) hasn't started.

pub mod color;
pub mod display_list;
pub mod raster;

pub use color::Color;
pub use display_list::{DisplayItem, DisplayList, build_display_list};
pub use raster::{Canvas, rasterize};
