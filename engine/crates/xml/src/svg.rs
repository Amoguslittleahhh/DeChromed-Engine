//! A8 wrap-up: standalone SVG documents (as opposed to `<svg>` inline in
//! HTML, which is `html::tree_builder`'s foreign-content job -- see that
//! crate's module docs). SVG is XML, so this is a thin layer over A10's
//! general-purpose XML parser, not a separate parser.
//!
//! Real standalone `.svg` files conventionally declare their own
//! `xmlns="http://www.w3.org/2000/svg"` on the root element, in which case
//! [`xml::parse_document`] alone already resolves every element's
//! namespace correctly with no SVG-specific help. This module's only
//! addition is defaulting to [`dom::SVG_NS`] for elements that end up with
//! no resolved namespace at all (an empty string) -- accommodating the
//! common case of a hand-authored or tool-generated SVG fragment that
//! omits the `xmlns` declaration entirely, still unambiguous in a file a
//! caller has already decided to parse specifically *as* SVG.
//!
//! Known gap: no SVG-specific semantics beyond namespace defaulting --
//! `<use>`/`<symbol>` reference resolution, geometry computation, and
//! rendering are all Track B/paint concerns that don't exist yet (see
//! `ROADMAP.md`'s A8 entry). A handful of SVG presentation properties
//! (`fill`, `stroke`, ...) are recognized by `css::cascade`'s property
//! table so they at least cascade/inherit correctly once a stylesheet
//! matches an SVG element, but nothing consumes their values for painting.

use crate::XmlError;
use dom::{Document, NodeData, NodeId};

/// Parses `input` as a standalone SVG document: ordinary XML parsing via
/// [`crate::parse_document`], then default any element left with no
/// resolved namespace to [`dom::SVG_NS`] (see module docs).
pub fn parse_svg_document(input: &str) -> Result<Document, XmlError> {
    let mut doc = crate::parse_document(input)?;
    let root = doc.root();
    default_unresolved_namespaces(&mut doc, root);
    Ok(doc)
}

fn default_unresolved_namespaces(doc: &mut Document, id: NodeId) {
    let children = doc.children(id).to_vec();
    if let NodeData::Element(el) = doc.data_mut(id) {
        if el.namespace.is_empty() {
            el.namespace = dom::SVG_NS.to_string();
        }
    }
    for child in children {
        default_unresolved_namespaces(doc, child);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_xmlns_is_respected() {
        let doc =
            parse_svg_document("<svg xmlns=\"http://www.w3.org/2000/svg\"><rect/></svg>").unwrap();
        let svg = doc.children(doc.root())[0];
        let NodeData::Element(el) = doc.data(svg) else {
            panic!()
        };
        assert_eq!(el.namespace, dom::SVG_NS);
    }

    #[test]
    fn missing_xmlns_defaults_to_svg_namespace() {
        let doc = parse_svg_document("<svg><rect/><g><circle/></g></svg>").unwrap();
        let svg = doc.children(doc.root())[0];
        let NodeData::Element(el) = doc.data(svg) else {
            panic!()
        };
        assert_eq!(el.namespace, dom::SVG_NS);
        let g = doc.children(svg)[1];
        let circle = doc.children(g)[0];
        let NodeData::Element(circle_el) = doc.data(circle) else {
            panic!()
        };
        assert_eq!(circle_el.namespace, dom::SVG_NS);
    }
}
