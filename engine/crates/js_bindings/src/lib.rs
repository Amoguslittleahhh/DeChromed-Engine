//! Roadmap phase: Track C (Script & Runtime).
//!
//! This crate's real shape depends on "The JS engine question" in
//! ROADMAP.md, which is deliberately still open: if the pragmatic path is
//! chosen this becomes bindings around an embedded V8/SpiderMonkey; if the
//! purist path is chosen, this becomes (or gains a sibling crate for) the
//! from-scratch parser/interpreter/GC/JIT of C3-C7. Named `js_bindings`
//! rather than `js` for now since bindings-around-an-embedded-engine is the
//! more likely near-term path per the roadmap's own recommendation, but
//! nothing here commits to that yet -- it's an empty placeholder either way
//! until that decision is made and C1 (DOM API) exists for it to bind to.

use dom::Document;

/// Placeholder entry point: eventually exposes the DOM (C1) to whichever JS
/// runtime C3-C8 settles on.
pub fn bind_document(_doc: &Document) {}
