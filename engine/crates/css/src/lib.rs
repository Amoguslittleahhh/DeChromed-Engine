//! Roadmap phase: Track A4 (CSS tokenizer/parser) through A7 (CSSOM &
//! style invalidation).
//!
//! Structure only for now: real types for the syntax tree a parser will
//! eventually produce, and a placeholder `parse_stylesheet()` so later
//! crates (layout) have a stable-shaped dependency to build against before
//! A4 actually implements the CSS Syntax Module tokenizer.
//!
//! Reference: <https://www.w3.org/TR/css-syntax-3/>

#[derive(Debug, Clone, Default)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub selector: String,
    pub declarations: Vec<Declaration>,
}

#[derive(Debug, Clone)]
pub struct Declaration {
    pub property: String,
    pub value: String,
    pub important: bool,
}

/// TODO(A4): replace with the real CSS Syntax Module tokenizer + parser.
/// Currently returns an empty stylesheet regardless of input.
pub fn parse_stylesheet(_input: &str) -> Stylesheet {
    Stylesheet::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_parses_to_nothing() {
        assert!(parse_stylesheet("p { color: red; }").rules.is_empty());
    }
}
