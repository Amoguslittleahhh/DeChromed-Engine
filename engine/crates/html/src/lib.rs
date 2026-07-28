//! Roadmap phase: Track A2 (tokenizer) / A3 (tree construction).
//!
//! A2's real WHATWG tokenizer state machine lives in `tokenizer.rs` (see its
//! module docs for exactly what's implemented vs. still a known gap). A3's
//! tree construction (`tree_builder.rs`) is still a placeholder.
//!
//! Reference: <https://html.spec.whatwg.org/multipage/parsing.html#tokenization>

pub mod entities;
pub mod token;
pub mod tokenizer;
pub mod tree_builder;

pub use token::Token;
pub use tokenizer::{tokenize, tokenize_from, tokenize_with, TokenizerState};
pub use tree_builder::build_tree;
