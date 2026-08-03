//! Roadmap phase: Track B10 (text shaping/fonts) through B12 (compositing).
//! Placeholder until B9's fragment tree exists to lower into a display list.

use layout::Fragment;

/// Placeholder for the ordered list of drawing commands paint consumes:
/// fill rect, draw text run, push clip, push transform (see ROADMAP.md B9).
#[derive(Debug, Default)]
pub struct DisplayList;

pub fn build_display_list(_fragment: &Fragment) -> DisplayList {
    DisplayList
}
