//! B9: fragment-tree query utilities -- the real architectural point of
//! keeping a fragment tree around instead of a toy "list of boxes":
//! `getBoundingClientRect()`/`elementFromPoint()`-equivalents implemented
//! purely by reading it, with no re-derivation of geometry.
//!
//! Every `Fragment::content_rect` already holds a genuinely **absolute**
//! position by the time `flow::layout()` returns the tree: each
//! formatting-context function positions a child via `reposition()`,
//! which shifts that child *and its entire already-built subtree*
//! together (see `flow.rs`'s `shift()`) -- so by construction, once a
//! fragment is placed by its parent, every coordinate in it and beneath
//! it is already correct in the whole tree's shared coordinate space.
//! Both functions here can therefore just read `content_rect`/
//! `border_box()` directly, with no per-level offset accumulation.
//!
//! Known gap: [`element_from_point`] picks the deepest fragment under the
//! point, breaking ties between overlapping siblings by later-in-tree-
//! order-wins (`float`/absolutely-positioned boxes are appended after
//! their in-flow siblings in `layout_block_children`'s output, so this
//! approximates real paint order reasonably for the common case) -- but
//! there's no real stacking-context/`z-index` paint-order model yet
//! (B6's own documented gap), so this isn't spec-correct for content
//! that actually relies on `z-index`.

use crate::flow::{Fragment, Rect};
use dom::NodeId;

/// <https://developer.mozilla.org/en-US/docs/Web/API/Element/getBoundingClientRect>:
/// `node`'s border-box rect. `None` if `node` has no fragment in this
/// tree (e.g. `display: none`, or a node this tree simply doesn't
/// contain).
pub fn bounding_client_rect(root: &Fragment, node: NodeId) -> Option<Rect> {
    if root.node == Some(node) {
        return Some(root.border_box());
    }
    root.children
        .iter()
        .find_map(|child| bounding_client_rect(child, node))
}

/// <https://developer.mozilla.org/en-US/docs/Web/API/Document/elementFromPoint>:
/// the deepest fragment whose border-box contains `(x, y)`, walking up to
/// its nearest real (non-anonymous) ancestor if it's itself anonymous (a
/// line box, an anonymous block wrapper, ...). `None` if the point isn't
/// over any fragment at all.
pub fn element_from_point(root: &Fragment, x: f64, y: f64) -> Option<NodeId> {
    let border_box = root.border_box();
    if x < border_box.x
        || x > border_box.x + border_box.width
        || y < border_box.y
        || y > border_box.y + border_box.height
    {
        return None;
    }
    // Later children paint over earlier ones absent real stacking-context
    // support -- see module docs -- so check in reverse.
    root.children
        .iter()
        .rev()
        .find_map(|child| element_from_point(child, x, y))
        .or(root.node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::box_tree::{StyleMap, build_box_tree};
    use crate::flow::layout;
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
    fn bounding_client_rect_accumulates_nested_offsets() {
        let mut doc = Document::new();
        let root = doc.root();
        let outer = el(&mut doc, root, "div", &[]);
        let inner = el(&mut doc, outer, "div", &[]);
        let mut styles = StyleMap::new();
        set_style(
            &mut styles,
            outer,
            &[
                ("display", "block"),
                ("padding-left", "10px"),
                ("padding-top", "5px"),
            ],
        );
        set_style(
            &mut styles,
            inner,
            &[
                ("display", "block"),
                ("margin-left", "3px"),
                ("width", "20px"),
                ("height", "8px"),
            ],
        );
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 100.0);
        let rect = bounding_client_rect(&fragment, inner).unwrap();
        // outer's padding pushes inner's content box right/down by
        // (10, 5); inner's own margin-left adds 3 more.
        assert_eq!(rect.x, 13.0);
        assert_eq!(rect.y, 5.0);
        assert_eq!(rect.width, 20.0);
        assert_eq!(rect.height, 8.0);
    }

    #[test]
    fn bounding_client_rect_returns_none_for_a_node_not_in_the_tree() {
        let mut doc = Document::new();
        let root = doc.root();
        let a = el(&mut doc, root, "div", &[]);
        let stray = el(&mut doc, root, "div", &[]);
        let mut styles = StyleMap::new();
        set_style(&mut styles, a, &[("display", "block"), ("height", "5px")]);
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 100.0);
        assert!(bounding_client_rect(&fragment, stray).is_none());
    }

    #[test]
    fn element_from_point_finds_the_deepest_matching_fragment() {
        let mut doc = Document::new();
        let root = doc.root();
        let outer = el(&mut doc, root, "div", &[]);
        let inner = el(&mut doc, outer, "div", &[]);
        let mut styles = StyleMap::new();
        set_style(&mut styles, outer, &[("display", "block")]);
        set_style(
            &mut styles,
            inner,
            &[("display", "block"), ("width", "20px"), ("height", "20px")],
        );
        let tree = build_box_tree(&doc, &styles).unwrap();
        let fragment = layout(&tree, &styles, 100.0);
        // A point inside `inner`'s box should resolve to `inner`, not
        // `outer`, even though both fragments' boxes contain it.
        assert_eq!(element_from_point(&fragment, 5.0, 5.0), Some(inner));
        // A point outside both boxes entirely.
        assert_eq!(element_from_point(&fragment, 500.0, 500.0), None);
    }
}
