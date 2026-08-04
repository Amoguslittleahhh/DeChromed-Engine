//! C1: `Element`/`Document`'s selector-based query methods --
//! `querySelector`, `querySelectorAll`, `closest`, and `Element.matches`
//! -- real implementations built directly on A5's own `selectors::
//! matches`/`SelectorList`. Lives here rather than in `dom` (which C1's
//! other query methods live in, see that crate's `api.rs`) because `css`
//! already depends on `dom` for selector matching; putting selector-aware
//! queries in `dom` instead would need the dependency pointing the other
//! way, which would let `dom` know about CSS selector syntax at all --
//! exactly the layering `css::selectors`'s own existence already avoids.
//!
//! Reference: <https://dom.spec.whatwg.org/#interface-parentnode> (`query
//! SelectorAll`), <https://dom.spec.whatwg.org/#dom-element-closest>.

use crate::selectors::{SelectorList, matches, matches_complex};
use dom::{Document, NodeData, NodeId};

/// `Element.matches(selectors)` / `ParentNode.querySelector`'s own match
/// test, exposed directly: does `id` match any complex selector in
/// `list`? A thin, self-documenting name over `selectors::matches` for
/// callers arriving from the DOM-API side rather than the cascade side.
pub fn element_matches(doc: &Document, id: NodeId, list: &SelectorList) -> bool {
    matches(doc, id, list)
}

/// `ParentNode.querySelector`: the first descendant of `root` (real
/// pre-order tree traversal, `root` itself excluded, per spec) that
/// matches `list`, or `None`.
pub fn query_selector(doc: &Document, root: NodeId, list: &SelectorList) -> Option<NodeId> {
    for &child in doc.children(root) {
        if matches!(doc.data(child), NodeData::Element(_)) && matches(doc, child, list) {
            return Some(child);
        }
        if let Some(found) = query_selector(doc, child, list) {
            return Some(found);
        }
    }
    None
}

/// `ParentNode.querySelectorAll`: every descendant of `root` (`root`
/// itself excluded) that matches `list`, in tree order.
pub fn query_selector_all(doc: &Document, root: NodeId, list: &SelectorList) -> Vec<NodeId> {
    let mut out = Vec::new();
    collect_matches(doc, root, list, &mut out);
    out
}

fn collect_matches(doc: &Document, id: NodeId, list: &SelectorList, out: &mut Vec<NodeId>) {
    for &child in doc.children(id) {
        if matches!(doc.data(child), NodeData::Element(_)) && matches(doc, child, list) {
            out.push(child);
        }
        collect_matches(doc, child, list, out);
    }
}

/// `Element.closest(selectors)`: `id` itself if it matches, else the
/// nearest matching ancestor -- a real upward walk, stopping at the
/// document root (`closest` never matches the document node itself,
/// since it's not an element).
pub fn closest(doc: &Document, id: NodeId, list: &SelectorList) -> Option<NodeId> {
    let mut current = Some(id);
    while let Some(node) = current {
        if matches!(doc.data(node), NodeData::Element(_)) && matches_complex_any(doc, node, list) {
            return Some(node);
        }
        current = doc.parent(node);
    }
    None
}

fn matches_complex_any(doc: &Document, id: NodeId, list: &SelectorList) -> bool {
    list.0.iter().any(|cs| matches_complex(doc, id, cs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selectors::parse_selector_list;
    use dom::{Document, ElementData};

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
    fn query_selector_finds_the_first_match_in_tree_order() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[]);
        let p1 = el(&mut doc, div, "p", &[]);
        el(&mut doc, root, "p", &[]);
        let list = parse_selector_list("p").unwrap();
        assert_eq!(query_selector(&doc, root, &list), Some(p1));
    }

    #[test]
    fn query_selector_returns_none_with_no_match() {
        let mut doc = Document::new();
        let root = doc.root();
        el(&mut doc, root, "div", &[]);
        let list = parse_selector_list("span").unwrap();
        assert_eq!(query_selector(&doc, root, &list), None);
    }

    #[test]
    fn query_selector_all_finds_every_match_in_tree_order() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[("class", "x")]);
        let a = el(&mut doc, div, "p", &[("class", "x")]);
        let b = el(&mut doc, root, "p", &[("class", "x")]);
        el(&mut doc, root, "span", &[("class", "x")]);
        let list = parse_selector_list(".x").unwrap();
        assert_eq!(query_selector_all(&doc, root, &list).len(), 4);
        let list_p = parse_selector_list("p.x").unwrap();
        assert_eq!(query_selector_all(&doc, root, &list_p), vec![a, b]);
    }

    #[test]
    fn closest_returns_self_if_it_matches() {
        let mut doc = Document::new();
        let root = doc.root();
        let p = el(&mut doc, root, "p", &[("class", "target")]);
        let list = parse_selector_list(".target").unwrap();
        assert_eq!(closest(&doc, p, &list), Some(p));
    }

    #[test]
    fn closest_walks_up_to_the_nearest_matching_ancestor() {
        let mut doc = Document::new();
        let root = doc.root();
        let section = el(&mut doc, root, "section", &[("class", "target")]);
        let div = el(&mut doc, section, "div", &[]);
        let span = el(&mut doc, div, "span", &[]);
        let list = parse_selector_list(".target").unwrap();
        assert_eq!(closest(&doc, span, &list), Some(section));
    }

    #[test]
    fn closest_returns_none_when_nothing_up_the_chain_matches() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[]);
        let span = el(&mut doc, div, "span", &[]);
        let list = parse_selector_list(".nonexistent").unwrap();
        assert_eq!(closest(&doc, span, &list), None);
    }

    #[test]
    fn element_matches_wraps_selectors_matches() {
        let mut doc = Document::new();
        let root = doc.root();
        let p = el(&mut doc, root, "p", &[("class", "greeting")]);
        let list = parse_selector_list("p.greeting").unwrap();
        assert!(element_matches(&doc, p, &list));
        let other = parse_selector_list("span").unwrap();
        assert!(!element_matches(&doc, p, &other));
    }
}
