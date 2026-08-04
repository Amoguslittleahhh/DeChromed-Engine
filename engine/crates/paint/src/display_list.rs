//! B9's "display list" half: lowering a `layout::Fragment` tree into an
//! ordered list of real drawing commands, the actual architectural
//! payoff of keeping a fragment tree around at all (per `ROADMAP.md`'s
//! B9 entry) -- paint (B11+) consumes this instead of re-deriving
//! geometry/style from the fragment tree itself.
//!
//! `color` is real CSS Color inheritance, threaded down through the walk
//! (the same pattern `layout::flow` already uses for `font-size`): a
//! fragment's own `color` (if set and not `currentcolor`) overrides the
//! inherited value for itself and everything beneath it; a leaf text
//! fragment has no computed style of its own (only real elements do), so
//! it always uses whatever color it inherited from its nearest styled
//! ancestor.
//!
//! Known gaps: only `background-color` (as a solid fill) and `color`
//! (for text) are read -- no `border-color`/border painting, no
//! `background-image`, no `box-shadow`, no `opacity` compositing, and no
//! clipping (`overflow: hidden` isn't implemented anywhere yet). Fragment
//! coordinates are already absolute (see `layout::Fragment`'s own doc
//! comment), so this is a direct, un-transformed lowering with no
//! `PushClip`/`PushTransform` items -- those become real once clipping/
//! transforms exist to justify them.

use crate::color::{Color, parse_color};
use layout::{Fragment, Rect, StyleMap};

#[derive(Debug, Clone, PartialEq)]
pub enum DisplayItem {
    FillRect {
        rect: Rect,
        color: Color,
    },
    DrawText {
        rect: Rect,
        text: String,
        color: Color,
        /// The font size to shape/rasterize `text` at, in CSS pixels --
        /// an explicit field rather than an implicit "the rasterizer
        /// infers it from `rect.height`" convention, so a future
        /// `DrawText` producer (e.g. a `<canvas>` text API) can't
        /// silently mis-size text by setting `rect.height` to something
        /// other than the font size (a line-box height, say) with no
        /// compiler signal that the old convention was broken.
        font_size_px: f64,
    },
}

#[derive(Debug, Default, Clone)]
pub struct DisplayList {
    pub items: Vec<DisplayItem>,
}

pub fn build_display_list(root: &Fragment, styles: &StyleMap) -> DisplayList {
    let mut items = Vec::new();
    build(root, styles, Color::BLACK, &mut items);
    DisplayList { items }
}

fn build(
    fragment: &Fragment,
    styles: &StyleMap,
    inherited_color: Color,
    out: &mut Vec<DisplayItem>,
) {
    let style = fragment.node.and_then(|n| styles.get(&n));
    let color = style
        .and_then(|s| s.get("color"))
        .and_then(|c| parse_color(c))
        .unwrap_or(inherited_color);

    if let Some(text) = &fragment.text {
        if !text.is_empty() {
            out.push(DisplayItem::DrawText {
                rect: fragment.content_rect,
                text: text.clone(),
                color,
                // `layout::flow` sets a word fragment's `content_rect.
                // height` to exactly its own resolved font size (see
                // `Fragment`'s construction in `layout_inline_children`)
                // -- reading it here, once, at the one place that
                // convention is actually established, keeps the
                // assumption in one visible spot instead of the
                // rasterizer re-deriving it implicitly a crate away.
                font_size_px: fragment.content_rect.height,
            });
        }
        return;
    }

    if let Some(bg) = style
        .and_then(|s| s.get("background-color"))
        .and_then(|c| parse_color(c))
        && bg.a > 0
    {
        out.push(DisplayItem::FillRect {
            rect: fragment.border_box(),
            color: bg,
        });
    }

    for child in &fragment.children {
        build(child, styles, color, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dom::{Document, ElementData, NodeData, NodeId};

    fn el(doc: &mut Document, parent: NodeId, tag: &str, attrs: &[(&str, &str)]) -> NodeId {
        doc.append(
            parent,
            NodeData::Element(ElementData::html(
                tag,
                attrs
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            )),
        )
    }

    fn set_style(styles: &mut StyleMap, node: NodeId, props: &[(&str, &str)]) {
        styles.insert(
            node,
            props
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        );
    }

    #[test]
    fn background_color_becomes_a_fill_rect() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[]);
        let mut styles = StyleMap::new();
        set_style(
            &mut styles,
            div,
            &[
                ("display", "block"),
                ("width", "10px"),
                ("height", "10px"),
                ("background-color", "red"),
            ],
        );
        let tree = layout::build_box_tree(&doc, &styles).unwrap();
        let fragment = layout::layout(&tree, &styles, 100.0);
        let list = build_display_list(&fragment, &styles);
        assert_eq!(list.items.len(), 1);
        match &list.items[0] {
            DisplayItem::FillRect { color, .. } => assert_eq!(*color, Color::rgb(255, 0, 0)),
            other => panic!("expected FillRect, got {other:?}"),
        }
    }

    #[test]
    fn transparent_background_emits_nothing() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[]);
        let mut styles = StyleMap::new();
        set_style(
            &mut styles,
            div,
            &[("display", "block"), ("height", "10px")],
        );
        let tree = layout::build_box_tree(&doc, &styles).unwrap();
        let fragment = layout::layout(&tree, &styles, 100.0);
        let list = build_display_list(&fragment, &styles);
        assert!(list.items.is_empty());
    }

    #[test]
    fn text_inherits_the_nearest_ancestors_color() {
        let mut doc = Document::new();
        let root = doc.root();
        let p = el(&mut doc, root, "p", &[]);
        doc.append(p, NodeData::Text("hi".into()));
        let mut styles = StyleMap::new();
        set_style(&mut styles, p, &[("display", "block"), ("color", "blue")]);
        let tree = layout::build_box_tree(&doc, &styles).unwrap();
        let fragment = layout::layout(&tree, &styles, 100.0);
        let list = build_display_list(&fragment, &styles);
        let text_item = list
            .items
            .iter()
            .find(|i| matches!(i, DisplayItem::DrawText { .. }))
            .expect("expected a DrawText item");
        match text_item {
            DisplayItem::DrawText { color, text, .. } => {
                assert_eq!(*color, Color::rgb(0, 0, 255));
                assert_eq!(text, "hi");
            }
            _ => unreachable!(),
        }
    }
}
