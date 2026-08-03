//! B1: style tree -> box tree (`display` computation and anonymous
//! block-box generation).
//!
//! Reference: <https://www.w3.org/TR/css-display-3/>,
//! <https://www.w3.org/TR/CSS22/visuren.html#anonymous-block-level> (the
//! "anonymous block boxes" rule this module's [`wrap_anonymous_blocks`]
//! implements: when a block container has a mix of block-level and
//! inline-level children, every maximal run of inline-level children gets
//! wrapped in a synthetic block box so the container's children are either
//! *all* block-level or *all* inline-level -- never mixed).
//!
//! Known gaps, documented up front rather than silently assumed away:
//! - **No `::before`/`::after` generated content.** That needs
//!   pseudo-element matching support in `css::cascade` (a `p::before`
//!   selector's `PseudoElement` subclass currently always fails to match
//!   any real DOM node -- see `css::selectors`' module docs), which
//!   doesn't exist yet. Tracked as follow-up work, not silently dropped.
//! - **`display: table`/`table-row`/`table-cell`/`flex`/`grid`/etc all
//!   collapse to plain block-level.** B3/B4/B5's own internal box
//!   generation isn't built yet; this is a real, harmless fallback (the
//!   box tree still generates something sane to lay out), not a crash,
//!   but it isn't the real spec box type either.
//! - **List markers are simplified**: only `disc`/`circle`/`square`/
//!   `decimal` `list-style-type` keywords are recognized (anything else
//!   falls back to a disc-style bullet); markers are always rendered
//!   "inside" the principal box's own inline content rather than in the
//!   margin the way `list-style-position: outside` (the initial value)
//!   actually requires; and numbering only counts same-parent
//!   `display: list-item` siblings -- no nested-list counter scoping, and
//!   no `<ol start>`/`<li value>` support.
//! - **Whitespace collapsing is minimal**: an entirely-whitespace text
//!   node produces no box at all, but non-whitespace text isn't otherwise
//!   collapsed (runs of internal spaces, leading/trailing whitespace) the
//!   way <https://www.w3.org/TR/css-text-3/#white-space-phase-1> actually
//!   specifies.
//! - **Block-level content nested inside an inline box** (e.g. a `<div>`
//!   inside a `<span>`, which real HTML parsing mostly prevents but CSS
//!   alone doesn't forbid) isn't split/reparented the way the spec
//!   describes for that edge case -- it's just laid out as an ordinary
//!   child of the inline box, a known simplification.

use dom::{Document, NodeData, NodeId};
use std::collections::HashMap;

pub type ComputedStyle = HashMap<String, String>;
pub type StyleMap = HashMap<NodeId, ComputedStyle>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Display {
    Block,
    Inline,
    InlineBlock,
    ListItem,
    None,
    Contents,
}

pub fn display_of(style: Option<&ComputedStyle>) -> Display {
    let value = style
        .and_then(|s| s.get("display"))
        .map(String::as_str)
        .unwrap_or("inline");
    match value {
        "none" => Display::None,
        "contents" => Display::Contents,
        "inline" => Display::Inline,
        "inline-block" => Display::InlineBlock,
        "list-item" => Display::ListItem,
        // See module docs' "table/flex/grid" known gap.
        _ => Display::Block,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxLevel {
    Block,
    Inline,
    InlineBlock,
}

#[derive(Debug, Clone)]
pub enum BoxKind {
    Container(Vec<LayoutBox>),
    Text(String),
}

/// A single box in the box tree. `node: None` marks an anonymous box (a
/// synthetic wrapper or a list marker) that doesn't correspond to any real
/// DOM node.
#[derive(Debug, Clone)]
pub struct LayoutBox {
    pub node: Option<NodeId>,
    pub level: BoxLevel,
    pub kind: BoxKind,
}

/// Builds the box tree for `doc`'s single root element (or `None` if the
/// document is empty or its root element is `display: none`).
pub fn build_box_tree(doc: &Document, styles: &StyleMap) -> Option<LayoutBox> {
    let mut out = Vec::new();
    for &child in doc.children(doc.root()) {
        append_child_boxes(doc, styles, child, &mut out);
    }
    out.into_iter().next()
}

fn build_children(doc: &Document, styles: &StyleMap, parent: NodeId) -> Vec<LayoutBox> {
    let mut out = Vec::new();
    for &child in doc.children(parent) {
        append_child_boxes(doc, styles, child, &mut out);
    }
    out
}

fn append_child_boxes(doc: &Document, styles: &StyleMap, node: NodeId, out: &mut Vec<LayoutBox>) {
    match doc.data(node) {
        NodeData::Text(text) => {
            if !text.trim().is_empty() {
                out.push(LayoutBox {
                    node: Some(node),
                    level: BoxLevel::Inline,
                    kind: BoxKind::Text(text.clone()),
                });
            }
        }
        NodeData::Element(_) => {
            let display = display_of(styles.get(&node));
            match display {
                Display::None => {}
                Display::Contents => {
                    for &child in doc.children(node) {
                        append_child_boxes(doc, styles, child, out);
                    }
                }
                Display::Inline => {
                    let children = build_children(doc, styles, node);
                    out.push(LayoutBox {
                        node: Some(node),
                        level: BoxLevel::Inline,
                        kind: BoxKind::Container(children),
                    });
                }
                Display::InlineBlock => {
                    let children = wrap_anonymous_blocks(build_children(doc, styles, node));
                    out.push(LayoutBox {
                        node: Some(node),
                        level: BoxLevel::InlineBlock,
                        kind: BoxKind::Container(children),
                    });
                }
                Display::Block => {
                    let children = wrap_anonymous_blocks(build_children(doc, styles, node));
                    out.push(LayoutBox {
                        node: Some(node),
                        level: BoxLevel::Block,
                        kind: BoxKind::Container(children),
                    });
                }
                Display::ListItem => {
                    let mut all = Vec::new();
                    if let Some(marker) =
                        marker_text(styles.get(&node), list_item_index(doc, styles, node))
                    {
                        all.push(LayoutBox {
                            node: None,
                            level: BoxLevel::Inline,
                            kind: BoxKind::Text(marker),
                        });
                    }
                    all.extend(build_children(doc, styles, node));
                    out.push(LayoutBox {
                        node: Some(node),
                        level: BoxLevel::Block,
                        kind: BoxKind::Container(wrap_anonymous_blocks(all)),
                    });
                }
            }
        }
        _ => {}
    }
}

/// <https://www.w3.org/TR/CSS22/visuren.html#anonymous-block-level>: if
/// `children` has no block-level box at all, it's left untouched (the
/// parent establishes an inline formatting context directly, the common
/// case); otherwise every maximal run of inline-level children is wrapped
/// in its own anonymous block box so every child ends up block-level.
fn wrap_anonymous_blocks(children: Vec<LayoutBox>) -> Vec<LayoutBox> {
    if !children.iter().any(|c| c.level == BoxLevel::Block) {
        return children;
    }
    let mut out = Vec::new();
    let mut run = Vec::new();
    for child in children {
        if child.level == BoxLevel::Block {
            if !run.is_empty() {
                out.push(anonymous_block(std::mem::take(&mut run)));
            }
            out.push(child);
        } else {
            run.push(child);
        }
    }
    if !run.is_empty() {
        out.push(anonymous_block(run));
    }
    out
}

fn anonymous_block(children: Vec<LayoutBox>) -> LayoutBox {
    LayoutBox {
        node: None,
        level: BoxLevel::Block,
        kind: BoxKind::Container(children),
    }
}

/// 1-based position of `node` among its parent's `display: list-item`
/// element siblings (see module docs' list-marker known gap).
fn list_item_index(doc: &Document, styles: &StyleMap, node: NodeId) -> usize {
    let Some(parent) = doc.parent(node) else {
        return 1;
    };
    let mut count = 0;
    for &sibling in doc.children(parent) {
        if sibling == node {
            break;
        }
        if matches!(doc.data(sibling), NodeData::Element(_))
            && display_of(styles.get(&sibling)) == Display::ListItem
        {
            count += 1;
        }
    }
    count + 1
}

fn marker_text(style: Option<&ComputedStyle>, index: usize) -> Option<String> {
    let kind = style
        .and_then(|s| s.get("list-style-type"))
        .map(String::as_str)
        .unwrap_or("disc");
    match kind {
        "none" => None,
        "circle" => Some("◦ ".to_string()),
        "square" => Some("▪ ".to_string()),
        "decimal" => Some(format!("{index}. ")),
        _ => Some("• ".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styles_with(pairs: &[(NodeId, &[(&str, &str)])]) -> StyleMap {
        pairs
            .iter()
            .map(|(id, props)| {
                let map: ComputedStyle = props
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();
                (*id, map)
            })
            .collect()
    }

    fn find_by_local_name(doc: &Document, name: &str) -> NodeId {
        let mut found = None;
        doc.walk(doc.root(), &mut |id, _| {
            if found.is_none()
                && let NodeData::Element(e) = doc.data(id)
                && e.local_name == name
            {
                found = Some(id);
            }
        });
        found.expect("element not found")
    }

    /// Finds the box for `node` within `root`'s subtree -- needed because
    /// HTML tree construction inserts an implied `<head>` before `<body>`,
    /// so `<body>` isn't reliably `<html>`'s first child.
    fn find_box(root: &LayoutBox, node: NodeId) -> &LayoutBox {
        if root.node == Some(node) {
            return root;
        }
        if let BoxKind::Container(children) = &root.kind {
            for child in children {
                if let Some(found) = try_find_box(child, node) {
                    return found;
                }
            }
        }
        panic!("box for node not found");
    }

    fn try_find_box(b: &LayoutBox, node: NodeId) -> Option<&LayoutBox> {
        if b.node == Some(node) {
            return Some(b);
        }
        if let BoxKind::Container(children) = &b.kind {
            for child in children {
                if let Some(found) = try_find_box(child, node) {
                    return Some(found);
                }
            }
        }
        None
    }

    #[test]
    fn display_none_produces_no_box() {
        let doc = html::parse_document("<html><body><div id=\"x\"></div></body></html>");
        let div = find_by_local_name(&doc, "div");
        let body = find_by_local_name(&doc, "body");
        let styles = styles_with(&[(div, &[("display", "none")])]);
        let root = build_box_tree(&doc, &styles).unwrap();
        let body_box = find_box(&root, body);
        let BoxKind::Container(body_children) = &body_box.kind else {
            panic!("expected container");
        };
        assert!(body_children.is_empty());
    }

    #[test]
    fn mixed_block_and_inline_children_get_anonymous_wrapper() {
        let doc = html::parse_document(
            "<html><body><div id=\"d\">text <b>bold</b><p>block</p></div></body></html>",
        );
        let div = find_by_local_name(&doc, "div");
        let p = find_by_local_name(&doc, "p");
        // No UA-default stylesheet is applied by this test, so <div>/<p>
        // need an explicit `display: block` to behave like a real UA's
        // default (this crate's own fallback default is `inline`, CSS's
        // real initial value, correct for an element no rule matches).
        let styles = styles_with(&[(div, &[("display", "block")]), (p, &[("display", "block")])]);
        let root = build_box_tree(&doc, &styles).unwrap();
        let div_box = find_box(&root, div);
        let BoxKind::Container(div_children) = &div_box.kind else {
            panic!()
        };
        // Expect: [anonymous block wrapping "text " + <b>bold</b>, <p>].
        assert_eq!(div_children.len(), 2);
        assert!(div_children[0].node.is_none());
        assert_eq!(div_children[0].level, BoxLevel::Block);
        assert_eq!(div_children[1].level, BoxLevel::Block);
        assert!(div_children[1].node.is_some());
    }

    #[test]
    fn pure_inline_children_are_not_wrapped() {
        let doc =
            html::parse_document("<html><body><p id=\"p\">hello <b>world</b></p></body></html>");
        let p = find_by_local_name(&doc, "p");
        let styles = StyleMap::new();
        let root = build_box_tree(&doc, &styles).unwrap();
        let p_box = find_box(&root, p);
        let BoxKind::Container(p_children) = &p_box.kind else {
            panic!()
        };
        // No anonymous wrapper: "hello " text run + <b> inline box directly.
        assert_eq!(p_children.len(), 2);
        assert_eq!(p_children[0].level, BoxLevel::Inline);
        assert!(matches!(p_children[0].kind, BoxKind::Text(_)));
        assert_eq!(p_children[1].level, BoxLevel::Inline);
        assert!(p_children[1].node.is_some());
    }

    #[test]
    fn list_item_markers_and_numbering() {
        let doc = html::parse_document(
            "<html><body><ul id=\"list\"><li id=\"a\">one</li><li id=\"b\">two</li></ul></body></html>",
        );
        let ul = find_by_local_name(&doc, "ul");
        let mut items = Vec::new();
        doc.walk(doc.root(), &mut |id, _| {
            if let NodeData::Element(e) = doc.data(id)
                && e.local_name == "li"
            {
                items.push(id);
            }
        });
        let styles = styles_with(&[
            (
                items[0],
                &[("display", "list-item"), ("list-style-type", "decimal")],
            ),
            (
                items[1],
                &[("display", "list-item"), ("list-style-type", "decimal")],
            ),
        ]);
        let root = build_box_tree(&doc, &styles).unwrap();
        let ul_box = find_box(&root, ul);
        let BoxKind::Container(ul_children) = &ul_box.kind else {
            panic!()
        };
        assert_eq!(ul_children.len(), 2);
        let BoxKind::Container(li1_children) = &ul_children[0].kind else {
            panic!()
        };
        let BoxKind::Text(marker) = &li1_children[0].kind else {
            panic!("expected marker text first")
        };
        assert_eq!(marker, "1. ");
        let BoxKind::Container(li2_children) = &ul_children[1].kind else {
            panic!()
        };
        let BoxKind::Text(marker2) = &li2_children[0].kind else {
            panic!("expected marker text first")
        };
        assert_eq!(marker2, "2. ");
    }

    #[test]
    fn display_contents_splices_children_into_parent() {
        let doc = html::parse_document(
            "<html><body><div id=\"c\"><span>a</span><span>b</span></div></body></html>",
        );
        let div = find_by_local_name(&doc, "div");
        let body = find_by_local_name(&doc, "body");
        let styles = styles_with(&[(div, &[("display", "contents")])]);
        let root = build_box_tree(&doc, &styles).unwrap();
        let body_box = find_box(&root, body);
        let BoxKind::Container(body_children) = &body_box.kind else {
            panic!()
        };
        // The <div> itself generates no box; its two <span> children splice
        // straight into <body>'s child list.
        assert_eq!(body_children.len(), 2);
        assert!(body_children.iter().all(|b| b.level == BoxLevel::Inline));
    }
}
