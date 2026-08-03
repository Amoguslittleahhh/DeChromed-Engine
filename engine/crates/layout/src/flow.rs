//! B2: block & inline formatting contexts -- turning a box tree (B1) into
//! a fragment tree of positioned, sized boxes.
//!
//! Reference: <https://www.w3.org/TR/CSS22/visuren.html#normal-flow>,
//! <https://www.w3.org/TR/CSS22/box.html#box-dimensions>,
//! <https://www.w3.org/TR/CSS22/visudet.html> (width/height resolution),
//! <https://www.w3.org/TR/CSS22/box.html#collapsing-margins> (margin
//! collapsing).
//!
//! ## What's real here
//! - Full box-model geometry: margin/border/padding/content-box widths
//!   resolved from computed-style strings (`crate::values`), including
//!   `auto` width filling the remaining containing-block space and
//!   percentage margins/padding resolved against the containing block.
//! - Block formatting context: children stack vertically; adjacent
//!   sibling margins collapse per the real CSS2.1 algorithm (positive
//!   margins take the max, a negative margin's magnitude is *subtracted*
//!   from that max, not just "take the bigger number").
//! - Inline formatting context: text is flattened across nested inline
//!   boxes (`<b>`/`<span>`/...) into words, and wraps into line boxes at
//!   the containing block's content width.
//! - `float: left|right` takes a box out of normal-flow vertical
//!   stacking and positions it at the content-box's left/right edge, and
//!   `clear: left|right|both` pushes a later in-flow sibling below the
//!   relevant floats' bottom edge -- both tracked per block formatting
//!   context, matching the real scoping rule (floats/clears interact
//!   with siblings in the *same* BFC, not ones nested inside a child's
//!   own BFC).
//!
//! ## Known gaps, documented rather than silently assumed away
//! - **No used-value text metrics.** There's no real font
//!   shaping/glyph-metrics engine yet (that's B10); word/line widths use a
//!   flat `font_size_px * 0.5` per-character heuristic instead of real
//!   glyph advances. Every number this module produces for text layout is
//!   therefore a rough visual approximation, not a pixel-accurate result
//!   -- expected to be revisited once B10 exists.
//! - **No shrink-to-fit / intrinsic sizing.** An auto-width `inline-block`
//!   or float falls back to the same "fill the remaining containing-block
//!   width" rule an ordinary auto-width block box uses, which is not
//!   spec-correct for either (both should shrink-fit to their content) --
//!   a documented placeholder, not an invented arbitrary constant.
//! - **Floats don't narrow sibling content.** A float is correctly taken
//!   out of vertical block stacking and positioned to a side, and`clear`
//!   correctly pushes below it, but normal-flow siblings/line boxes don't
//!   yet get their available width reduced to visually wrap around a
//!   float the way a real UA renders it -- that needs per-line
//!   float-aware width tracking, a further refinement on top of this
//!   phase's line-breaking.
//! - **No parent-child margin collapsing** (a block's top/bottom margin
//!   collapsing through into its parent's own margin when there's no
//!   border/padding between them) and **no collapsing-through-empty-box**
//!   case -- only adjacent-sibling collapsing is implemented.
//! - **No `auto` margin centering** (`margin: 0 auto` block centering);
//!   `auto` margins resolve to `0` here.
//! - Vertical text layout ignores baseline alignment across mixed font
//!   sizes on one line -- a line's height is simply the max resolved
//!   line-height among its items, and everything is top-aligned within it.

use crate::box_tree::{BoxKind, BoxLevel, ComputedStyle, LayoutBox, StyleMap};
use crate::values::{self, LengthPercentageAuto};
use dom::NodeId;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EdgeSizes {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

#[derive(Debug, Clone)]
pub struct Fragment {
    pub node: Option<NodeId>,
    /// Content-box position (in an abstract layout space whose origin is
    /// the root box's border-box top-left corner) and size.
    pub content_rect: Rect,
    pub margin: EdgeSizes,
    pub border: EdgeSizes,
    pub padding: EdgeSizes,
    pub children: Vec<Fragment>,
    /// `Some` for a text/marker fragment (a leaf with no further
    /// children); `None` for a container fragment.
    pub text: Option<String>,
}

impl Fragment {
    pub fn border_box(&self) -> Rect {
        Rect {
            x: self.content_rect.x - self.padding.left - self.border.left,
            y: self.content_rect.y - self.padding.top - self.border.top,
            width: self.content_rect.width
                + self.padding.left
                + self.padding.right
                + self.border.left
                + self.border.right,
            height: self.content_rect.height
                + self.padding.top
                + self.padding.bottom
                + self.border.top
                + self.border.bottom,
        }
    }

    pub fn margin_box(&self) -> Rect {
        let b = self.border_box();
        Rect {
            x: b.x - self.margin.left,
            y: b.y - self.margin.top,
            width: b.width + self.margin.left + self.margin.right,
            height: b.height + self.margin.top + self.margin.bottom,
        }
    }
}

fn get<'a>(style: Option<&'a ComputedStyle>, name: &str, default: &'a str) -> &'a str {
    style
        .and_then(|s| s.get(name))
        .map(String::as_str)
        .unwrap_or(default)
}

/// Lays out `root` (B1's box tree) as if it were the sole child of an
/// (unstyled) containing block `containing_width` pixels wide. This is the
/// crate's top-level entry point.
pub fn layout(root: &LayoutBox, styles: &StyleMap, containing_width: f64) -> Fragment {
    let root_font_size = resolve_font_size(
        root.node,
        styles,
        values::default_font_size_px(),
        values::default_font_size_px(),
    );
    layout_box(
        root,
        styles,
        containing_width,
        root_font_size,
        root_font_size,
    )
}

fn resolve_font_size(
    node: Option<NodeId>,
    styles: &StyleMap,
    parent_font_size_px: f64,
    root_font_size_px: f64,
) -> f64 {
    match node
        .and_then(|n| styles.get(&n))
        .and_then(|s| s.get("font-size"))
    {
        Some(v) => values::resolve_font_size_px(v, parent_font_size_px, root_font_size_px),
        None => parent_font_size_px,
    }
}

struct BoxModel {
    margin: EdgeSizes,
    border: EdgeSizes,
    padding: EdgeSizes,
    content_width: f64,
}

fn resolve_box_model(
    style: Option<&ComputedStyle>,
    containing_width: f64,
    font_size_px: f64,
    root_font_size_px: f64,
) -> BoxModel {
    let resolve = |name: &str| -> f64 {
        match values::parse_length_percentage_auto(
            get(style, name, "0"),
            font_size_px,
            root_font_size_px,
        ) {
            LengthPercentageAuto::Length(px) => px,
            LengthPercentageAuto::Percentage(pct) => containing_width * pct / 100.0,
            LengthPercentageAuto::Auto => 0.0,
        }
    };
    let margin = EdgeSizes {
        top: resolve("margin-top"),
        right: resolve("margin-right"),
        bottom: resolve("margin-bottom"),
        left: resolve("margin-left"),
    };
    let border_style_none = |side: &str| get(style, side, "none") == "none";
    let border_width = |side: &str| {
        if border_style_none(side) {
            0.0
        } else {
            values::parse_border_width_px(
                get(style, "border-width", "medium"),
                font_size_px,
                root_font_size_px,
            )
        }
    };
    let border = EdgeSizes {
        top: border_width("border-style"),
        right: border_width("border-style"),
        bottom: border_width("border-style"),
        left: border_width("border-style"),
    };
    let padding = EdgeSizes {
        top: resolve("padding-top"),
        right: resolve("padding-right"),
        bottom: resolve("padding-bottom"),
        left: resolve("padding-left"),
    };
    let non_content_width =
        margin.left + margin.right + border.left + border.right + padding.left + padding.right;
    let content_width = match values::parse_length_percentage_auto(
        get(style, "width", "auto"),
        font_size_px,
        root_font_size_px,
    ) {
        LengthPercentageAuto::Length(px) => px,
        LengthPercentageAuto::Percentage(pct) => containing_width * pct / 100.0,
        LengthPercentageAuto::Auto => (containing_width - non_content_width).max(0.0),
    };
    BoxModel {
        margin,
        border,
        padding,
        content_width,
    }
}

/// CSS2.1 8.3.1's real collapsing rule: take the max of the positive
/// margins, then subtract the max *magnitude* among the negative margins.
fn collapse_margins(a: f64, b: f64) -> f64 {
    let positive_max = a.max(0.0).max(b.max(0.0));
    let negative_max = (-a).max(0.0).max((-b).max(0.0));
    positive_max - negative_max
}

fn layout_box(
    b: &LayoutBox,
    styles: &StyleMap,
    containing_width: f64,
    parent_font_size_px: f64,
    root_font_size_px: f64,
) -> Fragment {
    let style = b.node.and_then(|n| styles.get(&n));
    let font_size_px = resolve_font_size(b.node, styles, parent_font_size_px, root_font_size_px);
    let model = resolve_box_model(style, containing_width, font_size_px, root_font_size_px);

    match &b.kind {
        BoxKind::Text(text) => Fragment {
            node: b.node,
            content_rect: Rect {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 0.0,
            },
            margin: EdgeSizes::default(),
            border: EdgeSizes::default(),
            padding: EdgeSizes::default(),
            children: Vec::new(),
            text: Some(text.clone()),
        },
        BoxKind::Container(children) => {
            let is_bfc = children.iter().any(|c| c.level == BoxLevel::Block);
            let (child_fragments, content_height) = if is_bfc {
                layout_block_children(
                    children,
                    styles,
                    model.content_width,
                    font_size_px,
                    root_font_size_px,
                )
            } else {
                layout_inline_children(
                    children,
                    styles,
                    model.content_width,
                    font_size_px,
                    root_font_size_px,
                )
            };
            let explicit_height = values::parse_length_percentage_auto(
                get(style, "height", "auto"),
                font_size_px,
                root_font_size_px,
            );
            let height = match explicit_height {
                LengthPercentageAuto::Length(px) => px,
                LengthPercentageAuto::Percentage(_) | LengthPercentageAuto::Auto => content_height,
            };
            Fragment {
                node: b.node,
                content_rect: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: model.content_width,
                    height,
                },
                margin: model.margin,
                border: model.border,
                padding: model.padding,
                children: child_fragments,
                text: None,
            }
        }
    }
}

/// Lays out `children` (all block-level, guaranteed by B1's anonymous-box
/// wrapping) top-to-bottom, applying sibling margin collapsing and
/// float/clear -- both scoped to this one block formatting context, per
/// the real spec rule that floats/clears interact with siblings in the
/// *same* BFC only.
fn layout_block_children(
    children: &[LayoutBox],
    styles: &StyleMap,
    containing_width: f64,
    font_size_px: f64,
    root_font_size_px: f64,
) -> (Vec<Fragment>, f64) {
    struct FloatBox {
        right_side: bool,
        bottom: f64,
    }
    let mut floats: Vec<FloatBox> = Vec::new();
    let mut out = Vec::new();
    let mut cursor_y = 0.0;
    let mut prev_margin_bottom: Option<f64> = None;

    for child in children {
        let style = child.node.and_then(|n| styles.get(&n));
        let float = get(style, "float", "none");
        let clear = get(style, "clear", "none");

        if clear == "left" || clear == "right" || clear == "both" {
            let clears_left = clear == "left" || clear == "both";
            let clears_right = clear == "right" || clear == "both";
            let clear_y = floats
                .iter()
                .filter(|f| (f.right_side && clears_right) || (!f.right_side && clears_left))
                .map(|f| f.bottom)
                .fold(cursor_y, f64::max);
            if clear_y > cursor_y {
                cursor_y = clear_y;
                prev_margin_bottom = None;
            }
        }

        let mut fragment = layout_box(
            child,
            styles,
            containing_width,
            font_size_px,
            root_font_size_px,
        );

        if float == "left" || float == "right" {
            // Floats never participate in margin collapsing and don't
            // consume a slot in the parent's normal vertical stacking.
            let x = if float == "right" {
                containing_width - fragment.margin_box().width
            } else {
                0.0
            };
            let margin_left = fragment.margin.left;
            let margin_top = fragment.margin.top;
            reposition(&mut fragment, x + margin_left, cursor_y + margin_top);
            let bottom = cursor_y + fragment.margin_box().height;
            floats.push(FloatBox {
                right_side: float == "right",
                bottom,
            });
            out.push(fragment);
            continue;
        }

        let gap = match prev_margin_bottom {
            Some(prev) => collapse_margins(prev, fragment.margin.top),
            None => fragment.margin.top,
        };
        cursor_y += gap;
        let margin_left = fragment.margin.left;
        reposition(&mut fragment, margin_left, cursor_y);
        cursor_y += fragment.border_box().height;
        prev_margin_bottom = Some(fragment.margin.bottom);
        out.push(fragment);
    }
    if let Some(bottom) = prev_margin_bottom {
        cursor_y += bottom;
    }
    (out, cursor_y)
}

fn reposition(fragment: &mut Fragment, x: f64, y: f64) {
    let dx = x - fragment.border_box().x;
    let dy = y - fragment.border_box().y;
    shift(fragment, dx, dy);
}

fn shift(fragment: &mut Fragment, dx: f64, dy: f64) {
    fragment.content_rect.x += dx;
    fragment.content_rect.y += dy;
    for child in &mut fragment.children {
        shift(child, dx, dy);
    }
}

enum InlineItem<'a> {
    Word {
        node: Option<NodeId>,
        text: String,
        font_size_px: f64,
    },
    Atomic(&'a LayoutBox, f64),
}

fn flatten_inline<'a>(
    b: &'a LayoutBox,
    styles: &StyleMap,
    font_size_px: f64,
    root_font_size_px: f64,
    out: &mut Vec<InlineItem<'a>>,
) {
    match &b.kind {
        BoxKind::Text(text) => {
            for word in text.split_whitespace() {
                out.push(InlineItem::Word {
                    node: b.node,
                    text: word.to_string(),
                    font_size_px,
                });
            }
        }
        BoxKind::Container(children) => {
            if b.level == BoxLevel::InlineBlock {
                out.push(InlineItem::Atomic(b, font_size_px));
            } else {
                let own_font_size =
                    resolve_font_size(b.node, styles, font_size_px, root_font_size_px);
                for child in children {
                    flatten_inline(child, styles, own_font_size, root_font_size_px, out);
                }
            }
        }
    }
}

/// Rough visual per-character width heuristic -- see module docs' "no
/// real font shaping" known gap.
fn word_width(text: &str, font_size_px: f64) -> f64 {
    text.chars().count() as f64 * font_size_px * 0.5
}

fn space_width(font_size_px: f64) -> f64 {
    font_size_px * 0.28
}

fn line_height_px(style: Option<&ComputedStyle>, font_size_px: f64, root_font_size_px: f64) -> f64 {
    let value = get(style, "line-height", "normal");
    if value == "normal" {
        return font_size_px * 1.2;
    }
    if let Ok(multiplier) = value.parse::<f64>() {
        return font_size_px * multiplier;
    }
    values::parse_length_px(value, font_size_px, root_font_size_px).unwrap_or(font_size_px * 1.2)
}

fn layout_inline_children(
    children: &[LayoutBox],
    styles: &StyleMap,
    containing_width: f64,
    font_size_px: f64,
    root_font_size_px: f64,
) -> (Vec<Fragment>, f64) {
    let mut items = Vec::new();
    for child in children {
        flatten_inline(child, styles, font_size_px, root_font_size_px, &mut items);
    }

    let mut lines: Vec<Vec<Fragment>> = vec![Vec::new()];
    let mut line_widths: Vec<f64> = vec![0.0];
    let mut line_max_font: Vec<f64> = vec![font_size_px];

    for item in items {
        let (mut fragment, width, item_font_size) = match item {
            InlineItem::Word {
                node,
                text,
                font_size_px,
            } => {
                let w = word_width(&text, font_size_px);
                (
                    Fragment {
                        node,
                        content_rect: Rect {
                            x: 0.0,
                            y: 0.0,
                            width: w,
                            height: font_size_px,
                        },
                        margin: EdgeSizes::default(),
                        border: EdgeSizes::default(),
                        padding: EdgeSizes::default(),
                        children: Vec::new(),
                        text: Some(text),
                    },
                    w,
                    font_size_px,
                )
            }
            InlineItem::Atomic(b, inherited_font_size) => {
                let f = layout_box(
                    b,
                    styles,
                    containing_width,
                    inherited_font_size,
                    root_font_size_px,
                );
                let w = f.margin_box().width;
                (f, w, inherited_font_size)
            }
        };

        let current = lines.len() - 1;
        let needs_space = line_widths[current] > 0.0;
        let extra = if needs_space {
            space_width(item_font_size)
        } else {
            0.0
        };
        if line_widths[current] > 0.0 && line_widths[current] + extra + width > containing_width {
            lines.push(Vec::new());
            line_widths.push(0.0);
            line_max_font.push(font_size_px);
            let current = lines.len() - 1;
            reposition(&mut fragment, 0.0, 0.0);
            line_widths[current] = width;
            line_max_font[current] = line_max_font[current].max(item_font_size);
            lines[current].push(fragment);
        } else {
            let x = line_widths[current] + extra;
            reposition(&mut fragment, x, 0.0);
            line_widths[current] = x + width;
            line_max_font[current] = line_max_font[current].max(item_font_size);
            lines[current].push(fragment);
        }
    }

    let mut out = Vec::new();
    let mut cursor_y = 0.0;
    for (i, mut line) in lines.into_iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let height = line_height_px(None, line_max_font[i], root_font_size_px);
        for fragment in &mut line {
            shift(fragment, 0.0, cursor_y);
        }
        cursor_y += height;
        out.push(Fragment {
            node: None,
            content_rect: Rect {
                x: 0.0,
                y: cursor_y - height,
                width: containing_width,
                height,
            },
            margin: EdgeSizes::default(),
            border: EdgeSizes::default(),
            padding: EdgeSizes::default(),
            children: line,
            text: None,
        });
    }
    (out, cursor_y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::box_tree::build_box_tree;
    use dom::{Document, ElementData, NodeData};

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

    #[test]
    fn width_and_padding_and_border_resolve_correctly() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[]);
        let mut styles = StyleMap::new();
        styles.insert(
            div,
            [
                ("display", "block"),
                ("width", "100px"),
                ("padding-left", "10px"),
                ("padding-right", "10px"),
                ("border-width", "5px"),
                ("border-style", "solid"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        );
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 500.0);
        assert_eq!(fragment.content_rect.width, 100.0);
        assert_eq!(fragment.padding.left, 10.0);
        assert_eq!(fragment.border.left, 5.0);
        let bb = fragment.border_box();
        assert_eq!(bb.width, 100.0 + 20.0 + 10.0);
    }

    #[test]
    fn auto_width_fills_containing_block_minus_margins() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[]);
        let mut styles = StyleMap::new();
        styles.insert(
            div,
            [
                ("display", "block"),
                ("margin-left", "20px"),
                ("margin-right", "30px"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        );
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 500.0);
        assert_eq!(fragment.content_rect.width, 500.0 - 20.0 - 30.0);
    }

    #[test]
    fn sibling_margins_collapse_to_the_max() {
        let mut doc = Document::new();
        let root = doc.root();
        let parent = el(&mut doc, root, "div", &[]);
        let a = el(&mut doc, parent, "p", &[]);
        let b = el(&mut doc, parent, "p", &[]);
        let mut styles = StyleMap::new();
        styles.insert(
            parent,
            [("display", "block")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        );
        styles.insert(
            a,
            [
                ("display", "block"),
                ("height", "10px"),
                ("margin-bottom", "30px"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        );
        styles.insert(
            b,
            [
                ("display", "block"),
                ("height", "10px"),
                ("margin-top", "20px"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        );
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 500.0);
        let a_fragment = &fragment.children[0];
        let b_fragment = &fragment.children[1];
        // Collapsed gap should be max(30, 20) = 30, not 30+20=50.
        assert_eq!(
            b_fragment.border_box().y - a_fragment.border_box().y - a_fragment.border_box().height,
            30.0
        );
    }

    #[test]
    fn negative_and_positive_margin_collapse_subtracts_magnitude() {
        assert_eq!(collapse_margins(30.0, -10.0), 20.0);
        assert_eq!(collapse_margins(-30.0, -10.0), -30.0);
        assert_eq!(collapse_margins(10.0, 20.0), 20.0);
    }

    #[test]
    fn narrow_containing_width_wraps_text_into_multiple_lines() {
        use dom::NodeData;
        let doc = html::parse_document(
            "<html><body><p id=\"p\">one two three four five six seven eight</p></body></html>",
        );
        let mut p_node = None;
        doc.walk(doc.root(), &mut |id, _| {
            if p_node.is_none()
                && let NodeData::Element(e) = doc.data(id)
                && e.local_name == "p"
            {
                p_node = Some(id);
            }
        });
        let p_node = p_node.unwrap();
        let body_node = {
            let mut found = None;
            doc.walk(doc.root(), &mut |id, _| {
                if found.is_none()
                    && let NodeData::Element(e) = doc.data(id)
                    && e.local_name == "body"
                {
                    found = Some(id);
                }
            });
            found.unwrap()
        };
        // No UA-default stylesheet is applied by this test, so <body>/<p>
        // need an explicit `display: block` to behave like a real UA's
        // default -- otherwise <body> defaults to this crate's own
        // `inline` fallback (CSS's real initial value), which makes <p>
        // (nested one level inside it) hit the documented "block content
        // inside an inline box isn't split" gap instead of exercising the
        // block/inline formatting-context split this test is actually
        // about.
        let mut styles = StyleMap::new();
        styles.insert(
            body_node,
            [("display".to_string(), "block".to_string())].into(),
        );
        styles.insert(
            p_node,
            [("display".to_string(), "block".to_string())].into(),
        );
        let tree = build_box_tree(&doc, &styles).unwrap();
        let wide = layout(&tree, &styles, 2000.0);
        let narrow = layout(&tree, &styles, 60.0);
        fn find_fragment(f: &Fragment, node: NodeId) -> &Fragment {
            if f.node == Some(node) {
                return f;
            }
            for child in &f.children {
                if let Some(found) = try_find_fragment(child, node) {
                    return found;
                }
            }
            panic!("fragment for node not found");
        }
        fn try_find_fragment(f: &Fragment, node: NodeId) -> Option<&Fragment> {
            if f.node == Some(node) {
                return Some(f);
            }
            for child in &f.children {
                if let Some(found) = try_find_fragment(child, node) {
                    return Some(found);
                }
            }
            None
        }
        assert_eq!(find_fragment(&wide, p_node).children.len(), 1);
        assert!(find_fragment(&narrow, p_node).children.len() > 1);
    }

    #[test]
    fn float_left_is_positioned_at_left_edge_and_out_of_flow_stacking() {
        let mut doc = Document::new();
        let root = doc.root();
        let parent = el(&mut doc, root, "div", &[]);
        let floated = el(&mut doc, parent, "div", &[]);
        let after = el(&mut doc, parent, "div", &[]);
        let mut styles = StyleMap::new();
        styles.insert(
            parent,
            [("display", "block")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        );
        styles.insert(
            floated,
            [
                ("display", "block"),
                ("float", "left"),
                ("width", "50px"),
                ("height", "40px"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        );
        styles.insert(
            after,
            [("display", "block"), ("height", "10px")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        );
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 500.0);
        let float_fragment = &fragment.children[0];
        let after_fragment = &fragment.children[1];
        assert_eq!(float_fragment.border_box().x, 0.0);
        // The float doesn't consume vertical space for the next in-flow
        // sibling (no `clear`), so `after` starts at y=0 too, not y=40.
        assert_eq!(after_fragment.border_box().y, 0.0);
    }

    #[test]
    fn clear_pushes_below_the_float() {
        let mut doc = Document::new();
        let root = doc.root();
        let parent = el(&mut doc, root, "div", &[]);
        let floated = el(&mut doc, parent, "div", &[]);
        let after = el(&mut doc, parent, "div", &[]);
        let mut styles = StyleMap::new();
        styles.insert(
            parent,
            [("display", "block")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        );
        styles.insert(
            floated,
            [
                ("display", "block"),
                ("float", "left"),
                ("width", "50px"),
                ("height", "40px"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        );
        styles.insert(
            after,
            [("display", "block"), ("clear", "left"), ("height", "10px")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        );
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 500.0);
        let after_fragment = &fragment.children[1];
        assert_eq!(after_fragment.border_box().y, 40.0);
    }
}
