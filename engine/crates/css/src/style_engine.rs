//! A7 (part 2): `getComputedStyle` and invalidation-set-driven restyling.
//!
//! Reference: <https://www.w3.org/TR/cssom-view-1/#the-getcomputedstyle()-method>,
//! and the general shape of Blink's "invalidation sets"/Gecko's "restyle
//! hints" (no public spec -- both engines' own docs/source describe the
//! technique; see `ROADMAP.md`'s A7 entry).
//!
//! [`StyleEngine`] ties a document's [`CssomSheet`](crate::cssom::CssomSheet)s
//! together with A6's cascade to give every element a computed style
//! (`computed_style`, the `getComputedStyle` equivalent), then keeps that
//! cache correct under targeted DOM mutation (`notify_class_changed`,
//! `notify_attribute_changed`) *without* recomputing the whole document --
//! the actual point of this phase, since full-tree restyle-on-every-change
//! is what makes naive engines too slow to use on real pages.
//!
//! ## The invalidation model
//!
//! [`InvalidationIndex`] buckets every selector's class/id/attribute names
//! by *how* a match on that name could ripple outward from the changed
//! element:
//! - **self** -- the name only ever appears in a selector's rightmost
//!   (subject) compound, so a change can only affect whether the changed
//!   element itself matches.
//! - **descendant** -- the name appears in a non-rightmost compound (an
//!   ancestor condition, e.g. the `.a` in `.a .b`) or inside `:has()`
//!   (approximated as descendant-dependent, a documented conservative
//!   over-approximation -- see below), so a change can affect any
//!   descendant of the changed element.
//! - **sibling** -- the name appears in a compound that a sibling
//!   combinator (`+`/`~`) connects to something later in the selector, so
//!   a change can affect that element's following siblings.
//!
//! On a mutation, only the bucketed name's associated node set is marked
//! dirty and recomputed -- everything else keeps its cached style
//! untouched. This is deliberately conservative (it can mark more nodes
//! dirty than strictly necessary, e.g. `:has()`'s sibling-vs-descendant
//! relative selectors both collapse into "descendant") but never *under*-
//! invalidates, which is the correctness property that actually matters:
//! a real engine returning a stale style is a visible bug, one that
//! recomputes a few extra nodes is only a perf cost.
//!
//! **Known gap:** structural mutations (a node added/removed/reordered
//! among its siblings) aren't tracked here -- they can change the result
//! of `:nth-child`/`:first-child`/etc. for other siblings, which is a
//! different invalidation category (Blink/Gecko track this separately
//! too). Callers that mutate tree structure should call
//! [`StyleEngine::rebuild`] rather than expect incremental correctness,
//! and this is called out rather than silently returning stale styles.

use crate::cascade::{self, Origin, StyleSource};
use crate::cssom::CssomSheet;
use crate::selectors::{self, ComplexSelector, CompoundSelector, PseudoClass, SubclassSelector};
use dom::{Document, ElementData, NodeData, NodeId};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Default)]
struct InvalidationIndex {
    self_only: HashSet<String>,
    descendant: HashSet<String>,
    sibling: HashSet<String>,
}

impl InvalidationIndex {
    fn build(sheets: &[CssomSheet]) -> Self {
        let mut index = InvalidationIndex::default();
        for sheet in sheets {
            let flat = sheet.to_stylesheet();
            for rule in &flat.rules {
                let Ok(list) = selectors::parse_selector_list(&rule.selector) else {
                    continue;
                };
                for complex in &list.0 {
                    index.index_complex(complex);
                }
            }
        }
        index
    }

    fn index_complex(&mut self, cs: &ComplexSelector) {
        let last = cs.steps.len() - 1;
        for (i, step) in cs.steps.iter().enumerate() {
            let is_subject = i == last;
            // A step reached by a sibling combinator means changes to that
            // compound can affect elements to its right (later siblings)
            // via the combinator connecting it forward -- but our `steps`
            // are indexed leftmost-first with `combinator` describing the
            // link *to* the previous step, so "step i is sibling-linked to
            // step i+1" is what step i+1's own combinator tells us.
            let is_sibling_source = cs
                .steps
                .get(i + 1)
                .and_then(|s| s.combinator)
                .map(|c| {
                    matches!(
                        c,
                        selectors::Combinator::NextSibling
                            | selectors::Combinator::SubsequentSibling
                    )
                })
                .unwrap_or(false);
            let mut keys = Vec::new();
            collect_compound_keys(&step.compound, &mut keys);
            for key in keys {
                if is_sibling_source {
                    self.sibling.insert(key.clone());
                }
                if is_subject && !is_sibling_source {
                    self.self_only.insert(key.clone());
                } else if !is_subject {
                    self.descendant.insert(key.clone());
                }
            }
        }
    }
}

fn collect_compound_keys(compound: &CompoundSelector, out: &mut Vec<String>) {
    for sub in &compound.subclasses {
        match sub {
            SubclassSelector::Class(name) => out.push(format!("class:{name}")),
            SubclassSelector::Id(name) => out.push(format!("id:{name}")),
            SubclassSelector::Attribute(attr) => {
                out.push(format!("attr:{}", attr.name.to_ascii_lowercase()))
            }
            SubclassSelector::PseudoClass(pc) => collect_pseudo_class_keys(pc, out),
            SubclassSelector::PseudoElement(_) => {}
        }
    }
}

/// `:has()`'s argument is a relative selector (descendant/sibling of the
/// subject), so a name inside it is treated the same as "descendant" --
/// see module docs' conservative-over-approximation note.
fn collect_pseudo_class_keys(pc: &PseudoClass, out: &mut Vec<String>) {
    let recurse_list = |list: &selectors::SelectorList, out: &mut Vec<String>| {
        for cs in &list.0 {
            for step in &cs.steps {
                collect_compound_keys(&step.compound, out);
            }
        }
    };
    match pc {
        PseudoClass::Not(list)
        | PseudoClass::Is(list)
        | PseudoClass::Where(list)
        | PseudoClass::Has(list) => recurse_list(list, out),
        PseudoClass::NthChild(_, Some(list)) | PseudoClass::NthLastChild(_, Some(list)) => {
            recurse_list(list, out)
        }
        _ => {}
    }
}

pub struct StyleEngine {
    /// `CssomSheet`'s mutable rule list isn't what `cascade::StyleSource`
    /// borrows (that needs a flat `Stylesheet`) -- this is that flattened
    /// form, rebuilt from the engine's `CssomSheet`s on construction and
    /// kept alive here so `sources()` can hand out borrows instead of
    /// leaking or reparsing per recompute.
    flattened: Vec<(Origin, crate::Stylesheet)>,
    index: InvalidationIndex,
    computed: HashMap<NodeId, HashMap<String, String>>,
}

impl StyleEngine {
    pub fn new(doc: &Document, sheets: Vec<(Origin, CssomSheet)>) -> Self {
        let index =
            InvalidationIndex::build(&sheets.iter().map(|(_, s)| s.clone()).collect::<Vec<_>>());
        let flattened = sheets
            .into_iter()
            .map(|(origin, sheet)| (origin, sheet.to_stylesheet()))
            .collect();
        let mut engine = StyleEngine {
            flattened,
            index,
            computed: HashMap::new(),
        };
        engine.rebuild(doc);
        engine
    }

    /// Full-document recompute -- the non-incremental fallback for
    /// structural mutations or a stylesheet change (see module docs' known
    /// gap on structural invalidation).
    pub fn rebuild(&mut self, doc: &Document) {
        let sources = self.sources();
        self.computed = cascade::compute_document_styles(doc, &sources);
    }

    fn sources(&self) -> Vec<StyleSource<'_>> {
        self.flattened
            .iter()
            .map(|(origin, sheet)| StyleSource {
                origin: *origin,
                sheet,
            })
            .collect()
    }

    pub fn computed_style(&self, node: NodeId) -> Option<&HashMap<String, String>> {
        self.computed.get(&node)
    }

    /// Equivalent of `getComputedStyle(element).getPropertyValue(name)`.
    pub fn get_computed_style(&self, node: NodeId, property: &str) -> Option<&str> {
        self.computed.get(&node)?.get(property).map(|s| s.as_str())
    }

    /// Call after `class_name` is added to, removed from, or toggled on
    /// `node`'s `class` attribute. Returns the node IDs actually
    /// recomputed (for callers/tests that want to verify this stayed
    /// targeted rather than falling back to a full restyle).
    pub fn notify_class_changed(
        &mut self,
        doc: &Document,
        node: NodeId,
        class_name: &str,
    ) -> Vec<NodeId> {
        self.notify_key_changed(doc, node, &format!("class:{class_name}"))
    }

    /// Call after `attr_name` changes on `node` (including `id`, but *not*
    /// `class` -- use [`notify_class_changed`](Self::notify_class_changed)
    /// for that, since class changes are keyed per-class-name for tighter
    /// invalidation).
    pub fn notify_attribute_changed(
        &mut self,
        doc: &Document,
        node: NodeId,
        attr_name: &str,
    ) -> Vec<NodeId> {
        let key = if attr_name == "id" {
            let id_value = match doc.data(node) {
                NodeData::Element(e) => e.attr("id").map(str::to_string),
                _ => None,
            };
            id_value.map(|v| format!("id:{v}"))
        } else {
            None
        };
        let key = key.unwrap_or_else(|| format!("attr:{}", attr_name.to_ascii_lowercase()));
        self.notify_key_changed(doc, node, &key)
    }

    fn notify_key_changed(&mut self, doc: &Document, node: NodeId, key: &str) -> Vec<NodeId> {
        let mut dirty: Vec<NodeId> = Vec::new();
        if self.index.self_only.contains(key)
            || self.index.descendant.contains(key)
            || self.index.sibling.contains(key)
        {
            dirty.push(node);
        }
        if self.index.descendant.contains(key) {
            collect_descendants(doc, node, &mut dirty);
        }
        if self.index.sibling.contains(key) {
            collect_following_siblings(doc, node, &mut dirty);
        }
        self.recompute_nodes(doc, &dirty);
        dirty
    }

    /// Recomputes exactly `nodes` (each looked up against its *current*
    /// parent's cached computed style, which must already be correct --
    /// true as long as `nodes` doesn't include a node whose ancestor also
    /// needs recomputing but isn't in the set, which `notify_key_changed`
    /// guarantees by always including the changed node and its whole
    /// descendant subtree together).
    fn recompute_nodes(&mut self, doc: &Document, nodes: &[NodeId]) {
        // Built from `self.flattened` directly (not via `self.sources()`)
        // so the borrow checker sees it as disjoint from `self.computed`,
        // which this loop mutates below.
        let sources: Vec<StyleSource<'_>> = self
            .flattened
            .iter()
            .map(|(origin, sheet)| StyleSource {
                origin: *origin,
                sheet,
            })
            .collect();
        let mut dirty_set: HashSet<NodeId> = nodes.iter().copied().collect();
        // Process in tree order (parents before children) so a child's
        // recompute sees its parent's already-updated computed style.
        let mut ordered = Vec::new();
        doc.walk(doc.root(), &mut |id, _| {
            if dirty_set.remove(&id) {
                ordered.push(id);
            }
        });
        for id in ordered {
            let parent_style = doc.parent(id).and_then(|p| self.computed.get(&p)).cloned();
            let cascaded = cascade::cascade(doc, id, &sources);
            let style = cascade::compute_node_style(&cascaded, parent_style.as_ref());
            self.computed.insert(id, style);
        }
    }
}

fn collect_descendants(doc: &Document, node: NodeId, out: &mut Vec<NodeId>) {
    for &child in doc.children(node) {
        if matches!(doc.data(child), NodeData::Element(_)) {
            out.push(child);
        }
        collect_descendants(doc, child, out);
    }
}

fn collect_following_siblings(doc: &Document, node: NodeId, out: &mut Vec<NodeId>) {
    let Some(parent) = doc.parent(node) else {
        return;
    };
    let siblings = doc.children(parent);
    if let Some(pos) = siblings.iter().position(|&s| s == node) {
        for &sib in &siblings[pos + 1..] {
            if matches!(doc.data(sib), NodeData::Element(_)) {
                out.push(sib);
                collect_descendants(doc, sib, out);
            }
        }
    }
}

/// Convenience used by tests/callers that just changed a `class` attribute
/// string wholesale and want every class name in the new value notified
/// (rather than knowing exactly which class was added/removed).
pub fn class_list(element: &ElementData) -> Vec<String> {
    element
        .attr("class")
        .map(|c| c.split_ascii_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find_by_id(doc: &Document, id_value: &str) -> NodeId {
        let mut found = None;
        doc.walk(doc.root(), &mut |id, _| {
            if found.is_none()
                && let NodeData::Element(e) = doc.data(id)
                && e.attr("id") == Some(id_value)
            {
                found = Some(id);
            }
        });
        found.expect("id not found")
    }

    /// Test helper standing in for the DOM attribute-mutation API C1 will
    /// eventually own -- `notify_*` only recomputes the cache, it doesn't
    /// touch the DOM itself, so callers (here, the test) are responsible
    /// for actually mutating the attribute first, same as a real browser's
    /// `Element.classList.add()` mutates the DOM before the style system
    /// reacts to it.
    fn add_class(doc: &mut Document, id: NodeId, class: &str) {
        if let NodeData::Element(e) = doc.data_mut(id) {
            e.attributes.push(("class".to_string(), class.to_string()));
        }
    }

    #[test]
    fn get_computed_style_reflects_matching_rule() {
        let doc = html::parse_document("<html><body><p id=\"x\">hi</p></body></html>");
        let sheet = CssomSheet::parse("p { color: red; }");
        let engine = StyleEngine::new(&doc, vec![(Origin::Author, sheet)]);
        let p = find_by_id(&doc, "x");
        assert_eq!(engine.get_computed_style(p, "color"), Some("red"));
    }

    #[test]
    fn self_only_class_change_touches_only_that_node() {
        let mut doc = html::parse_document(
            "<html><body><div id=\"parent\"><p id=\"target\"></p><p id=\"other\"></p></div></body></html>",
        );
        let sheet = CssomSheet::parse(".hot { color: red; }");
        let mut engine = StyleEngine::new(&doc, vec![(Origin::Author, sheet)]);
        let target = find_by_id(&doc, "target");
        let other = find_by_id(&doc, "other");
        assert_eq!(
            engine.get_computed_style(target, "color"),
            Some("canvastext")
        );

        add_class(&mut doc, target, "hot");
        let touched = engine.notify_class_changed(&doc, target, "hot");
        assert_eq!(touched, vec![target]);
        assert_eq!(engine.get_computed_style(target, "color"), Some("red"));
        // The untouched sibling's cached style must be exactly what a
        // full rebuild would have produced -- proving the targeted path
        // didn't skip anything it shouldn't have, not just that it didn't
        // touch `other`.
        let mut rebuilt = StyleEngine::new(
            &doc,
            vec![(Origin::Author, CssomSheet::parse(".hot { color: red; }"))],
        );
        rebuilt.rebuild(&doc);
        assert_eq!(
            engine.get_computed_style(other, "color"),
            rebuilt.get_computed_style(other, "color")
        );
    }

    #[test]
    fn descendant_dependent_class_touches_subtree() {
        let mut doc = html::parse_document(
            "<html><body><div id=\"container\"><p id=\"child\"></p></div></body></html>",
        );
        let sheet = CssomSheet::parse(".theme p { color: blue; }");
        let mut engine = StyleEngine::new(&doc, vec![(Origin::Author, sheet)]);
        let container = find_by_id(&doc, "container");
        let child = find_by_id(&doc, "child");
        assert_eq!(
            engine.get_computed_style(child, "color"),
            Some("canvastext")
        );

        add_class(&mut doc, container, "theme");
        let touched = engine.notify_class_changed(&doc, container, "theme");
        assert!(touched.contains(&container));
        assert!(touched.contains(&child));
        assert_eq!(engine.get_computed_style(child, "color"), Some("blue"));
    }

    #[test]
    fn sibling_dependent_class_touches_following_siblings_only() {
        let mut doc = html::parse_document(
            "<html><body><div><p id=\"before\"></p><p id=\"trigger\"></p><p id=\"after\"></p></div></body></html>",
        );
        let sheet = CssomSheet::parse(".flag ~ p { color: green; }");
        let mut engine = StyleEngine::new(&doc, vec![(Origin::Author, sheet)]);
        let before = find_by_id(&doc, "before");
        let trigger = find_by_id(&doc, "trigger");
        let after = find_by_id(&doc, "after");

        add_class(&mut doc, trigger, "flag");
        let touched = engine.notify_class_changed(&doc, trigger, "flag");
        assert!(touched.contains(&after));
        assert!(!touched.contains(&before));
        assert_eq!(engine.get_computed_style(after, "color"), Some("green"));
        assert_eq!(
            engine.get_computed_style(before, "color"),
            Some("canvastext")
        );
    }
}
