//! Roadmap phase: Track B1 (box tree) through B9 (fragment tree & display
//! list). Everything here is a placeholder shape until Track A (parsing +
//! style) is far enough along to have something real to lay out.
//!
//! See ROADMAP.md's reference-architecture section on B1/B2/B9 for why the
//! eventual `FragmentTree` here should be an immutable per-pass output
//! (LayoutNG-style) rather than a mutable persistent frame tree (Gecko's
//! older reflow model) -- worth deciding before this crate grows past a
//! placeholder.

/// Placeholder for B9's real output type: a tree of positioned, sized boxes
/// referencing their originating DOM/style nodes.
#[derive(Debug, Default)]
pub struct FragmentTree;

/// TODO(B1+): takes a styled tree and produces a fragment tree. Not
/// implemented until Track A produces a styled tree to consume.
pub fn layout() -> FragmentTree {
    FragmentTree
}
