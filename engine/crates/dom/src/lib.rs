//! Roadmap phase: Track A (tree shape) / Track C1 (addressable DOM API).
//!
//! This crate holds the tree representation shared by the HTML parser (A2/A3),
//! the CSS engine (A4-A7), and eventually the JS bindings (C1/C8). It starts as
//! a plain arena-backed tree with no spec-mandated API surface (no live
//! `NodeList`, no mutation algorithms) -- that's C1's job once Track C starts.
//! For now it only needs to be shaped correctly enough for the HTML tree
//! construction algorithm (A3) to build against, including the handful of
//! primitives A3's insertion-mode algorithms actually need: inserting before
//! an arbitrary reference node (for foster parenting), detaching/reattaching
//! an existing node (for the adoption agency algorithm's reparenting), and
//! merging adjacent character insertions into one text node (matching how
//! html5lib-tests' expected tree dumps coalesce consecutive characters).

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
#[derive(Debug, Clone)]
pub enum NodeData {
    Document,
    Doctype(DoctypeData),
    Element(ElementData),
    Text(String),
    Comment(String),
}

#[derive(Debug, Clone, Default)]
pub struct DoctypeData {
    pub name: String,
    pub public_id: Option<String>,
    pub system_id: Option<String>,
}

/// An element's tag name and attributes.
///
/// `local_name` is kept separate from a future `namespace` field on purpose:
/// full foreign-content handling (SVG/MathML embedded in HTML) needs
/// namespace-qualified names, and retrofitting that onto a bare `String` tag
/// name later would touch every call site. Better to leave the seam visible
/// now even though namespaces aren't implemented yet (a documented A3 gap --
/// see `tree_builder.rs`).
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

    /// Sets `name` to `value` only if `name` isn't already present. Used by
    /// the tree builder's "second `<html>`/`<body>` start tag" handling,
    /// where the spec merges any *new* attributes onto the existing root
    /// element without overwriting ones already there.
    pub fn set_if_absent(&mut self, name: &str, value: &str) {
        if !self.attributes.iter().any(|(k, _)| k == name) {
            self.attributes.push((name.to_string(), value.to_string()));
        }
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
    pub fn append(&mut self, parent: NodeId, data: NodeData) -> NodeId {
        let id = self.push_node(data, Some(parent));
        self.nodes[parent.0 as usize].children.push(id);
        id
    }

    /// Appends a character to `parent`'s content, merging into the last
    /// child if it's already a text node (matching the spec's "insert a
    /// character" algorithm, and how html5lib-tests' expected tree dumps
    /// coalesce runs of consecutive characters into one `"..."` string).
    pub fn append_char(&mut self, parent: NodeId, c: char) {
        if let Some(&last) = self.nodes[parent.0 as usize].children.last() {
            if let NodeData::Text(s) = &mut self.nodes[last.0 as usize].data {
                s.push(c);
                return;
            }
        }
        self.append(parent, NodeData::Text(c.to_string()));
    }

    /// Inserts a new node with `data` as a child of `parent`, positioned
    /// immediately before `before` (or at the end if `before` is `None`).
    /// Used for foster parenting: table-related insertion modes redirect
    /// stray content to just before the offending `<table>` element rather
    /// than appending it normally.
    pub fn insert_before(
        &mut self,
        parent: NodeId,
        before: Option<NodeId>,
        data: NodeData,
    ) -> NodeId {
        let id = self.push_node(data, Some(parent));
        self.insert_id_before(parent, before, id);
        id
    }

    /// Like [`Document::append_char`], but inserting before a reference
    /// node instead of at the end -- the foster-parenting equivalent of
    /// character merging.
    pub fn insert_char_before(&mut self, parent: NodeId, before: Option<NodeId>, c: char) {
        let children = &self.nodes[parent.0 as usize].children;
        let prev = match before {
            Some(b) => children.iter().position(|&c| c == b).and_then(|i| {
                if i == 0 {
                    None
                } else {
                    Some(children[i - 1])
                }
            }),
            None => children.last().copied(),
        };
        if let Some(prev) = prev {
            if let NodeData::Text(s) = &mut self.nodes[prev.0 as usize].data {
                s.push(c);
                return;
            }
        }
        self.insert_before(parent, before, NodeData::Text(c.to_string()));
    }

    fn insert_id_before(&mut self, parent: NodeId, before: Option<NodeId>, id: NodeId) {
        let children = &mut self.nodes[parent.0 as usize].children;
        match before {
            Some(b) => {
                let idx = children
                    .iter()
                    .position(|&c| c == b)
                    .unwrap_or(children.len());
                children.insert(idx, id);
            }
            None => children.push(id),
        }
    }

    /// Detaches `id` from its current parent (if any), leaving it and its
    /// own children intact but orphaned until reattached. Used by the
    /// adoption agency algorithm, which moves existing nodes around the
    /// tree rather than only ever creating new ones.
    pub fn detach(&mut self, id: NodeId) {
        if let Some(parent) = self.nodes[id.0 as usize].parent {
            self.nodes[parent.0 as usize].children.retain(|&c| c != id);
        }
        self.nodes[id.0 as usize].parent = None;
    }

    /// Re-attaches an already-detached (or currently-attached-elsewhere)
    /// node as the last child of `parent`.
    pub fn append_existing(&mut self, parent: NodeId, id: NodeId) {
        self.detach(id);
        self.nodes[id.0 as usize].parent = Some(parent);
        self.nodes[parent.0 as usize].children.push(id);
    }

    /// Re-attaches an already-detached node before a reference child of
    /// `parent` (or at the end if `before` is `None`).
    pub fn insert_existing_before(&mut self, parent: NodeId, before: Option<NodeId>, id: NodeId) {
        self.detach(id);
        self.nodes[id.0 as usize].parent = Some(parent);
        self.insert_id_before(parent, before, id);
    }

    pub fn data(&self, id: NodeId) -> &NodeData {
        &self.nodes[id.0 as usize].data
    }

    pub fn data_mut(&mut self, id: NodeId) -> &mut NodeData {
        &mut self.nodes[id.0 as usize].data
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id.0 as usize].parent
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id.0 as usize].children
    }

    pub fn last_child(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id.0 as usize].children.last().copied()
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
            result = (|| {
                match self.data(id) {
                    NodeData::Document => writeln!(f, "{indent}#document")?,
                    NodeData::Doctype(d) => match (&d.public_id, &d.system_id) {
                        (None, None) => writeln!(f, "{indent}<!DOCTYPE {}>", d.name)?,
                        (public, system) => writeln!(
                            f,
                            "{indent}<!DOCTYPE {} \"{}\" \"{}\">",
                            d.name,
                            public.as_deref().unwrap_or(""),
                            system.as_deref().unwrap_or("")
                        )?,
                    },
                    NodeData::Element(el) => {
                        writeln!(f, "{indent}<{}>", el.local_name)?;
                        for (k, v) in &el.attributes {
                            writeln!(f, "{indent}  {k}=\"{v}\"")?;
                        }
                    }
                    NodeData::Text(t) => writeln!(f, "{indent}\"{t}\"")?,
                    NodeData::Comment(c) => writeln!(f, "{indent}<!-- {c} -->")?,
                }
                Ok(())
            })();
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

    #[test]
    fn append_char_merges_into_last_text_node() {
        let mut doc = Document::new();
        let body = doc.append(
            doc.root(),
            NodeData::Element(ElementData {
                local_name: "body".into(),
                attributes: vec![],
            }),
        );
        doc.append_char(body, 'a');
        doc.append_char(body, 'b');
        doc.append_char(body, 'c');
        assert_eq!(doc.children(body).len(), 1);
        match doc.data(doc.children(body)[0]) {
            NodeData::Text(s) => assert_eq!(s, "abc"),
            other => panic!("expected text node, got {other:?}"),
        }
    }

    #[test]
    fn detach_and_reattach_moves_a_node() {
        let mut doc = Document::new();
        let a = doc.append(
            doc.root(),
            NodeData::Element(ElementData {
                local_name: "a".into(),
                attributes: vec![],
            }),
        );
        let b = doc.append(
            doc.root(),
            NodeData::Element(ElementData {
                local_name: "b".into(),
                attributes: vec![],
            }),
        );
        let child = doc.append(a, NodeData::Text("x".into()));
        doc.append_existing(b, child);
        assert_eq!(doc.children(a), &[]);
        assert_eq!(doc.children(b), &[child]);
        assert_eq!(doc.parent(child), Some(b));
    }
}
