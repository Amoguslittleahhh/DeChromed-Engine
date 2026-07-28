//! Roadmap phase: Track F1 (DevTools protocol & inspector).
//!
//! ROADMAP.md flags a real, undecided fork here: implement something
//! CDP-compatible (broad existing tool-ecosystem compatibility) vs. an
//! RDP-style actor-model protocol (cleaner internal architecture). Not
//! decided yet -- placeholder until there's a DOM tree (C1) worth
//! inspecting at all.

use dom::Document;

pub fn describe(_doc: &Document) -> String {
    "devtools: not yet implemented".to_string()
}
