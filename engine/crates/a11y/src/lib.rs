//! Roadmap phase: Track F2 (accessibility tree).
//!
//! Per ROADMAP.md's reference-architecture note on F2: both Blink and Gecko
//! build this as a genuinely separate, incrementally-updated tree derived
//! from DOM/style/layout via notification hooks, not something computed on
//! demand at query time -- and it should be scoped from the start as
//! per-process + centrally stitched once Track E3's multi-process work
//! lands. Placeholder until DOM (C1) and layout (B1+) exist to derive from.

use dom::Document;

#[derive(Debug, Default)]
pub struct AccessibilityTree;

pub fn build_accessibility_tree(_doc: &Document) -> AccessibilityTree {
    AccessibilityTree
}
