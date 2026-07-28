//! Roadmap phase: Track A4 (CSS tokenizer/parser, done) through A7 (CSSOM &
//! style invalidation, still ahead).
//!
//! `tokenizer.rs` and `parser.rs` implement the real CSS Syntax Module
//! Level 3 algorithms. `parse_stylesheet()` here is the public entry point
//! A5 (selectors) and A6 (cascade) build on: it flattens qualified
//! (style) rules -- including ones found inside conditional-group at-rules
//! like `@media`/`@supports`, since we don't evaluate their conditions yet
//! (a documented simplification, not a correctness claim) -- into a flat
//! rule list, and separately records other at-rules' raw name/prelude/block
//! text for whichever later phase needs them (e.g. `@font-face`, `@import`).
//!
//! Reference: <https://www.w3.org/TR/css-syntax-3/>

pub mod parser;
pub mod selectors;
pub mod tokenizer;

use parser::{ComponentValue, Parser, Rule as ParsedRule};

#[derive(Debug, Clone, Default)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
    /// At-rules that aren't (yet) expanded into ordinary rules -- e.g.
    /// `@font-face`, `@import`, `@keyframes`, `@page`. Each entry keeps the
    /// at-rule's name and its prelude/block re-serialized back to text,
    /// since we don't have per-at-rule semantic parsing yet.
    pub other_at_rules: Vec<AtRuleSummary>,
}

#[derive(Debug, Clone)]
pub struct AtRuleSummary {
    pub name: String,
    pub prelude: String,
    pub block: Option<String>,
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

/// At-rules whose block is a nested rule list (conditional group rules,
/// per <https://www.w3.org/TR/css-conditional-3/>) rather than a
/// declaration list. Their inner qualified rules get flattened into the
/// top-level `Stylesheet::rules` list rather than tracked separately,
/// since we don't evaluate the condition (media query / supports test) --
/// treating it as always-true is the honest way to say "not implemented"
/// without silently dropping content a real cascade might need.
const RULE_LIST_AT_RULES: &[&str] = &["media", "supports", "document", "layer"];

pub fn parse_stylesheet(input: &str) -> Stylesheet {
    let mut parser = Parser::new(input);
    let rules = parser.consume_rules(true);
    let mut sheet = Stylesheet::default();
    collect_rules(rules, &mut sheet);
    sheet
}

fn collect_rules(rules: Vec<ParsedRule>, sheet: &mut Stylesheet) {
    for rule in rules {
        match rule {
            ParsedRule::Qualified(q) => {
                let selector = parser::serialize(&q.prelude);
                let declarations = Parser::parse_declaration_list(&q.block);
                sheet.rules.push(Rule {
                    selector,
                    declarations,
                });
            }
            ParsedRule::At(at) => {
                if RULE_LIST_AT_RULES.contains(&at.name.to_ascii_lowercase().as_str()) {
                    if let Some(block) = &at.block {
                        let mut inner_parser = TokensAsParser::new(block.clone());
                        let inner_rules = inner_parser.consume_rules(false);
                        collect_rules(inner_rules, sheet);
                    }
                } else {
                    sheet.other_at_rules.push(AtRuleSummary {
                        name: at.name,
                        prelude: parser::serialize(&at.prelude),
                        block: at.block.as_ref().map(|b| parser::serialize(b)),
                    });
                }
            }
        }
    }
}

/// A tiny adapter so `consume_rules` (which parses from a flat `Token`
/// stream) can be re-run over an at-rule's already-parsed block contents
/// (a `Vec<ComponentValue>`) by flattening block markers back into bracket
/// tokens -- avoids a second parser implementation just for this.
struct TokensAsParser(Parser);

impl TokensAsParser {
    fn new(block: Vec<ComponentValue>) -> Self {
        let text = parser::serialize(&block);
        TokensAsParser(Parser::new(&text))
    }

    fn consume_rules(&mut self, top_level: bool) -> Vec<ParsedRule> {
        self.0.consume_rules(top_level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_rule() {
        let sheet = parse_stylesheet("p { color: red; }");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].selector, "p");
        assert_eq!(sheet.rules[0].declarations.len(), 1);
        assert_eq!(sheet.rules[0].declarations[0].property, "color");
        assert_eq!(sheet.rules[0].declarations[0].value, "red");
        assert!(!sheet.rules[0].declarations[0].important);
    }

    #[test]
    fn multiple_declarations_and_important() {
        let sheet = parse_stylesheet("div { color: red; margin: 0 !important; }");
        let decls = &sheet.rules[0].declarations;
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[1].property, "margin");
        assert_eq!(decls[1].value, "0");
        assert!(decls[1].important);
    }

    #[test]
    fn multiple_selectors_and_rules() {
        let sheet = parse_stylesheet("a, b { x: 1; } .c#d { y: 2; }");
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.rules[0].selector, "a, b");
        assert_eq!(sheet.rules[1].selector, ".c#d");
    }

    #[test]
    fn media_query_rules_are_flattened_in() {
        let sheet = parse_stylesheet("@media screen { p { color: blue; } }");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].selector, "p");
        assert_eq!(sheet.rules[0].declarations[0].value, "blue");
    }

    #[test]
    fn other_at_rules_are_recorded_not_expanded() {
        let sheet = parse_stylesheet("@import url(foo.css); p { x: 1; }");
        assert_eq!(sheet.other_at_rules.len(), 1);
        assert_eq!(sheet.other_at_rules[0].name, "import");
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn malformed_rule_recovers() {
        // A rule with no closing brace: parse error, but shouldn't crash
        // or poison rules parsed before it -- the spec has "consume a
        // simple block" stop at EOF too, so this becomes a second,
        // empty-bodied rule rather than being dropped or panicking.
        let sheet = parse_stylesheet("p { color: red; } div {");
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.rules[0].selector, "p");
        assert_eq!(sheet.rules[1].selector, "div");
        assert!(sheet.rules[1].declarations.is_empty());
    }
}
