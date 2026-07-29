//! Roadmap phase: Track A2 (tokenizer, done), A3 (tree construction, done
//! for the common HTML-content case), A8/A9 (SVG/MathML foreign content,
//! done -- see `tree_builder.rs` and `foreign_content.rs` module docs for
//! what's implemented and what's not).
//!
//! Reference: <https://html.spec.whatwg.org/multipage/parsing.html#tokenization>

pub mod entities;
pub mod foreign_content;
pub mod token;
pub mod tokenizer;
pub mod tree_builder;

pub use token::Token;
pub use tokenizer::{tokenize, tokenize_from, tokenize_with, Tokenizer, TokenizerState};
pub use tree_builder::{build_tree, parse_document};
