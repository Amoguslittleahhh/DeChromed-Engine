//! B2: block & inline formatting contexts -- turning a box tree (B1) into
//! a fragment tree of positioned, sized boxes. Also B3 (table layout,
//! [`layout_table`]), B4 (flexbox, [`layout_flex_container`]), B5 (grid,
//! [`layout_grid_container`]), and the start of B6 (`position: relative`/
//! `absolute`/`fixed`, in [`layout_block_children`]).
//!
//! Reference: <https://www.w3.org/TR/CSS22/visuren.html#normal-flow>,
//! <https://www.w3.org/TR/CSS22/box.html#box-dimensions>,
//! <https://www.w3.org/TR/CSS22/visudet.html> (width/height resolution),
//! <https://www.w3.org/TR/CSS22/box.html#collapsing-margins> (margin
//! collapsing), <https://www.w3.org/TR/css-flexbox-1/> (B4),
//! <https://www.w3.org/TR/css-grid-1/> (B5),
//! <https://www.w3.org/TR/CSS22/visuren.html#propdef-position> (B6).
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
//! - **B3 table layout is simplified** (no min/max-content sizing pass,
//!   no `rowspan`, no `border-spacing`) -- see `box_tree.rs`'s module
//!   docs for the full list, and [`layout_table`]'s own doc comment for
//!   the column-width algorithm actually used.
//! - **B4 flexbox is real but scoped**: `flex-wrap` and cross-axis
//!   `stretch` are only implemented for `flex-direction: row` (`column`
//!   direction is always single-line, and its cross axis -- width --
//!   never stretches, since that would need a second content-reflow pass
//!   this phase doesn't perform); `flex-grow`/`flex-shrink` distribution
//!   is the real weighted algorithm but clamps shrunk items at a `0`
//!   floor rather than a real min-content size (no intrinsic sizing,
//!   same gap as above); `order`, `gap`/`row-gap`/`column-gap`, and
//!   multi-line `align-content` spacing aren't implemented. See
//!   [`layout_flex_container`]'s own doc comment for the axis-agnostic
//!   approach the algorithm takes.
//! - **B5 grid is significantly scoped**: columns come from
//!   `grid-template-columns` only (rows are always implicit, sizing to
//!   content or a matching `Fixed` `grid-template-rows` entry -- `fr`/
//!   `auto` row tracks are effectively unused, since there's no definite
//!   grid container height to distribute them against in general);
//!   `repeat()`, `minmax()`, named lines, and subgrid aren't implemented;
//!   placement only reads `grid-column` (`grid-row` isn't implemented,
//!   and the `"start / end"` range syntax isn't either -- only a bare
//!   line number or `span N`). See [`layout_grid_container`]'s own doc
//!   comment for exactly what the placement/sizing algorithm does.
//! - **B6 positioning is just started**: `position: relative` is real
//!   (still fully in-flow for sizing/margin-collapsing/sibling
//!   positioning purposes, only its own final visual position shifts).
//!   `absolute`/`fixed` are simplified -- both are taken out of normal
//!   flow and positioned via `top`/`left` (pixel lengths only, not
//!   percentages, and `right`/`bottom` aren't read at all) relative to
//!   the **immediate parent's** content-box origin, not the spec's real
//!   "nearest positioned ancestor" (which needs ancestor-chain
//!   position-type tracking this phase doesn't implement) or, for
//!   `fixed`, the viewport (no distinct viewport/scroll-container
//!   concept exists yet, so `fixed` behaves identically to `absolute`
//!   here). `position: sticky` isn't implemented at all (behaves as
//!   `static`, since there's no scroll container to stick within).
//!   `z-index`/stacking contexts/paint order aren't addressed by this
//!   landing -- there's no paint pipeline yet (B10-B12) for a stacking
//!   order to actually affect, so it's out of scope rather than faked.

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

/// B6: resolves `top`/`right`/`bottom`/`left` -- pixel lengths only (not
/// percentages, which would need a definite containing-block dimension
/// this phase doesn't resolve in general -- see module docs' positioning
/// known gap); `auto` or anything else unresolvable returns `None`.
fn resolve_offset(
    style: Option<&ComputedStyle>,
    property: &str,
    font_size_px: f64,
    root_font_size_px: f64,
) -> Option<f64> {
    values::parse_length_px(
        get(style, property, "auto"),
        font_size_px,
        root_font_size_px,
    )
}

/// Lays out `root` (B1's box tree) as if it were the sole child of an
/// (unstyled) containing block `containing_width` pixels wide. This is the
/// crate's top-level entry point.
pub fn layout(root: &LayoutBox, styles: &StyleMap, containing_width: f64) -> Fragment {
    let (root_font_size, root_font_size_raw) = resolve_font_size(
        root.node,
        styles,
        values::default_font_size_px(),
        None,
        values::default_font_size_px(),
    );
    layout_box(
        root,
        styles,
        containing_width,
        root_font_size,
        root_font_size_raw.as_deref(),
        root_font_size,
    )
}

/// Resolves `node`'s `font-size`, returning both the resolved pixel value
/// and the raw computed-style string that produced it (for the caller to
/// pass back in as `parent_font_size_raw` when resolving *this* node's
/// children).
///
/// `css::cascade` doesn't resolve `font-size`'s relative units (`%`/`em`)
/// before inheriting it -- a documented A6/A7 "no used-value resolution"
/// gap -- so when a node has no matching rule for `font-size`, cascade's
/// inheritance just copies the parent's *unresolved* computed string
/// through verbatim (see `PROPERTY_TABLE`'s `inherited_value()` in
/// `css::cascade`). Without this check, re-resolving that copied-through
/// string against the parent's already-resolved pixel size on every
/// generation compounds a relative `font-size` (e.g. `150%`) once per
/// descendant that doesn't redeclare it, instead of it being fixed once
/// where actually declared. Comparing the node's own raw string against
/// `parent_font_size_raw` detects "this is cascade's inheritance copy,
/// not a fresh declaration" and reuses the parent's resolved size instead.
fn resolve_font_size(
    node: Option<NodeId>,
    styles: &StyleMap,
    parent_font_size_px: f64,
    parent_font_size_raw: Option<&str>,
    root_font_size_px: f64,
) -> (f64, Option<String>) {
    let own_raw = node
        .and_then(|n| styles.get(&n))
        .and_then(|s| s.get("font-size"))
        .map(String::as_str);
    match own_raw {
        None => (
            parent_font_size_px,
            parent_font_size_raw.map(str::to_string),
        ),
        Some(raw) if Some(raw) == parent_font_size_raw => {
            (parent_font_size_px, Some(raw.to_string()))
        }
        Some(raw) => (
            values::resolve_font_size_px(raw, parent_font_size_px, root_font_size_px),
            Some(raw.to_string()),
        ),
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
    // `css::cascade`'s property table only has a single (non-per-side)
    // `border-width`/`border-style`, so all four sides always share one
    // resolved value -- computed once here rather than re-parsed per side.
    let border_width_px = if get(style, "border-style", "none") == "none" {
        0.0
    } else {
        values::parse_border_width_px(
            get(style, "border-width", "medium"),
            font_size_px,
            root_font_size_px,
        )
    };
    let border = EdgeSizes {
        top: border_width_px,
        right: border_width_px,
        bottom: border_width_px,
        left: border_width_px,
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
    parent_font_size_raw: Option<&str>,
    root_font_size_px: f64,
) -> Fragment {
    let style = b.node.and_then(|n| styles.get(&n));
    let (font_size_px, font_size_raw) = resolve_font_size(
        b.node,
        styles,
        parent_font_size_px,
        parent_font_size_raw,
        root_font_size_px,
    );
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
                    font_size_raw.as_deref(),
                    root_font_size_px,
                )
            } else {
                layout_inline_children(
                    children,
                    styles,
                    model.content_width,
                    font_size_px,
                    font_size_raw.as_deref(),
                    root_font_size_px,
                    b.node,
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
        BoxKind::FlexContainer(items) => {
            let (child_fragments, content_height) = layout_flex_container(
                items,
                styles,
                style,
                model.content_width,
                font_size_px,
                font_size_raw.as_deref(),
                root_font_size_px,
            );
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
        BoxKind::GridContainer(items) => {
            let (child_fragments, content_height) = layout_grid_container(
                items,
                styles,
                style,
                model.content_width,
                font_size_px,
                font_size_raw.as_deref(),
                root_font_size_px,
            );
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
        BoxKind::Table(rows) => {
            let (child_fragments, content_height) = layout_table(
                rows,
                styles,
                model.content_width,
                font_size_px,
                font_size_raw.as_deref(),
                root_font_size_px,
            );
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
        BoxKind::TableCell { children, .. } => {
            let is_bfc = children.iter().any(|c| c.level == BoxLevel::Block);
            let (child_fragments, content_height) = if is_bfc {
                layout_block_children(
                    children,
                    styles,
                    model.content_width,
                    font_size_px,
                    font_size_raw.as_deref(),
                    root_font_size_px,
                )
            } else {
                layout_inline_children(
                    children,
                    styles,
                    model.content_width,
                    font_size_px,
                    font_size_raw.as_deref(),
                    root_font_size_px,
                    b.node,
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
        BoxKind::TableRow(_) => {
            unreachable!("TableRow is only laid out from within layout_table")
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
    font_size_raw: Option<&str>,
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
        let position = get(style, "position", "static");

        if position == "absolute" || position == "fixed" {
            // B6: taken out of normal flow entirely (no margin
            // collapsing, no cursor_y contribution, no float
            // interaction) and positioned via `top`/`left` -- see
            // module docs for this phase's containing-block
            // simplification (always the immediate parent, not the
            // spec's "nearest positioned ancestor") and why `fixed`
            // behaves identically to `absolute` here (no viewport
            // concept to give it distinct behavior).
            let mut fragment = layout_box(
                child,
                styles,
                containing_width,
                font_size_px,
                font_size_raw,
                root_font_size_px,
            );
            let x = resolve_offset(style, "left", font_size_px, root_font_size_px).unwrap_or(0.0);
            let y = resolve_offset(style, "top", font_size_px, root_font_size_px).unwrap_or(0.0);
            reposition(&mut fragment, x, y);
            out.push(fragment);
            continue;
        }

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
            font_size_raw,
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
        if position == "relative" {
            // B6: still fully participates in normal flow (sizing,
            // margin collapsing, and every sibling's position are
            // computed as if this offset didn't exist) -- only the
            // box's own visual position shifts afterward.
            let dx = resolve_offset(style, "left", font_size_px, root_font_size_px).unwrap_or(0.0);
            let dy = resolve_offset(style, "top", font_size_px, root_font_size_px).unwrap_or(0.0);
            shift(&mut fragment, dx, dy);
        }
        out.push(fragment);
    }
    if let Some(bottom) = prev_margin_bottom {
        cursor_y += bottom;
    }
    (out, cursor_y)
}

/// B3: a simplified CSS2.1 table layout -- see module docs for exactly
/// what's cut relative to the real automatic-table-layout algorithm (no
/// min/max-content sizing pass, no `rowspan`, no `border-spacing`).
/// Column count and per-column widths are determined first (single-
/// colspan cells with an explicit `width` hint their column; the rest of
/// the width splits evenly among unhinted columns), then every row's
/// cells are laid out at their spanned column width and stacked
/// top-to-bottom.
fn layout_table(
    rows: &[LayoutBox],
    styles: &StyleMap,
    content_width: f64,
    font_size_px: f64,
    font_size_raw: Option<&str>,
    root_font_size_px: f64,
) -> (Vec<Fragment>, f64) {
    let mut column_count = 0usize;
    let mut column_hint: Vec<Option<f64>> = Vec::new();
    for row in rows {
        let BoxKind::TableRow(cells) = &row.kind else {
            continue;
        };
        let mut col = 0usize;
        for cell in cells {
            let BoxKind::TableCell { colspan, .. } = &cell.kind else {
                continue;
            };
            if column_hint.len() < col + colspan {
                column_hint.resize(col + colspan, None);
            }
            if *colspan == 1 {
                let cell_style = cell.node.and_then(|n| styles.get(&n));
                if let LengthPercentageAuto::Length(px) = values::parse_length_percentage_auto(
                    get(cell_style, "width", "auto"),
                    font_size_px,
                    root_font_size_px,
                ) {
                    column_hint[col] =
                        Some(column_hint[col].map_or(px, |existing| existing.max(px)));
                }
            }
            col += colspan;
        }
        column_count = column_count.max(col);
    }
    if column_count == 0 {
        return (Vec::new(), 0.0);
    }

    let hinted_total: f64 = column_hint.iter().flatten().sum();
    let unhinted_count = column_count - column_hint.iter().filter(|h| h.is_some()).count();
    let remaining = (content_width - hinted_total).max(0.0);
    let share = if unhinted_count > 0 {
        remaining / unhinted_count as f64
    } else {
        0.0
    };
    let mut column_widths = vec![0.0; column_count];
    for (i, width) in column_widths.iter_mut().enumerate() {
        *width = column_hint.get(i).copied().flatten().unwrap_or(share);
    }
    let mut column_x = vec![0.0; column_count + 1];
    for i in 0..column_count {
        column_x[i + 1] = column_x[i] + column_widths[i];
    }

    let mut row_fragments = Vec::new();
    let mut cursor_y = 0.0;
    for row in rows {
        let BoxKind::TableRow(cells) = &row.kind else {
            continue;
        };
        let mut col = 0usize;
        let mut cell_fragments = Vec::new();
        let mut row_height = 0.0f64;
        for cell in cells {
            let BoxKind::TableCell { colspan, .. } = &cell.kind else {
                continue;
            };
            let span_end = (col + colspan).min(column_count);
            let cell_width = (column_x[span_end] - column_x[col]).max(0.0);
            let mut fragment = layout_box(
                cell,
                styles,
                cell_width,
                font_size_px,
                font_size_raw,
                root_font_size_px,
            );
            reposition(&mut fragment, column_x[col], 0.0);
            row_height = row_height.max(fragment.border_box().height);
            cell_fragments.push(fragment);
            col += colspan;
        }
        let mut row_fragment = Fragment {
            node: row.node,
            content_rect: Rect {
                x: 0.0,
                y: 0.0,
                width: content_width,
                height: row_height,
            },
            margin: EdgeSizes::default(),
            border: EdgeSizes::default(),
            padding: EdgeSizes::default(),
            children: cell_fragments,
            text: None,
        };
        reposition(&mut row_fragment, 0.0, cursor_y);
        cursor_y += row_height;
        row_fragments.push(row_fragment);
    }
    (row_fragments, cursor_y)
}

/// B4: [CSS Flexible Box Layout](https://www.w3.org/TR/css-flexbox-1/),
/// real but scoped -- see module docs for exactly what's cut. Works in
/// abstract main/cross axis terms throughout (`column` selects which of
/// width/height is "main"), so the same code handles `flex-direction:
/// row`/`column`; only wrapping and cross-axis stretch are restricted to
/// row direction, both documented below and in the module docs.
fn layout_flex_container(
    items: &[LayoutBox],
    styles: &StyleMap,
    container_style: Option<&ComputedStyle>,
    content_width: f64,
    font_size_px: f64,
    font_size_raw: Option<&str>,
    root_font_size_px: f64,
) -> (Vec<Fragment>, f64) {
    let direction = get(container_style, "flex-direction", "row");
    let column = direction == "column" || direction == "column-reverse";
    let reverse = direction == "row-reverse" || direction == "column-reverse";
    // Wrap and cross-axis stretch are only implemented for row direction:
    // row direction's main axis (width) is always a known/bounded size,
    // so "does the next item still fit on this line" is well-defined and
    // stretch (a height override) doesn't require re-flowing content.
    // Column direction's main axis (height) is usually auto/unbounded
    // (no line-break point) and its cross axis (width) *would* need
    // content to re-flow at a new width to really stretch -- both
    // deferred, see module docs.
    let wraps = !column && get(container_style, "flex-wrap", "nowrap") != "nowrap";
    let justify = get(container_style, "justify-content", "normal");
    let align_items_default = get(container_style, "align-items", "normal");
    let main_bound: Option<f64> = if column { None } else { Some(content_width) };

    struct Item<'a> {
        b: &'a LayoutBox,
        margin: EdgeSizes,
        border: EdgeSizes,
        padding: EdgeSizes,
        natural_cross_content: f64, // ordinary (non-flex) resolution along the CROSS axis, reused as-is for column direction's width
        basis: f64,
        grow: f64,
        shrink: f64,
        align_self: String,
        cross_auto: bool,
    }

    let prepared: Vec<Item> = items
        .iter()
        .map(|b| {
            let style = b.node.and_then(|n| styles.get(&n));
            let model = resolve_box_model(style, content_width, font_size_px, root_font_size_px);
            let basis = resolve_flex_basis(
                style,
                column,
                content_width,
                font_size_px,
                root_font_size_px,
            )
            .unwrap_or(0.0);
            let grow = get(style, "flex-grow", "0")
                .parse::<f64>()
                .unwrap_or(0.0)
                .max(0.0);
            let shrink = get(style, "flex-shrink", "1")
                .parse::<f64>()
                .unwrap_or(1.0)
                .max(0.0);
            let align_self_raw = get(style, "align-self", "auto");
            let align_self = if align_self_raw == "auto" {
                align_items_default.to_string()
            } else {
                align_self_raw.to_string()
            };
            let cross_prop = if column { "width" } else { "height" };
            let cross_auto = get(style, cross_prop, "auto") == "auto";
            Item {
                b,
                margin: model.margin,
                border: model.border,
                padding: model.padding,
                natural_cross_content: model.content_width,
                basis,
                grow,
                shrink,
                align_self,
                cross_auto,
            }
        })
        .collect();

    let main_extra = |it: &Item| -> f64 {
        if column {
            it.margin.top
                + it.margin.bottom
                + it.border.top
                + it.border.bottom
                + it.padding.top
                + it.padding.bottom
        } else {
            it.margin.left
                + it.margin.right
                + it.border.left
                + it.border.right
                + it.padding.left
                + it.padding.right
        }
    };

    let order: Vec<usize> = if reverse {
        (0..prepared.len()).rev().collect()
    } else {
        (0..prepared.len()).collect()
    };

    // Line-breaking: greedily accumulate items until the next one would
    // overflow `main_bound` (row direction only -- `wraps` is false for
    // column, so this is always a single line there).
    let mut lines: Vec<Vec<usize>> = vec![Vec::new()];
    let mut line_main_sum: Vec<f64> = vec![0.0];
    for &idx in &order {
        let outer = prepared[idx].basis + main_extra(&prepared[idx]);
        let cur = lines.len() - 1;
        if wraps
            && line_main_sum[cur] > 0.0
            && line_main_sum[cur] + outer > main_bound.unwrap_or(f64::INFINITY)
        {
            lines.push(Vec::new());
            line_main_sum.push(0.0);
        }
        let cur = lines.len() - 1;
        line_main_sum[cur] += outer;
        lines[cur].push(idx);
    }

    // Flex-grow/shrink: real CSS Flexible Box §9.7 distribution, per
    // line -- positive free space distributes by `flex-grow` weight,
    // negative free space (overflow) distributes by `flex-shrink *
    // basis` weight, clamped at a 0 floor (a real UA clamps at the
    // item's min-content size instead, which needs intrinsic sizing this
    // project doesn't have yet -- documented gap).
    let mut final_main = vec![0.0; prepared.len()];
    for line in &lines {
        let sum_outer: f64 = line
            .iter()
            .map(|&i| prepared[i].basis + main_extra(&prepared[i]))
            .sum();
        let bound = main_bound.unwrap_or(sum_outer);
        let free = bound - sum_outer;
        let sum_grow: f64 = line.iter().map(|&i| prepared[i].grow).sum();
        let sum_shrink_basis: f64 = line
            .iter()
            .map(|&i| prepared[i].shrink * prepared[i].basis)
            .sum();
        for &i in line {
            let it = &prepared[i];
            final_main[i] = if free > 0.0 && sum_grow > 0.0 {
                it.basis + free * (it.grow / sum_grow)
            } else if free < 0.0 && sum_shrink_basis > 0.0 {
                (it.basis + free * (it.shrink * it.basis) / sum_shrink_basis).max(0.0)
            } else {
                it.basis
            };
        }
    }

    // Lay out each item's own content at its resolved main size (see
    // `layout_flex_item_content`), producing an unstretched/"hypothetical"
    // cross size per item.
    let mut item_fragments: Vec<Fragment> = Vec::with_capacity(prepared.len());
    for (idx, it) in prepared.iter().enumerate() {
        let children_containing_width = if column {
            it.natural_cross_content
        } else {
            final_main[idx]
        };
        let (child_fragments, natural_content_main_or_cross) = layout_flex_item_content(
            it.b,
            styles,
            children_containing_width,
            font_size_px,
            font_size_raw,
            root_font_size_px,
        );
        let style = it.b.node.and_then(|n| styles.get(&n));
        let (width, height) = if column {
            (it.natural_cross_content, final_main[idx])
        } else {
            let explicit_height = values::parse_length_percentage_auto(
                get(style, "height", "auto"),
                font_size_px,
                root_font_size_px,
            );
            let height = match explicit_height {
                LengthPercentageAuto::Length(px) => px,
                LengthPercentageAuto::Percentage(_) | LengthPercentageAuto::Auto => {
                    natural_content_main_or_cross
                }
            };
            (final_main[idx], height)
        };
        item_fragments.push(Fragment {
            node: it.b.node,
            content_rect: Rect {
                x: 0.0,
                y: 0.0,
                width,
                height,
            },
            margin: it.margin,
            border: it.border,
            padding: it.padding,
            children: child_fragments,
            text: None,
        });
    }

    // Cross size per line (max item outer cross size in that line), then
    // row-direction stretch overrides -- see the `wraps`/module-docs note
    // on why column direction doesn't stretch.
    let cross_of = |f: &Fragment| -> f64 {
        if column {
            f.border_box().width
        } else {
            f.border_box().height
        }
    };
    let line_cross_size: Vec<f64> = lines
        .iter()
        .map(|line| {
            line.iter()
                .map(|&i| cross_of(&item_fragments[i]))
                .fold(0.0, f64::max)
        })
        .collect();
    if !column {
        for (line_idx, line) in lines.iter().enumerate() {
            for &i in line {
                let stretches =
                    prepared[i].align_self == "stretch" || prepared[i].align_self == "normal";
                if stretches && prepared[i].cross_auto {
                    let target = line_cross_size[line_idx]
                        - prepared[i].margin.top
                        - prepared[i].margin.bottom
                        - prepared[i].border.top
                        - prepared[i].border.bottom
                        - prepared[i].padding.top
                        - prepared[i].padding.bottom;
                    item_fragments[i].content_rect.height = target.max(0.0);
                }
            }
        }
    }

    // Position: main-axis via `justify-content`, cross-axis (within its
    // line) via `align-items`/`align-self`, lines stacked along the cross
    // axis in order.
    let mut cross_cursor = 0.0;
    for (line_idx, line) in lines.iter().enumerate() {
        let line_cross = line_cross_size[line_idx];
        let used_main: f64 = line
            .iter()
            .map(|&i| final_main[i] + main_extra(&prepared[i]))
            .sum();
        let free_main = main_bound.map(|b| (b - used_main).max(0.0)).unwrap_or(0.0);
        let n = line.len();
        let (mut main_cursor, gap) = match justify {
            "flex-end" => (free_main, 0.0),
            "center" => (free_main / 2.0, 0.0),
            "space-between" if n > 1 => (0.0, free_main / (n - 1) as f64),
            "space-around" if n > 0 => (free_main / n as f64 / 2.0, free_main / n as f64),
            _ => (0.0, 0.0),
        };
        for &i in line {
            let it = &prepared[i];
            let main_margin_start = if column {
                it.margin.top
            } else {
                it.margin.left
            };
            let cross_margin_start = if column {
                it.margin.left
            } else {
                it.margin.top
            };
            let item_cross_outer = cross_of(&item_fragments[i]);
            let align_offset = match it.align_self.as_str() {
                "flex-end" => line_cross - item_cross_outer,
                "center" => (line_cross - item_cross_outer) / 2.0,
                _ => 0.0, // flex-start/stretch/normal/baseline(unsupported, falls back to flex-start)
            };
            let main_pos = main_cursor + main_margin_start;
            let cross_pos = cross_cursor + cross_margin_start + align_offset.max(0.0);
            let (x, y) = if column {
                (cross_pos, main_pos)
            } else {
                (main_pos, cross_pos)
            };
            reposition(&mut item_fragments[i], x, y);
            main_cursor += final_main[i] + main_extra(it) + gap;
        }
        cross_cursor += line_cross;
    }

    let content_main_or_cross_total = if column {
        lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|&i| final_main[i] + main_extra(&prepared[i]))
                    .sum::<f64>()
            })
            .fold(0.0, f64::max)
    } else {
        cross_cursor
    };
    (item_fragments, content_main_or_cross_total)
}

/// See `layout_flex_container`'s `basis` field: resolves `flex-basis`
/// (falling back to `width`/`height`, whichever is the main-axis
/// property, when `flex-basis` is `auto`) into a content-box pixel size.
/// Returns `None` when nothing resolvable is found (truly `auto` with no
/// explicit main-axis length either) -- the caller falls back to `0.0`,
/// the same "no intrinsic sizing" simplification B2 already documents.
fn resolve_flex_basis(
    style: Option<&ComputedStyle>,
    column: bool,
    content_width: f64,
    font_size_px: f64,
    root_font_size_px: f64,
) -> Option<f64> {
    let basis_raw = get(style, "flex-basis", "auto");
    if basis_raw != "auto" {
        match values::parse_length_percentage_auto(basis_raw, font_size_px, root_font_size_px) {
            LengthPercentageAuto::Length(px) => return Some(px),
            LengthPercentageAuto::Percentage(pct) if !column => {
                return Some(content_width * pct / 100.0);
            }
            _ => {}
        }
    }
    let main_prop = if column { "height" } else { "width" };
    match values::parse_length_percentage_auto(
        get(style, main_prop, "auto"),
        font_size_px,
        root_font_size_px,
    ) {
        LengthPercentageAuto::Length(px) => Some(px),
        LengthPercentageAuto::Percentage(pct) if !column => Some(content_width * pct / 100.0),
        _ => None,
    }
}

/// Lays out a flex item's own children (its `Container`'s block/inline
/// formatting context, same dispatch `layout_box` uses) at
/// `children_containing_width`, returning the child fragments and the
/// resulting natural content size along whichever axis ordinary block
/// layout would compute automatically (content_height, in the usual
/// sense) -- the caller decides whether that's the item's cross size
/// (row direction) or gets discarded in favor of the flex-resolved main
/// size (column direction).
fn layout_flex_item_content(
    b: &LayoutBox,
    styles: &StyleMap,
    children_containing_width: f64,
    font_size_px: f64,
    font_size_raw: Option<&str>,
    root_font_size_px: f64,
) -> (Vec<Fragment>, f64) {
    match &b.kind {
        BoxKind::Container(children) => {
            let is_bfc = children.iter().any(|c| c.level == BoxLevel::Block);
            if is_bfc {
                layout_block_children(
                    children,
                    styles,
                    children_containing_width,
                    font_size_px,
                    font_size_raw,
                    root_font_size_px,
                )
            } else {
                layout_inline_children(
                    children,
                    styles,
                    children_containing_width,
                    font_size_px,
                    font_size_raw,
                    root_font_size_px,
                    b.node,
                )
            }
        }
        // B1's `flex_items` always blockifies real elements into
        // `Container` boxes (an anonymous wrapper for stray inline
        // content is also a `Container`); nested table/flex formatting
        // contexts directly as a flex item aren't laid out specially
        // here yet -- documented gap, falls back to empty content rather
        // than panicking.
        _ => (Vec::new(), 0.0),
    }
}

/// B5: a resolved grid track -- either a fixed pixel size or a `fr`-style
/// weight sharing whatever space is left after every `Fixed` track is
/// subtracted. `auto` tracks are treated as `Fraction(1.0)` (a documented
/// simplification -- a real `auto` track sizes to its content, which
/// needs intrinsic sizing this project doesn't have yet).
#[derive(Debug, Clone, Copy)]
enum TrackSize {
    Fixed(f64),
    Fraction(f64),
}

/// Parses `grid-template-columns`/`grid-template-rows`'s value into a
/// track list. Only bare `<length>`, `<percentage>`, `fr`, and `auto`
/// tokens are recognized -- `repeat()`, `minmax()`, named lines, and
/// subgrid aren't implemented (see module docs); an unparseable token
/// falls back to a `0`-width fixed track rather than panicking.
fn parse_track_list(
    value: &str,
    available_for_percentage: f64,
    font_size_px: f64,
    root_font_size_px: f64,
) -> Vec<TrackSize> {
    if value == "none" {
        return Vec::new();
    }
    value
        .split_whitespace()
        .map(|token| {
            if let Some(n) = token.strip_suffix("fr") {
                TrackSize::Fraction(n.parse::<f64>().unwrap_or(1.0).max(0.0))
            } else if token == "auto" {
                TrackSize::Fraction(1.0)
            } else if let Some(n) = token.strip_suffix('%') {
                TrackSize::Fixed(available_for_percentage * n.parse().unwrap_or(0.0) / 100.0)
            } else {
                TrackSize::Fixed(
                    values::parse_length_px(token, font_size_px, root_font_size_px).unwrap_or(0.0),
                )
            }
        })
        .collect()
}

/// Resolves each track's final pixel size: `Fixed` tracks keep their
/// value; the space left over (available minus the sum of fixed tracks)
/// splits among `Fraction` tracks proportional to their weight.
fn resolve_track_sizes(tracks: &[TrackSize], available: f64) -> Vec<f64> {
    let fixed_total: f64 = tracks
        .iter()
        .map(|t| match t {
            TrackSize::Fixed(px) => *px,
            TrackSize::Fraction(_) => 0.0,
        })
        .sum();
    let fraction_total: f64 = tracks
        .iter()
        .map(|t| match t {
            TrackSize::Fraction(w) => *w,
            TrackSize::Fixed(_) => 0.0,
        })
        .sum();
    let remaining = (available - fixed_total).max(0.0);
    tracks
        .iter()
        .map(|t| match t {
            TrackSize::Fixed(px) => *px,
            TrackSize::Fraction(w) if fraction_total > 0.0 => remaining * (w / fraction_total),
            TrackSize::Fraction(_) => 0.0,
        })
        .collect()
}

fn track_prefix_sums(sizes: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; sizes.len() + 1];
    for (i, size) in sizes.iter().enumerate() {
        out[i + 1] = out[i] + size;
    }
    out
}

/// B5: resolves `grid-column`/`grid-row`'s value into a `(explicit
/// 0-based start line, span)` pair -- `None` start means auto-placed.
/// Only three forms are recognized: a bare integer (`"2"`, 1-based, span
/// 1), `"span N"` (auto start, explicit span), and `"auto"`; the
/// `"start / end"` range syntax isn't implemented (a documented gap --
/// see module docs), and unparseable/other input falls back to `auto`.
fn resolve_grid_placement(style: Option<&ComputedStyle>, property: &str) -> (Option<usize>, usize) {
    let raw = get(style, property, "auto").trim();
    if raw == "auto" {
        return (None, 1);
    }
    if let Some(rest) = raw.strip_prefix("span") {
        let span = rest.trim().parse::<usize>().unwrap_or(1).max(1);
        return (None, span);
    }
    if let Ok(n) = raw.parse::<i64>() {
        let start = (n - 1).max(0) as usize;
        return (Some(start), 1);
    }
    (None, 1)
}

/// B5: [CSS Grid Layout](https://www.w3.org/TR/css-grid-1/), real but
/// significantly scoped -- see module docs for the full list of cuts.
/// Column tracks come from `grid-template-columns` (a single implicit
/// full-width column if absent); rows are **always implicit**, growing
/// one at a time as auto-placement needs them, and size to the tallest
/// item placed in them (or to `grid-template-rows`' matching `Fixed`
/// track, if one exists at that row index) -- `grid-template-rows`'
/// `fr`/`auto` entries are effectively unused, since there's no definite
/// grid container height to distribute them against in the general case.
/// Placement only reads `grid-column` (`grid-row` isn't implemented, see
/// module docs); un-placed items auto-flow row-major, skipping cells an
/// earlier explicitly-placed item already claimed.
fn layout_grid_container(
    items: &[LayoutBox],
    styles: &StyleMap,
    container_style: Option<&ComputedStyle>,
    content_width: f64,
    font_size_px: f64,
    font_size_raw: Option<&str>,
    root_font_size_px: f64,
) -> (Vec<Fragment>, f64) {
    let column_tracks = parse_track_list(
        get(container_style, "grid-template-columns", "none"),
        content_width,
        font_size_px,
        root_font_size_px,
    );
    let column_tracks = if column_tracks.is_empty() {
        vec![TrackSize::Fixed(content_width)]
    } else {
        column_tracks
    };
    let column_count = column_tracks.len();
    let column_widths = resolve_track_sizes(&column_tracks, content_width);
    let column_x = track_prefix_sums(&column_widths);

    let row_tracks = parse_track_list(
        get(container_style, "grid-template-rows", "none"),
        0.0,
        font_size_px,
        root_font_size_px,
    );
    let explicit_row_height = |row: usize| -> Option<f64> {
        match row_tracks.get(row) {
            Some(TrackSize::Fixed(px)) => Some(*px),
            _ => None,
        }
    };

    // Placement pass: explicitly-column-placed items search forward from
    // the current auto-placement cursor row for the first row where their
    // column span is free; auto-placed items advance the cursor row-major,
    // skipping any cell an earlier item already claimed.
    let mut occupied: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    let mut placements: Vec<(usize, usize, usize)> = Vec::with_capacity(items.len()); // (row, col_start, span)
    let mut cursor_col = 0usize;
    let mut cursor_row = 0usize;
    for item in items {
        let style = item.node.and_then(|n| styles.get(&n));
        let (explicit_start, span) = resolve_grid_placement(style, "grid-column");
        let span = span.min(column_count.max(1));
        let (row, col_start) = match explicit_start {
            Some(start) => {
                let start = start.min(column_count.saturating_sub(1));
                let mut row = cursor_row;
                loop {
                    if (start..(start + span).min(column_count))
                        .all(|c| !occupied.contains(&(row, c)))
                    {
                        break;
                    }
                    row += 1;
                }
                (row, start)
            }
            None => loop {
                if cursor_col + span > column_count {
                    cursor_col = 0;
                    cursor_row += 1;
                    continue;
                }
                if (cursor_col..cursor_col + span).all(|c| !occupied.contains(&(cursor_row, c))) {
                    let placed = (cursor_row, cursor_col);
                    cursor_col += span;
                    break placed;
                }
                cursor_col += 1;
            },
        };
        for c in col_start..(col_start + span).min(column_count) {
            occupied.insert((row, c));
        }
        placements.push((row, col_start, span));
    }

    // Lay out each item at its spanned column width (an ordinary
    // auto-width block box correctly fills that span -- unlike Flexbox,
    // no box-model bypass is needed here) and find each row's height.
    let mut item_fragments = Vec::with_capacity(items.len());
    let row_count = placements
        .iter()
        .map(|(row, _, _)| row + 1)
        .max()
        .unwrap_or(0);
    let mut row_heights: Vec<f64> = vec![0.0; row_count];
    for (item, &(row, col_start, span)) in items.iter().zip(&placements) {
        let col_end = (col_start + span).min(column_count);
        let span_width = (column_x[col_end] - column_x[col_start]).max(0.0);
        let fragment = layout_box(
            item,
            styles,
            span_width,
            font_size_px,
            font_size_raw,
            root_font_size_px,
        );
        row_heights[row] = row_heights[row].max(fragment.border_box().height);
        item_fragments.push(fragment);
    }
    for (row, height) in row_heights.iter_mut().enumerate() {
        if let Some(explicit) = explicit_row_height(row) {
            *height = explicit;
        }
    }
    let row_y = track_prefix_sums(&row_heights);

    for (fragment, &(row, col_start, _)) in item_fragments.iter_mut().zip(&placements) {
        reposition(fragment, column_x[col_start], row_y[row]);
    }

    let content_height = row_y.last().copied().unwrap_or(0.0);
    (item_fragments, content_height)
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
        /// The leaf text node itself -- used as the produced `Fragment`'s
        /// identity, not for style lookups (text nodes never have their
        /// own computed style; only `style_node` does).
        text_node: Option<NodeId>,
        /// The nearest enclosing *element* (the innermost inline box
        /// actually wrapping this text, or the containing block box
        /// itself if none does) -- inherited properties like
        /// `line-height` are looked up against this, not `text_node`.
        style_node: Option<NodeId>,
        text: String,
        font_size_px: f64,
    },
    Atomic(&'a LayoutBox, f64, Option<String>),
}

fn flatten_inline<'a>(
    b: &'a LayoutBox,
    styles: &StyleMap,
    font_size_px: f64,
    font_size_raw: Option<&str>,
    root_font_size_px: f64,
    style_node: Option<NodeId>,
    out: &mut Vec<InlineItem<'a>>,
) {
    match &b.kind {
        BoxKind::Text(text) => {
            for word in text.split_whitespace() {
                out.push(InlineItem::Word {
                    text_node: b.node,
                    style_node,
                    text: word.to_string(),
                    font_size_px,
                });
            }
        }
        BoxKind::Container(children) => {
            if b.level == BoxLevel::InlineBlock {
                out.push(InlineItem::Atomic(
                    b,
                    font_size_px,
                    font_size_raw.map(str::to_string),
                ));
            } else {
                let (own_font_size, own_font_size_raw) = resolve_font_size(
                    b.node,
                    styles,
                    font_size_px,
                    font_size_raw,
                    root_font_size_px,
                );
                let child_style_node = b.node.or(style_node);
                for child in children {
                    flatten_inline(
                        child,
                        styles,
                        own_font_size,
                        own_font_size_raw.as_deref(),
                        root_font_size_px,
                        child_style_node,
                        out,
                    );
                }
            }
        }
        // A flex/grid/table box directly inside an inline formatting
        // context is an unusual edge case (B1's `flex_items`/table
        // dispatch always produce block-level boxes, so this only
        // happens if one somehow ends up as `display: inline`-adjacent
        // content) -- drop it rather than trying to flatten table/flex/
        // grid-internal structure into words, a documented gap.
        BoxKind::FlexContainer(_)
        | BoxKind::GridContainer(_)
        | BoxKind::Table(_)
        | BoxKind::TableRow(_)
        | BoxKind::TableCell { .. } => {}
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
    font_size_raw: Option<&str>,
    root_font_size_px: f64,
    container_node: Option<NodeId>,
) -> (Vec<Fragment>, f64) {
    let mut items = Vec::new();
    for child in children {
        flatten_inline(
            child,
            styles,
            font_size_px,
            font_size_raw,
            root_font_size_px,
            container_node,
            &mut items,
        );
    }

    let mut lines: Vec<Vec<Fragment>> = vec![Vec::new()];
    let mut line_widths: Vec<f64> = vec![0.0];
    let mut line_max_font: Vec<f64> = vec![font_size_px];
    let default_line_height = line_height_px(None, font_size_px, root_font_size_px);
    let mut line_max_height: Vec<f64> = vec![default_line_height];

    for item in items {
        let (mut fragment, width, item_font_size, item_line_height) = match item {
            InlineItem::Word {
                text_node,
                style_node,
                text,
                font_size_px,
            } => {
                let w = word_width(&text, font_size_px);
                let style = style_node.and_then(|n| styles.get(&n));
                let lh = line_height_px(style, font_size_px, root_font_size_px);
                (
                    Fragment {
                        node: text_node,
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
                    lh,
                )
            }
            InlineItem::Atomic(b, inherited_font_size, inherited_font_size_raw) => {
                let f = layout_box(
                    b,
                    styles,
                    containing_width,
                    inherited_font_size,
                    inherited_font_size_raw.as_deref(),
                    root_font_size_px,
                );
                let w = f.margin_box().width;
                let style = b.node.and_then(|n| styles.get(&n));
                let lh = line_height_px(style, inherited_font_size, root_font_size_px);
                (f, w, inherited_font_size, lh)
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
            line_max_height.push(default_line_height);
            let current = lines.len() - 1;
            reposition(&mut fragment, 0.0, 0.0);
            line_widths[current] = width;
            line_max_font[current] = line_max_font[current].max(item_font_size);
            line_max_height[current] = line_max_height[current].max(item_line_height);
            lines[current].push(fragment);
        } else {
            let x = line_widths[current] + extra;
            reposition(&mut fragment, x, 0.0);
            line_widths[current] = x + width;
            line_max_font[current] = line_max_font[current].max(item_font_size);
            line_max_height[current] = line_max_height[current].max(item_line_height);
            lines[current].push(fragment);
        }
    }

    let mut out = Vec::new();
    let mut cursor_y = 0.0;
    for (i, mut line) in lines.into_iter().enumerate() {
        if line.is_empty() {
            continue;
        }
        let height = line_max_height[i];
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

    #[test]
    fn inherited_percentage_font_size_does_not_compound_across_generations() {
        let mut doc = Document::new();
        let root = doc.root();
        let outer = el(&mut doc, root, "div", &[]);
        let middle = el(&mut doc, outer, "div", &[]);
        let inner = el(&mut doc, middle, "div", &[]);
        doc.append(inner, NodeData::Text("hi".into()));

        // Simulates real `css::cascade` behavior: an inherited property
        // with no matching rule copies the parent's *computed* string
        // through verbatim rather than resolving it (see
        // `PROPERTY_TABLE`'s `inherited_value()` in `css::cascade`), so
        // `middle`/`inner` -- which have no `font-size` rule of their own
        // -- end up with the exact same `"150%"` string as `outer`, not a
        // resolved px value.
        let mut styles = StyleMap::new();
        for id in [outer, middle, inner] {
            styles.insert(
                id,
                [("display", "block"), ("font-size", "150%")]
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            );
        }

        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 500.0);
        // outer -> middle -> inner -> (line box) -> "hi" word fragment.
        let word = &fragment.children[0].children[0].children[0].children[0];
        // 16px default * 1.5 = 24px, resolved once against the root's
        // default size and then correctly *inherited* unchanged, not
        // recompounded to 16*1.5*1.5*1.5 = 54px.
        assert_eq!(word.content_rect.height, 24.0);
    }

    #[test]
    fn explicit_line_height_is_not_ignored() {
        let mut doc = Document::new();
        let root = doc.root();
        let p = el(&mut doc, root, "p", &[]);
        doc.append(p, NodeData::Text("hi".into()));
        let mut styles = StyleMap::new();
        styles.insert(
            p,
            [("display", "block"), ("line-height", "3")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        );
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 500.0);
        let line = &fragment.children[0];
        // Default font-size is 16px, so `line-height: 3` should produce a
        // 48px line box, not the `normal` (1.2x) 19.2px default.
        assert_eq!(line.content_rect.height, 48.0);
    }

    fn build_table(
        doc: &mut Document,
        styles: &mut StyleMap,
        cols_per_row: &[usize],
    ) -> Vec<NodeId> {
        let root = doc.root();
        let table = el(doc, root, "table", &[]);
        set_style(styles, table, &[("display", "table")]);
        for &n in cols_per_row {
            let row = el(doc, table, "tr", &[]);
            set_style(styles, row, &[("display", "table-row")]);
            for _ in 0..n {
                let cell = el(doc, row, "td", &[]);
                set_style(styles, cell, &[("display", "table-cell")]);
            }
        }
        vec![table]
    }

    #[test]
    fn table_columns_split_width_equally_with_no_hints() {
        let mut doc = Document::new();
        let mut styles = StyleMap::new();
        build_table(&mut doc, &mut styles, &[2]);
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 400.0);
        let row = &fragment.children[0];
        assert_eq!(row.children[0].border_box().width, 200.0);
        assert_eq!(row.children[1].border_box().x, 200.0);
        assert_eq!(row.children[1].border_box().width, 200.0);
    }

    #[test]
    fn table_explicit_cell_width_hints_its_column() {
        let mut doc = Document::new();
        let root = doc.root();
        let table = el(&mut doc, root, "table", &[]);
        let row = el(&mut doc, table, "tr", &[]);
        let a = el(&mut doc, row, "td", &[]);
        let b = el(&mut doc, row, "td", &[]);
        let mut styles = StyleMap::new();
        set_style(&mut styles, table, &[("display", "table")]);
        set_style(&mut styles, row, &[("display", "table-row")]);
        set_style(
            &mut styles,
            a,
            &[("display", "table-cell"), ("width", "100px")],
        );
        set_style(&mut styles, b, &[("display", "table-cell")]);
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 400.0);
        let row_fragment = &fragment.children[0];
        assert_eq!(row_fragment.children[0].border_box().width, 100.0);
        // The remaining 300px goes entirely to the one unhinted column.
        assert_eq!(row_fragment.children[1].border_box().width, 300.0);
        assert_eq!(row_fragment.children[1].border_box().x, 100.0);
    }

    #[test]
    fn table_colspan_spans_multiple_columns_and_rows_stack() {
        let mut doc = Document::new();
        let mut styles = StyleMap::new();
        let root = doc.root();
        let table = el(&mut doc, root, "table", &[]);
        set_style(&mut styles, table, &[("display", "table")]);
        let row1 = el(&mut doc, table, "tr", &[]);
        set_style(&mut styles, row1, &[("display", "table-row")]);
        let spanning = el(&mut doc, row1, "td", &[("colspan", "2")]);
        set_style(&mut styles, spanning, &[("display", "table-cell")]);
        let solo = el(&mut doc, row1, "td", &[]);
        set_style(&mut styles, solo, &[("display", "table-cell")]);
        let row2 = el(&mut doc, table, "tr", &[]);
        set_style(&mut styles, row2, &[("display", "table-row")]);
        for _ in 0..3 {
            let c = el(&mut doc, row2, "td", &[]);
            set_style(&mut styles, c, &[("display", "table-cell")]);
        }

        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 300.0);
        let row1_fragment = &fragment.children[0];
        // 3 equal columns of 100px each; the colspan=2 cell spans the
        // first two.
        assert_eq!(row1_fragment.children[0].border_box().width, 200.0);
        assert_eq!(row1_fragment.children[1].border_box().x, 200.0);
        let row2_fragment = &fragment.children[1];
        assert_eq!(
            row2_fragment.border_box().y,
            row1_fragment.border_box().height
        );
    }

    #[test]
    fn flex_row_grow_distributes_free_space_by_weight() {
        let mut doc = Document::new();
        let root = doc.root();
        let container = el(&mut doc, root, "div", &[]);
        let a = el(&mut doc, container, "div", &[]);
        let b = el(&mut doc, container, "div", &[]);
        let mut styles = StyleMap::new();
        set_style(&mut styles, container, &[("display", "flex")]);
        set_style(
            &mut styles,
            a,
            &[
                ("display", "block"),
                ("flex-basis", "0"),
                ("flex-grow", "1"),
            ],
        );
        set_style(
            &mut styles,
            b,
            &[
                ("display", "block"),
                ("flex-basis", "0"),
                ("flex-grow", "3"),
            ],
        );
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 400.0);
        // 400px free space split 1:3 -> 100px and 300px.
        assert_eq!(fragment.children[0].content_rect.width, 100.0);
        assert_eq!(fragment.children[1].content_rect.width, 300.0);
        assert_eq!(fragment.children[1].content_rect.x, 100.0);
    }

    #[test]
    fn flex_row_shrink_distributes_overflow_by_weight() {
        let mut doc = Document::new();
        let root = doc.root();
        let container = el(&mut doc, root, "div", &[]);
        let a = el(&mut doc, container, "div", &[]);
        let b = el(&mut doc, container, "div", &[]);
        let mut styles = StyleMap::new();
        set_style(&mut styles, container, &[("display", "flex")]);
        set_style(&mut styles, a, &[("display", "block"), ("width", "80px")]);
        set_style(&mut styles, b, &[("display", "block"), ("width", "80px")]);
        let tree = build_box_tree(&doc, &styles).unwrap();
        // 160px of basis crammed into 100px -- equal basis and default
        // flex-shrink:1 means the 60px overflow splits evenly, 30px each.
        let fragment = layout(&tree, &styles, 100.0);
        assert_eq!(fragment.children[0].content_rect.width, 50.0);
        assert_eq!(fragment.children[1].content_rect.width, 50.0);
    }

    #[test]
    fn flex_justify_content_center_centers_the_line() {
        let mut doc = Document::new();
        let root = doc.root();
        let container = el(&mut doc, root, "div", &[]);
        let a = el(&mut doc, container, "div", &[]);
        let b = el(&mut doc, container, "div", &[]);
        let mut styles = StyleMap::new();
        set_style(
            &mut styles,
            container,
            &[("display", "flex"), ("justify-content", "center")],
        );
        set_style(&mut styles, a, &[("display", "block"), ("width", "50px")]);
        set_style(&mut styles, b, &[("display", "block"), ("width", "50px")]);
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 300.0);
        // Used main size = 100px, free = 200px, centered -> 100px offset.
        assert_eq!(fragment.children[0].content_rect.x, 100.0);
        assert_eq!(fragment.children[1].content_rect.x, 150.0);
    }

    #[test]
    fn flex_wrap_starts_a_new_line_when_items_overflow() {
        let mut doc = Document::new();
        let root = doc.root();
        let container = el(&mut doc, root, "div", &[]);
        let a = el(&mut doc, container, "div", &[]);
        let b = el(&mut doc, container, "div", &[]);
        let mut styles = StyleMap::new();
        set_style(
            &mut styles,
            container,
            &[("display", "flex"), ("flex-wrap", "wrap")],
        );
        set_style(
            &mut styles,
            a,
            &[("display", "block"), ("width", "80px"), ("height", "20px")],
        );
        set_style(
            &mut styles,
            b,
            &[("display", "block"), ("width", "80px"), ("height", "30px")],
        );
        let tree = build_box_tree(&doc, &styles).unwrap();
        // Container is 100px wide -- two 80px items can't share a line.
        let fragment = layout(&tree, &styles, 100.0);
        assert_eq!(fragment.children[0].content_rect.y, 0.0);
        // Second line starts below the first line's cross size (20px).
        assert_eq!(fragment.children[1].content_rect.y, 20.0);
    }

    #[test]
    fn flex_column_direction_stacks_items_vertically() {
        let mut doc = Document::new();
        let root = doc.root();
        let container = el(&mut doc, root, "div", &[]);
        let a = el(&mut doc, container, "div", &[]);
        let b = el(&mut doc, container, "div", &[]);
        let mut styles = StyleMap::new();
        set_style(
            &mut styles,
            container,
            &[("display", "flex"), ("flex-direction", "column")],
        );
        set_style(&mut styles, a, &[("display", "block"), ("height", "50px")]);
        set_style(&mut styles, b, &[("display", "block"), ("height", "30px")]);
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 200.0);
        assert_eq!(fragment.children[0].content_rect.y, 0.0);
        assert_eq!(fragment.children[1].content_rect.y, 50.0);
        // Container's own auto height should be the sum of both items.
        assert_eq!(fragment.content_rect.height, 80.0);
    }

    #[test]
    fn flex_align_items_stretch_fills_cross_size() {
        let mut doc = Document::new();
        let root = doc.root();
        let container = el(&mut doc, root, "div", &[]);
        let a = el(&mut doc, container, "div", &[]);
        let b = el(&mut doc, container, "div", &[]);
        let mut styles = StyleMap::new();
        set_style(&mut styles, container, &[("display", "flex")]);
        set_style(
            &mut styles,
            a,
            &[("display", "block"), ("width", "50px"), ("height", "10px")],
        );
        set_style(&mut styles, b, &[("display", "block"), ("width", "50px")]);
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 200.0);
        // `b` has no explicit height (auto) and default align-items is
        // stretch, so it should stretch to the line's cross size -- the
        // tallest item, `a`'s explicit 10px.
        assert_eq!(fragment.children[1].content_rect.height, 10.0);
    }

    #[test]
    fn grid_columns_split_by_fr_weight_and_wrap_to_new_row() {
        let mut doc = Document::new();
        let root = doc.root();
        let container = el(&mut doc, root, "div", &[]);
        let a = el(&mut doc, container, "div", &[]);
        let b = el(&mut doc, container, "div", &[]);
        let c = el(&mut doc, container, "div", &[]);
        let mut styles = StyleMap::new();
        set_style(
            &mut styles,
            container,
            &[("display", "grid"), ("grid-template-columns", "1fr 2fr")],
        );
        set_style(&mut styles, a, &[("display", "block"), ("height", "10px")]);
        set_style(&mut styles, b, &[("display", "block"), ("height", "10px")]);
        set_style(&mut styles, c, &[("display", "block"), ("height", "10px")]);
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 300.0);
        // 300px split 1:2 -> 100px, 200px columns.
        assert_eq!(fragment.children[0].content_rect.width, 100.0);
        assert_eq!(fragment.children[1].content_rect.x, 100.0);
        assert_eq!(fragment.children[1].content_rect.width, 200.0);
        // Only 2 columns -- the third item wraps to a new row.
        assert_eq!(fragment.children[2].content_rect.x, 0.0);
        assert_eq!(fragment.children[2].content_rect.y, 10.0);
    }

    #[test]
    fn grid_explicit_column_placement_leaves_earlier_cells_for_auto_items() {
        let mut doc = Document::new();
        let root = doc.root();
        let container = el(&mut doc, root, "div", &[]);
        let a = el(&mut doc, container, "div", &[]);
        let b = el(&mut doc, container, "div", &[]);
        let mut styles = StyleMap::new();
        set_style(
            &mut styles,
            container,
            &[
                ("display", "grid"),
                ("grid-template-columns", "100px 100px 100px"),
            ],
        );
        // `a` explicitly claims column 2 (1-based) = 0-based column 1.
        set_style(
            &mut styles,
            a,
            &[("display", "block"), ("grid-column", "2")],
        );
        // `b` is auto-placed and should land in the still-free column 0.
        set_style(&mut styles, b, &[("display", "block")]);
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 300.0);
        assert_eq!(fragment.children[0].content_rect.x, 100.0);
        assert_eq!(fragment.children[1].content_rect.x, 0.0);
    }

    #[test]
    fn grid_row_height_comes_from_tallest_item_in_that_row() {
        let mut doc = Document::new();
        let root = doc.root();
        let container = el(&mut doc, root, "div", &[]);
        let a = el(&mut doc, container, "div", &[]);
        let b = el(&mut doc, container, "div", &[]);
        let c = el(&mut doc, container, "div", &[]);
        let mut styles = StyleMap::new();
        set_style(
            &mut styles,
            container,
            &[("display", "grid"), ("grid-template-columns", "50px 50px")],
        );
        set_style(&mut styles, a, &[("display", "block"), ("height", "30px")]);
        set_style(&mut styles, b, &[("display", "block"), ("height", "10px")]);
        set_style(&mut styles, c, &[("display", "block"), ("height", "5px")]);
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 100.0);
        // Row 0's height is 30px (the tallest of a/b), so row 1's `c`
        // starts at y=30.
        assert_eq!(fragment.children[2].content_rect.y, 30.0);
        assert_eq!(fragment.content_rect.height, 35.0);
    }

    #[test]
    fn grid_explicit_row_track_overrides_content_height() {
        let mut doc = Document::new();
        let root = doc.root();
        let container = el(&mut doc, root, "div", &[]);
        let a = el(&mut doc, container, "div", &[]);
        let mut styles = StyleMap::new();
        set_style(
            &mut styles,
            container,
            &[
                ("display", "grid"),
                ("grid-template-columns", "100px"),
                ("grid-template-rows", "50px"),
            ],
        );
        set_style(&mut styles, a, &[("display", "block"), ("height", "5px")]);
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 100.0);
        assert_eq!(fragment.content_rect.height, 50.0);
    }

    #[test]
    fn position_relative_offsets_without_disturbing_siblings() {
        let mut doc = Document::new();
        let root = doc.root();
        let parent = el(&mut doc, root, "div", &[]);
        let a = el(&mut doc, parent, "div", &[]);
        let b = el(&mut doc, parent, "div", &[]);
        let mut styles = StyleMap::new();
        set_style(&mut styles, parent, &[("display", "block")]);
        set_style(
            &mut styles,
            a,
            &[
                ("display", "block"),
                ("height", "10px"),
                ("position", "relative"),
                ("top", "5px"),
                ("left", "3px"),
            ],
        );
        set_style(&mut styles, b, &[("display", "block"), ("height", "10px")]);
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 100.0);
        // `a` visually shifts by (3, 5)...
        assert_eq!(fragment.children[0].content_rect.x, 3.0);
        assert_eq!(fragment.children[0].content_rect.y, 5.0);
        // ...but `b` is positioned exactly where it would be if `a` had
        // never moved (still-in-flow, so it stacks below `a`'s original
        // 10px-tall box, not the offset one).
        assert_eq!(fragment.children[1].content_rect.y, 10.0);
    }

    #[test]
    fn position_absolute_is_out_of_flow_and_positioned_by_offsets() {
        let mut doc = Document::new();
        let root = doc.root();
        let parent = el(&mut doc, root, "div", &[]);
        let a = el(&mut doc, parent, "div", &[]);
        let b = el(&mut doc, parent, "div", &[]);
        let mut styles = StyleMap::new();
        set_style(&mut styles, parent, &[("display", "block")]);
        set_style(
            &mut styles,
            a,
            &[
                ("display", "block"),
                ("height", "10px"),
                ("position", "absolute"),
                ("top", "40px"),
                ("left", "20px"),
            ],
        );
        set_style(&mut styles, b, &[("display", "block"), ("height", "10px")]);
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 100.0);
        assert_eq!(fragment.children[0].content_rect.x, 20.0);
        assert_eq!(fragment.children[0].content_rect.y, 40.0);
        // `b` doesn't see `a` at all -- it's the first in-flow box.
        assert_eq!(fragment.children[1].content_rect.y, 0.0);
    }
}
