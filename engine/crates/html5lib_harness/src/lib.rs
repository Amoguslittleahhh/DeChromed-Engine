//! Shared library half of this crate: the tokenizer harness (A2) lives in
//! `main.rs` as its own binary; the tree-construction harness (A3) lives
//! here as a module so `src/bin/tree_construction_harness.rs` can use it.

pub mod tree_construction;
