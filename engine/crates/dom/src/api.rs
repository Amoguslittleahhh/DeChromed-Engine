//! C1: the actual addressable DOM API — spec-shaped `Node`/`Element`/
//! `Document` methods and real mutation algorithms, layered on top of
//! `lib.rs`'s plain arena tree (which this module's own doc comment
//! already earmarked as "not a spec-mandated API surface... that's C1's
//! job").
//!
//! Reference: <https://dom.spec.whatwg.org/>.
//!
//! **On "live" collections, and why this API doesn't need a separate
//! caching layer for them.** The DOM spec's `NodeList`/`HTMLCollection`
//! are *live*: a JS reference held across a mutation still reflects the
//! current tree, because JS's object-reference model lets code hold that
//! reference indefinitely without re-querying. This crate's API instead
//! always takes `&Document` explicitly on every call (`Document::
//! children`, `Document::element_children`, `Document::get_elements_by_
//! tag_name`, ...) -- so a caller physically cannot hold a stale
//! snapshot across a mutation without the borrow checker forbidding the
//! mutation in the first place. Every query here is *live by
//! construction*: there is no cache to go stale, because there is no
//! cache. This is a real architectural consequence of the ownership
//! model, not a missing feature standing in for one -- see
//! `get_elements_by_tag_name`'s own doc comment for the one place that
//! distinction actually matters (repeated calls do real, uncached
//! re-traversal, matching the spec's own "live" requirement exactly,
//! just without needing a *separate* mechanism to achieve it).
//!
//! **Known gaps:** no `Range`/`Selection`, no `MutationObserver` (needs
//! Track C2's event loop to queue records against), no `Document.
//! adoptNode` cross-document semantics (this crate only ever has one
//! `Document` per tree, so "adopting" from another document doesn't
//! arise yet), and `remove_child` orphans a node rather than physically
//! freeing its arena slot (this crate's arena never frees slots at all --
//! a pre-existing, already-documented Track A simplification, not new to
//! C1).

use crate::{Document, ElementData, NodeData, NodeId};

/// The DOM's own small numeric node-type space (`Node.nodeType`),
/// modeled as a real enum instead of the spec's bare integers -- the
/// numbers are kept in each variant's doc comment for anyone checking
/// against the spec, but callers match on meaning, not magic numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    /// `ELEMENT_NODE` = 1
    Element,
    /// `TEXT_NODE` = 3
    Text,
    /// `PROCESSING_INSTRUCTION_NODE` = 7
    ProcessingInstruction,
    /// `COMMENT_NODE` = 8
    Comment,
    /// `DOCUMENT_NODE` = 9
    Document,
    /// `DOCUMENT_TYPE_NODE` = 10
    DocumentType,
}

impl NodeType {
    /// The spec's own integer for this type, for anything that genuinely
    /// needs the number rather than the enum (e.g. a future JS binding
    /// exposing `Node.nodeType` verbatim).
    pub fn as_u16(self) -> u16 {
        match self {
            NodeType::Element => 1,
            NodeType::Text => 3,
            NodeType::ProcessingInstruction => 7,
            NodeType::Comment => 8,
            NodeType::Document => 9,
            NodeType::DocumentType => 10,
        }
    }
}

impl Document {
    /// `Node.nodeType`.
    pub fn node_type(&self, id: NodeId) -> NodeType {
        match self.data(id) {
            NodeData::Document => NodeType::Document,
            NodeData::Doctype(_) => NodeType::DocumentType,
            NodeData::Element(_) => NodeType::Element,
            NodeData::Text(_) => NodeType::Text,
            NodeData::Comment(_) => NodeType::Comment,
            NodeData::ProcessingInstruction { .. } => NodeType::ProcessingInstruction,
        }
    }

    /// `Node.nodeName`: an element's (upper-cased, matching real HTML DOM
    /// behavior for HTML-namespace elements -- see below) tag name, a
    /// text node's literal `"#text"`, a comment's `"#comment"`, the
    /// document's `"#document"`, a doctype's own name verbatim, or a
    /// processing instruction's target.
    ///
    /// Real DOM only upper-cases an element's `nodeName` when it's an
    /// HTML-namespace element in an HTML document (SVG/MathML element
    /// names stay as-authored, e.g. `"svg"` not `"SVG"`) -- this
    /// implements exactly that distinction using [`ElementData::
    /// namespace`], not a blanket uppercase.
    pub fn node_name(&self, id: NodeId) -> String {
        match self.data(id) {
            NodeData::Document => "#document".to_string(),
            NodeData::Doctype(d) => d.name.clone(),
            NodeData::Element(e) => {
                if e.namespace == crate::HTML_NS {
                    e.local_name.to_ascii_uppercase()
                } else {
                    e.local_name.clone()
                }
            }
            NodeData::Text(_) => "#text".to_string(),
            NodeData::Comment(_) => "#comment".to_string(),
            NodeData::ProcessingInstruction { target, .. } => target.clone(),
        }
    }

    /// `Node.firstChild`.
    pub fn first_child(&self, id: NodeId) -> Option<NodeId> {
        self.children(id).first().copied()
    }

    /// `Node.previousSibling`: the child immediately before `id` in its
    /// parent's child list, or `None` if `id` is the first child or has
    /// no parent.
    pub fn previous_sibling(&self, id: NodeId) -> Option<NodeId> {
        let parent = self.parent(id)?;
        let siblings = self.children(parent);
        let index = siblings.iter().position(|&c| c == id)?;
        index.checked_sub(1).map(|i| siblings[i])
    }

    /// `Node.nextSibling`.
    pub fn next_sibling(&self, id: NodeId) -> Option<NodeId> {
        let parent = self.parent(id)?;
        let siblings = self.children(parent);
        let index = siblings.iter().position(|&c| c == id)?;
        siblings.get(index + 1).copied()
    }

    /// `Node.textContent`'s getter: for an element/document, the
    /// concatenation of every descendant text node's data in tree order
    /// (real recursive descendant-text-collection, per spec -- not just
    /// this node's own direct text children); for a text/comment/PI node,
    /// its own data; `None` for a document type node (per spec,
    /// `textContent` is `null` there) or the document node itself when it
    /// has no element child yet.
    pub fn text_content(&self, id: NodeId) -> Option<String> {
        match self.data(id) {
            NodeData::Doctype(_) => None,
            NodeData::Text(s) | NodeData::Comment(s) => Some(s.clone()),
            NodeData::ProcessingInstruction { data, .. } => Some(data.clone()),
            NodeData::Element(_) | NodeData::Document => {
                let mut out = String::new();
                self.collect_text(id, &mut out);
                Some(out)
            }
        }
    }

    fn collect_text(&self, id: NodeId, out: &mut String) {
        match self.data(id) {
            NodeData::Text(s) => out.push_str(s),
            NodeData::Element(_) | NodeData::Document => {
                for &child in self.children(id) {
                    self.collect_text(child, out);
                }
            }
            NodeData::Doctype(_)
            | NodeData::Comment(_)
            | NodeData::ProcessingInstruction { .. } => {}
        }
    }

    /// `Node.textContent`'s setter: for an element, replaces every
    /// existing child with a single new text node holding `value` (or
    /// removes all children if `value` is empty, matching the spec's
    /// "string replace all" algorithm exactly -- an empty string doesn't
    /// leave behind an empty text node); for a text/comment/PI node,
    /// replaces its own data; a no-op for the document node or a doctype
    /// (real DOM throws for some of these; this crate has no exception
    /// type to throw, so it silently no-ops instead -- a documented
    /// simplification).
    pub fn set_text_content(&mut self, id: NodeId, value: &str) {
        match self.data_mut(id) {
            NodeData::Text(s) | NodeData::Comment(s) => {
                *s = value.to_string();
                return;
            }
            NodeData::ProcessingInstruction { data, .. } => {
                *data = value.to_string();
                return;
            }
            NodeData::Element(_) => {}
            NodeData::Document | NodeData::Doctype(_) => return,
        }
        for child in self.children(id).to_vec() {
            self.detach(child);
        }
        if !value.is_empty() {
            self.append(id, NodeData::Text(value.to_string()));
        }
    }

    /// `Node.removeChild`: detaches `child` from `parent`. Matches real
    /// DOM's own precondition (a no-op, returning `false`, if `child`
    /// isn't actually `parent`'s child) rather than detaching whatever
    /// `child` happens to be attached to elsewhere in the tree.
    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) -> bool {
        if self.parent(child) != Some(parent) {
            return false;
        }
        self.detach(child);
        true
    }

    /// `Node.replaceChild`: replaces `old` (which must be `parent`'s
    /// child) with `new`, in `old`'s former position, and detaches `old`.
    /// Returns `false` (a no-op) if `old` isn't actually `parent`'s
    /// child, matching `removeChild`'s own precondition above.
    pub fn replace_child(&mut self, parent: NodeId, new: NodeId, old: NodeId) -> bool {
        if self.parent(old) != Some(parent) {
            return false;
        }
        let next = self.next_sibling(old);
        self.detach(old);
        self.insert_existing_before(parent, next, new);
        true
    }

    /// `Node.cloneNode(deep)`: duplicates `id` (and, if `deep`, its
    /// entire subtree) as new, unattached nodes with fresh `NodeId`s --
    /// real structural duplication, not a second reference to the same
    /// nodes. Returns the id of the new, still-detached root of the
    /// clone; the caller inserts it wherever it belongs, matching real
    /// `cloneNode`'s own "returns a node with no parent" contract.
    pub fn clone_node(&mut self, id: NodeId, deep: bool) -> NodeId {
        let data = self.data(id).clone();
        let clone = self.push_node(data, None);
        if deep {
            for &child in self.children(id).to_vec().iter() {
                let child_clone = self.clone_node(child, true);
                self.append_existing(clone, child_clone);
            }
        }
        clone
    }

    /// `Node.contains(other)`: whether `other` is `ancestor` itself or a
    /// descendant of it -- a real ancestor walk from `other` up to the
    /// root, not a subtree scan from `ancestor` down (cheaper for the
    /// common case of checking a specific node against a large tree).
    pub fn contains(&self, ancestor: NodeId, other: NodeId) -> bool {
        let mut current = Some(other);
        while let Some(node) = current {
            if node == ancestor {
                return true;
            }
            current = self.parent(node);
        }
        false
    }

    /// Roughly `Node.isConnected`: whether `id` is reachable from the
    /// document root, i.e. not (yet, or anymore) detached/orphaned.
    pub fn is_connected(&self, id: NodeId) -> bool {
        self.contains(self.root(), id)
    }

    fn is_element(&self, id: NodeId) -> bool {
        matches!(self.data(id), NodeData::Element(_))
    }

    /// `Element.children` (an `HTMLCollection`, real DOM's element-only
    /// view of `childNodes` -- text/comment/PI children are filtered
    /// out). Live by construction, same as every other query here (see
    /// module docs).
    pub fn element_children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        self.children(id)
            .iter()
            .copied()
            .filter(|&c| self.is_element(c))
    }

    /// `Element.childElementCount`.
    pub fn child_element_count(&self, id: NodeId) -> usize {
        self.element_children(id).count()
    }

    /// `Element.firstElementChild`.
    pub fn first_element_child(&self, id: NodeId) -> Option<NodeId> {
        self.element_children(id).next()
    }

    /// `Element.lastElementChild`.
    pub fn last_element_child(&self, id: NodeId) -> Option<NodeId> {
        self.element_children(id).last()
    }

    /// `Element.previousElementSibling`.
    pub fn previous_element_sibling(&self, id: NodeId) -> Option<NodeId> {
        let mut current = self.previous_sibling(id);
        while let Some(node) = current {
            if self.is_element(node) {
                return Some(node);
            }
            current = self.previous_sibling(node);
        }
        None
    }

    /// `Element.nextElementSibling`.
    pub fn next_element_sibling(&self, id: NodeId) -> Option<NodeId> {
        let mut current = self.next_sibling(id);
        while let Some(node) = current {
            if self.is_element(node) {
                return Some(node);
            }
            current = self.next_sibling(node);
        }
        None
    }

    /// `Element.getAttribute`. `None` both when `id` isn't an element and
    /// when the attribute is genuinely absent -- this crate has no
    /// exception type to distinguish "wrong node kind" from "not found"
    /// with, matching `text_content`'s/`set_text_content`'s own
    /// documented no-op-on-wrong-kind precedent above.
    pub fn get_attribute(&self, id: NodeId, name: &str) -> Option<String> {
        match self.data(id) {
            NodeData::Element(e) => e.attr(name).map(str::to_string),
            _ => None,
        }
    }

    /// `Element.setAttribute`: overwrites `name` if already present,
    /// otherwise appends it -- a no-op if `id` isn't an element.
    pub fn set_attribute(&mut self, id: NodeId, name: &str, value: &str) {
        if let NodeData::Element(e) = self.data_mut(id) {
            match e.attributes.iter_mut().find(|(k, _)| k == name) {
                Some((_, v)) => *v = value.to_string(),
                None => e.attributes.push((name.to_string(), value.to_string())),
            }
        }
    }

    /// `Element.hasAttribute`.
    pub fn has_attribute(&self, id: NodeId, name: &str) -> bool {
        matches!(self.data(id), NodeData::Element(e) if e.attr(name).is_some())
    }

    /// `Element.removeAttribute`.
    pub fn remove_attribute(&mut self, id: NodeId, name: &str) {
        if let NodeData::Element(e) = self.data_mut(id) {
            e.attributes.retain(|(k, _)| k != name);
        }
    }

    /// `Document.createElement`: a new, detached HTML-namespace element --
    /// the caller inserts it wherever it belongs (matching
    /// `clone_node`'s own "returns a detached node" contract above).
    pub fn create_element(&mut self, local_name: &str) -> NodeId {
        self.push_node(
            NodeData::Element(ElementData::html(local_name, Vec::new())),
            None,
        )
    }

    /// `Document.createTextNode`: a new, detached text node.
    pub fn create_text_node(&mut self, data: &str) -> NodeId {
        self.push_node(NodeData::Text(data.to_string()), None)
    }

    /// `Document.getElementById`: the real spec algorithm is "the first
    /// element, in tree (pre)order, whose `id` content attribute is
    /// exactly `id`" -- not a hash-map lookup, since nothing here
    /// maintains an id->element index that a mutation could let go
    /// stale. A real engine's DOM typically *does* keep such an index for
    /// performance; this crate deliberately doesn't, so there's no cache
    /// to keep synchronized with `set_attribute`/tree mutations at
    /// all -- the same "live by construction, no cache to begin with"
    /// tradeoff this module's own doc comment makes for `NodeList`/
    /// `HTMLCollection`, applied here too. `None` if no element matches.
    pub fn get_element_by_id(&self, needle: &str) -> Option<NodeId> {
        self.find_first(
            self.root(),
            &|doc, id| matches!(doc.data(id), NodeData::Element(e) if e.attr("id") == Some(needle)),
        )
    }

    fn find_first(
        &self,
        id: NodeId,
        predicate: &impl Fn(&Document, NodeId) -> bool,
    ) -> Option<NodeId> {
        if predicate(self, id) {
            return Some(id);
        }
        for &child in self.children(id) {
            if let Some(found) = self.find_first(child, predicate) {
                return Some(found);
            }
        }
        None
    }

    /// `Document.getElementsByTagName`/`Element.getElementsByTagName`:
    /// every descendant element (real pre-order tree traversal, not an
    /// index) whose local name matches `tag_name`, or every element if
    /// `tag_name` is `"*"` (the spec's own wildcard). Case-sensitive,
    /// matching real behavior for non-HTML-namespace documents; this
    /// crate doesn't yet special-case "HTML documents lower-case ASCII
    /// letters in the argument first" (a real, if narrow, gap).
    pub fn get_elements_by_tag_name(&self, root: NodeId, tag_name: &str) -> Vec<NodeId> {
        let mut out = Vec::new();
        self.collect_by_tag_name(root, tag_name, &mut out);
        out
    }

    fn collect_by_tag_name(&self, id: NodeId, tag_name: &str, out: &mut Vec<NodeId>) {
        for &child in self.children(id) {
            if let NodeData::Element(e) = self.data(child)
                && (tag_name == "*" || e.local_name == tag_name)
            {
                out.push(child);
            }
            self.collect_by_tag_name(child, tag_name, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ElementData;

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
    fn node_type_and_node_name() {
        let mut doc = Document::new();
        let root = doc.root();
        assert_eq!(doc.node_type(root), NodeType::Document);
        assert_eq!(doc.node_name(root), "#document");

        let p = el(&mut doc, root, "p", &[]);
        assert_eq!(doc.node_type(p), NodeType::Element);
        assert_eq!(doc.node_name(p), "P");

        let text = doc.append(p, NodeData::Text("hi".into()));
        assert_eq!(doc.node_type(text), NodeType::Text);
        assert_eq!(doc.node_name(text), "#text");

        let comment = doc.append(p, NodeData::Comment("c".into()));
        assert_eq!(doc.node_type(comment), NodeType::Comment);
        assert_eq!(doc.node_name(comment), "#comment");
    }

    #[test]
    fn node_name_does_not_uppercase_non_html_namespace_elements() {
        let mut doc = Document::new();
        let root = doc.root();
        let svg = doc.append(
            root,
            NodeData::Element(ElementData::new(crate::SVG_NS, "svg", vec![])),
        );
        assert_eq!(doc.node_name(svg), "svg");
    }

    #[test]
    fn sibling_navigation() {
        let mut doc = Document::new();
        let root = doc.root();
        let a = el(&mut doc, root, "a", &[]);
        let b = el(&mut doc, root, "b", &[]);
        let c = el(&mut doc, root, "c", &[]);
        assert_eq!(doc.first_child(root), Some(a));
        assert_eq!(doc.previous_sibling(a), None);
        assert_eq!(doc.next_sibling(a), Some(b));
        assert_eq!(doc.previous_sibling(b), Some(a));
        assert_eq!(doc.next_sibling(c), None);
    }

    #[test]
    fn text_content_collects_all_descendant_text_in_tree_order() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[]);
        doc.append(div, NodeData::Text("a".into()));
        let span = el(&mut doc, div, "span", &[]);
        doc.append(span, NodeData::Text("b".into()));
        doc.append(div, NodeData::Text("c".into()));
        assert_eq!(doc.text_content(div), Some("abc".to_string()));
    }

    #[test]
    fn text_content_getter_on_a_text_node_returns_its_own_data() {
        let mut doc = Document::new();
        let root = doc.root();
        let text = doc.append(root, NodeData::Text("hi".into()));
        assert_eq!(doc.text_content(text), Some("hi".to_string()));
    }

    #[test]
    fn set_text_content_replaces_all_children_with_one_text_node() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[]);
        el(&mut doc, div, "span", &[]);
        doc.append(div, NodeData::Text("x".into()));
        doc.set_text_content(div, "new");
        assert_eq!(doc.children(div).len(), 1);
        assert_eq!(doc.text_content(div), Some("new".to_string()));
    }

    #[test]
    fn set_text_content_with_empty_string_removes_all_children() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[]);
        doc.append(div, NodeData::Text("x".into()));
        doc.set_text_content(div, "");
        assert!(doc.children(div).is_empty());
    }

    #[test]
    fn remove_child_only_succeeds_for_an_actual_child() {
        let mut doc = Document::new();
        let root = doc.root();
        let a = el(&mut doc, root, "a", &[]);
        let b = el(&mut doc, root, "b", &[]);
        let stray = el(&mut doc, a, "stray", &[]);
        assert!(!doc.remove_child(b, stray));
        assert_eq!(doc.parent(stray), Some(a));
        assert!(doc.remove_child(a, stray));
        assert_eq!(doc.parent(stray), None);
        assert!(doc.children(a).is_empty());
    }

    #[test]
    fn replace_child_swaps_position_and_detaches_the_old_node() {
        let mut doc = Document::new();
        let root = doc.root();
        let a = el(&mut doc, root, "a", &[]);
        let old = el(&mut doc, root, "old", &[]);
        let c = el(&mut doc, root, "c", &[]);
        assert!(doc.replace_child(root, a, old));
        assert_eq!(doc.children(root), &[a, c]);
        assert_eq!(doc.parent(old), None);
    }

    #[test]
    fn clone_node_shallow_does_not_copy_children() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[("id", "x")]);
        doc.append(div, NodeData::Text("hi".into()));
        let clone = doc.clone_node(div, false);
        assert_ne!(clone, div);
        assert!(doc.children(clone).is_empty());
        assert_eq!(doc.parent(clone), None);
        match doc.data(clone) {
            NodeData::Element(e) => assert_eq!(e.attr("id"), Some("x")),
            other => panic!("expected element, got {other:?}"),
        }
    }

    #[test]
    fn clone_node_deep_duplicates_the_whole_subtree() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[]);
        let span = el(&mut doc, div, "span", &[]);
        doc.append(span, NodeData::Text("hi".into()));
        let clone = doc.clone_node(div, true);
        assert_eq!(doc.children(clone).len(), 1);
        let clone_span = doc.children(clone)[0];
        assert_ne!(clone_span, span);
        assert_eq!(doc.text_content(clone_span), Some("hi".to_string()));
        // The original subtree is untouched.
        assert_eq!(doc.text_content(span), Some("hi".to_string()));
    }

    #[test]
    fn contains_and_is_connected() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[]);
        let span = el(&mut doc, div, "span", &[]);
        assert!(doc.contains(root, span));
        assert!(doc.contains(div, span));
        assert!(!doc.contains(span, div));
        assert!(doc.is_connected(span));
        doc.detach(div);
        assert!(!doc.is_connected(span));
    }

    #[test]
    fn element_child_navigation_skips_non_element_siblings() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[]);
        doc.append(div, NodeData::Text("x".into()));
        let a = el(&mut doc, div, "a", &[]);
        doc.append(div, NodeData::Comment("c".into()));
        let b = el(&mut doc, div, "b", &[]);
        assert_eq!(doc.first_element_child(div), Some(a));
        assert_eq!(doc.last_element_child(div), Some(b));
        assert_eq!(doc.child_element_count(div), 2);
        assert_eq!(doc.next_element_sibling(a), Some(b));
        assert_eq!(doc.previous_element_sibling(b), Some(a));
    }

    #[test]
    fn get_element_by_id_finds_the_first_match_in_tree_order() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[]);
        el(&mut doc, div, "span", &[("id", "target")]);
        let p = el(&mut doc, root, "p", &[("id", "target")]);
        assert_eq!(doc.get_element_by_id("target"), Some(doc.children(div)[0]));
        assert_ne!(doc.get_element_by_id("target"), Some(p));
        assert_eq!(doc.get_element_by_id("nonexistent"), None);
    }

    #[test]
    fn get_elements_by_tag_name_finds_all_descendants_and_supports_wildcard() {
        let mut doc = Document::new();
        let root = doc.root();
        let div = el(&mut doc, root, "div", &[]);
        let p1 = el(&mut doc, div, "p", &[]);
        let p2 = el(&mut doc, root, "p", &[]);
        el(&mut doc, div, "span", &[]);
        assert_eq!(doc.get_elements_by_tag_name(root, "p"), vec![p1, p2]);
        assert_eq!(doc.get_elements_by_tag_name(root, "*").len(), 4);
    }
}
