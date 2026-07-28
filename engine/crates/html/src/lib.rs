//! Roadmap phase: Track A2 (tokenizer) / A3 (tree construction).
//!
//! This crate is the actual target of A2/A3, not yet their implementation.
//! `TokenizerState` enumerates the real WHATWG tokenizer states so the shape
//! of the state machine is visible and reviewable before it's filled in --
//! but `tokenize()` itself is a placeholder that intentionally does not
//! implement the spec yet, so the html5lib-tests harness has something real
//! to report a (near-zero) pass rate against from day one, per A1's exit
//! criterion.
//!
//! Reference while implementing this for real:
//! <https://html.spec.whatwg.org/multipage/parsing.html#tokenization>

pub mod token;
pub mod tokenizer;
pub mod tree_builder;

pub use token::Token;
pub use tokenizer::{tokenize, TokenizerState};
pub use tree_builder::build_tree;
