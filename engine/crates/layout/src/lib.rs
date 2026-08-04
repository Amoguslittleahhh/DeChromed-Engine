//! B1 through B8 (box tree generation through writing modes) are started
//! here -- see `box_tree.rs` and `flow.rs` for what's real and what's a
//! documented gap in each. `query.rs` starts B9's "fragment tree" half
//! (`getBoundingClientRect`/`elementFromPoint`, implemented by reading
//! the tree directly rather than re-deriving geometry); B9's "display
//! list" half lives in the `paint` crate, which lowers this crate's
//! `Fragment` tree into real drawing commands. See `ROADMAP.md`'s
//! reference-architecture section on B1/B2/B9 for why the fragment tree
//! here is an immutable per-pass output (LayoutNG-style) rather than a
//! mutable persistent frame tree (Gecko's older reflow model), a
//! principle `Fragment` already follows and `query.rs` depends on (no
//! mutation to keep in sync with a query).

pub mod box_tree;
pub mod flow;
pub mod query;
pub mod values;

pub use box_tree::{
    BoxKind, BoxLevel, ComputedStyle, Display, LayoutBox, StyleMap, build_box_tree,
};
pub use flow::{EdgeSizes, Fragment, Rect, layout};
pub use query::{bounding_client_rect, element_from_point};
