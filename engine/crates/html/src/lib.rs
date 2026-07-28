//! Roadmap phase: Track A2 (tokenizer, done) / A3 (tree construction, done
//! for the common HTML-content case; see `tree_builder.rs` module docs for
//! documented gaps -- foreign content, fragment parsing, framesets).
//!
//! Reference: <https://html.spec.whatwg.org/multipage/parsing.html#tokenization>

pub mod entities;
pub mod token;
pub mod tokenizer;
pub mod tree_builder;

pub use token::Token;
pub use tokenizer::{tokenize, tokenize_from, tokenize_with, Tokenizer, TokenizerState};
pub use tree_builder::{build_tree, parse_document};
