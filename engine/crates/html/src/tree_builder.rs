//! A3: HTML tree construction (the insertion-mode state machine, adoption
//! agency algorithm, implicit tag closing, foster parenting, etc).
//!
//! `build_tree()` is a **placeholder**: it just appends whatever tokens it's
//! given as flat children of the document root, with no insertion modes, no
//! implicit head/body insertion, and no error recovery. That's obviously not
//! spec-conforming -- it exists so the html5lib-tests tree-construction
//! harness has a real (if near-always-wrong) tree to diff against, rather
//! than nothing running at all.
//!
//! Reference while implementing this for real:
//! <https://html.spec.whatwg.org/multipage/parsing.html#tree-construction>

use crate::Token;
use dom::{Document, ElementData, NodeData};

pub fn build_tree(tokens: &[Token]) -> Document {
    let mut doc = Document::new();
    let root = doc.root();

    // TODO(A3): replace with the real insertion-mode state machine:
    // "initial" -> "before html" -> "before head" -> "in head" -> "in body"
    // -> ..., plus the adoption agency algorithm for mismatched tags.
    for token in tokens {
        match token {
            Token::StartTag {
                name, attributes, ..
            } => {
                doc.append(
                    root,
                    NodeData::Element(ElementData {
                        local_name: name.clone(),
                        attributes: attributes.clone(),
                    }),
                );
            }
            Token::Character(c) => {
                doc.append(root, NodeData::Text(c.to_string()));
            }
            Token::Comment(c) => {
                doc.append(root, NodeData::Comment(c.clone()));
            }
            Token::EndTag { .. } | Token::Doctype { .. } | Token::Eof => {
                // Not handled by the placeholder; A3 needs the open-elements
                // stack to know what an end tag even closes.
            }
        }
    }

    doc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_appends_flat_children() {
        let tokens = vec![
            Token::StartTag {
                name: "p".into(),
                attributes: vec![],
                self_closing: false,
            },
            Token::Character('h'),
            Token::Eof,
        ];
        let doc = build_tree(&tokens);
        assert_eq!(doc.children(doc.root()).len(), 2);
    }
}
