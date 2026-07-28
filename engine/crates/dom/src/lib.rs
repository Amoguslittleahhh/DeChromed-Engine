//! Roadmap phase: Track A (tree shape) / Track C1 (addressable DOM API).
//!
//! This crate holds the tree representation shared by the HTML parser (A2/A3),
//! the CSS engine (A4-A7), and eventually the JS bindings (C1/C8). It starts as
//! a plain arena-backed tree with no spec-mandated API surface (no live
//! `NodeList`, no mutation algorithms) -- that's C1's job once Track C starts.
//! For now it only needs to be shaped correctly enough for the HTML tree
//! construction algorithm (A3) to build against.

use std::fmt;

/// Index into a [`Document`]'s node arena. Nodes never move once inserted,
/// so a `NodeId` stays valid for the document's lifetime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(u32);

/// The document root: an arena of nodes plus the tree edges between them.
///
/// Real engines don't represent the DOM as a single flat arena forever --
/// this is a deliberately simple starting representation for A2/A3, not a
/// architectural commitment. See ROADMAP.md Track B9 for why layout will
/// need its own tree shape downstream regardless of what this looks like.
#[derive(Debug, Default)]
pub struct Document {
    nodes: Vec<NodeSlot>,
}

#[derive(Debug)]
struct NodeSlot {
    data: NodeData,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
}

/// What kind of node this is, and the data specific to that kind.
///
/// This enum is intentionally small right now (matches what the HTML5
/// tokenizer in A2 can produce): elements, text, comments, and the document
/// node itself. Doctype nodes get added alongside A3's tree-construction
/// insertion modes, which need to represent `<!DOCTYPE html>` explicitly.
#[derive(Debug, Clone)]
pub enum NodeData {
    Document,
    Element(ElementData),
    Text(String),
    Comment(String),
}

/// An element's tag name and attributes.
///
/// `local_name` is kept separate from a future `namespace` field on purpose:
/// A3's foreign-content handling (SVG/MathML embedded in HTML) needs
/// namespace-qualified names, and retrofitting that onto a bare `String` tag
/// name later would touch every call site. Better to leave the seam visible
/// now even though namespaces aren't implemented yet.
#[derive(Debug, Clone)]
pub struct ElementData {
    pub local_name: String,
    pub attributes: Vec<(String, String)>,
}

impl ElementData {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

impl Document {
    pub fn new() -> Self {
        let mut doc = Document { nodes: Vec::new() };
        let root = doc.push_node(NodeData::Document, None);
        debug_assert_eq!(root, doc.root());
        doc
    }

    /// The implicit `#document` node created with the document.
    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    fn push_node(&mut self, data: NodeData, parent: Option<NodeId>) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(NodeSlot {
            data,
            parent,
            children: Vec::new(),
        });
        id
    }

    /// Appends a new child node under `parent`, returning the new node's id.
    ///
    /// Panics if `parent` is not a valid id in this document -- this is an
    /// internal tree-building API for the HTML parser (A3), not a
    /// spec-shaped DOM mutation method (that's C1's `Node.appendChild`,
    /// which has to handle far more: existing-parent removal, event
    /// dispatch hooks, mutation records, etc).
    pub fn append(&mut self, parent: NodeId, data: NodeData) -> NodeId {
        let id = self.push_node(data, Some(parent));
        self.nodes[parent.0 as usize].children.push(id);
        id
    }

    pub fn data(&self, id: NodeId) -> &NodeData {
        &self.nodes[id.0 as usize].data
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id.0 as usize].parent
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id.0 as usize].children
    }

    /// Depth-first pre-order walk starting at `id`, calling `visit` for each
    /// node. Used by the html5lib-tests harness to serialize a tree back out
    /// for comparison against expected test output.
    pub fn walk(&self, id: NodeId, visit: &mut impl FnMut(NodeId, usize)) {
        self.walk_inner(id, 0, visit);
    }

    fn walk_inner(&self, id: NodeId, depth: usize, visit: &mut impl FnMut(NodeId, usize)) {
        visit(id, depth);
        for &child in self.children(id) {
            self.walk_inner(child, depth + 1, visit);
        }
    }
}

impl fmt::Display for Document {
    /// A `html5lib-tests`-style indented tree dump, e.g.:
    /// ```text
    /// #document
    ///   <html>
    ///     <head>
    ///     <body>
    ///       "hello"
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut result = Ok(());
        self.walk(self.root(), &mut |id, depth| {
            if result.is_err() {
                return;
            }
            let indent = "  ".repeat(depth);
            result = match self.data(id) {
                NodeData::Document => writeln!(f, "{indent}#document"),
                NodeData::Element(el) => writeln!(f, "{indent}<{}>", el.local_name),
                NodeData::Text(t) => writeln!(f, "{indent}\"{t}\""),
                NodeData::Comment(c) => writeln!(f, "{indent}<!-- {c} -->"),
            };
        });
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_document_has_only_the_document_node() {
        let doc = Document::new();
        assert!(matches!(doc.data(doc.root()), NodeData::Document));
        assert!(doc.children(doc.root()).is_empty());
    }

    #[test]
    fn append_builds_a_tree() {
        let mut doc = Document::new();
        let html = doc.append(
            doc.root(),
            NodeData::Element(ElementData {
                local_name: "html".into(),
                attributes: vec![],
            }),
        );
        let body = doc.append(
            html,
            NodeData::Element(ElementData {
                local_name: "body".into(),
                attributes: vec![],
            }),
        );
        doc.append(body, NodeData::Text("hi".into()));

        assert_eq!(doc.parent(body), Some(html));
        assert_eq!(doc.children(html), &[body]);
        assert_eq!(
            doc.to_string(),
            "#document\n  <html>\n    <body>\n      \"hi\"\n"
        );
    }
}
