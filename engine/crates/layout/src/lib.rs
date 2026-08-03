//! B1 (box tree generation) and B2 (block & inline formatting contexts)
//! are started here -- see `box_tree.rs` and `flow.rs` for what's real and
//! what's a documented gap in each. B3 (tables) through B9 (fragment tree
//! & display list) are still ahead; see `ROADMAP.md`'s reference-
//! architecture section on B1/B2/B9 for why the eventual full fragment
//! tree here should stay an immutable per-pass output (LayoutNG-style)
//! rather than a mutable persistent frame tree (Gecko's older reflow
//! model), a principle this module's `Fragment` already follows.

pub mod box_tree;
pub mod flow;
pub mod values;

pub use box_tree::{
    BoxKind, BoxLevel, ComputedStyle, Display, LayoutBox, StyleMap, build_box_tree,
};
pub use flow::{EdgeSizes, Fragment, Rect, layout};
