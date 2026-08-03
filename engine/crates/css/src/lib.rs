//! Roadmap phase: Track A4-A7 (CSS tokenizer/parser through CSSOM & style
//! invalidation) -- all done, see each module's own doc comment for what
//! "done" means and its documented gaps. Also implements two features that
//! shipped in every major browser well after A4-A7 first landed: the
//! [CSS Nesting Module](https://www.w3.org/TR/css-nesting-1/) (native
//! `&`-relative nested rules, resolved here rather than requiring an
//! author-side preprocessor) and real
//! [cascade layers](https://www.w3.org/TR/css-cascade-5/#layering)
//! (`@layer`, tracked via [`Rule::layer`]/[`Stylesheet::layer_order`] and
//! given real cascade priority in `cascade.rs`, rather than the "flatten
//! and pretend unlayered" treatment `@media`/`@supports` still get).
//!
//! `tokenizer.rs` and `parser.rs` implement the real CSS Syntax Module
//! Level 3 algorithms. `parse_stylesheet()` here is the public entry point
//! A5 (selectors) and A6 (cascade) build on: it flattens qualified
//! (style) rules -- including ones found inside conditional-group at-rules
//! like `@media`/`@supports`, since we don't evaluate their conditions yet
//! (a documented simplification, not a correctness claim), and ones nested
//! inside another style rule (CSS Nesting, resolved via `&`-substitution --
//! see [`resolve_nested_selector`]) -- into a flat rule list tagged with
//! its cascade layer (if any), and separately records other at-rules' raw
//! name/prelude/block text for whichever later phase needs them (e.g.
//! `@font-face`, `@import`). `selectors.rs` (A5) matches that against a
//! DOM tree. `cascade.rs` (A6) resolves competing declarations (including
//! layer priority) and computes styles. `cssom.rs`/`style_engine.rs` (A7)
//! wrap all of the above in a mutable object graph with incremental,
//! invalidation-set-driven restyling.
//!
//! **Known gap:** nested layers (`@layer a { @layer b { ... } }`) are
//! ordered by their full dotted path's (`"a.b"`) first appearance in one
//! flat list, not the spec's real per-component path comparison. This is
//! exactly right for the dominant real-world pattern (a single flat list
//! of top-level layers, e.g. `@layer reset, base, utilities;`) and
//! reasonable for simple nesting, but can diverge from spec for elaborate
//! cross-nested layer trees -- see [`Stylesheet::layer_order`].
//!
//! Reference: <https://www.w3.org/TR/css-syntax-3/>, <https://www.w3.org/TR/css-nesting-1/>,
//! <https://www.w3.org/TR/css-cascade-5/>

pub mod cascade;
pub mod cssom;
pub mod parser;
pub mod selectors;
pub mod style_engine;
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
    /// [Cascade layer](https://www.w3.org/TR/css-cascade-5/#layering) names
    /// (full dotted paths for nested layers, e.g. `"outer.inner"`), in
    /// first-declaration order -- an `@layer name;` statement registers a
    /// name's position even before any `@layer name { ... }` block gives
    /// it rules. `css::cascade` uses this order directly as cascade
    /// priority among same-origin, same-importance declarations. See
    /// [`Rule::layer`] and the module docs' nesting-comparison gap note.
    pub layer_order: Vec<String>,
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
    /// The cascade layer this rule belongs to (its full dotted path), or
    /// `None` for the (author-origin) "unlayered" style all ordinary
    /// rules previously belonged to implicitly. See
    /// [`Stylesheet::layer_order`].
    pub layer: Option<String>,
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
/// without silently dropping content a real cascade might need. `@layer`
/// is deliberately *not* here -- it needs its own name-tracking handling
/// in `collect_rules`, not "flatten and pretend unlayered".
const RULE_LIST_AT_RULES: &[&str] = &["media", "supports", "document"];

pub fn parse_stylesheet(input: &str) -> Stylesheet {
    let mut parser = Parser::new(input);
    let rules = parser.consume_rules(true);
    let mut sheet = Stylesheet::default();
    let mut anon_layer_counter = 0u32;
    collect_rules(rules, None, None, &mut anon_layer_counter, &mut sheet);
    sheet
}

/// Registers `path` in `sheet.layer_order` if it hasn't been seen before --
/// first declaration (whether by an `@layer name;` statement or an
/// `@layer name { ... }` block) fixes a layer's cascade position, and
/// later re-declarations of the same name just add more rules to it
/// without moving it.
fn register_layer(sheet: &mut Stylesheet, path: &str) {
    if !sheet.layer_order.iter().any(|l| l == path) {
        sheet.layer_order.push(path.to_string());
    }
}

fn layer_path(current_layer: Option<&str>, name: &str) -> String {
    match current_layer {
        Some(parent) => format!("{parent}.{name}"),
        None => name.to_string(),
    }
}

/// `parent_selector` is `None` at the stylesheet's top level and `Some` once
/// we're inside a style rule's block (CSS Nesting) -- it's what a nested
/// rule's `&` (or implicit-descendant, if it has no `&` at all) resolves
/// against, and what a nested at-rule's own *bare* declarations (not
/// further-nested rules) implicitly belong to. `current_layer` is the
/// enclosing `@layer`'s full dotted path, if any. `anon_layer_counter`
/// names anonymous `@layer { ... }` blocks uniquely within one
/// `parse_stylesheet` call (each is its own layer, per spec, even if two
/// anonymous blocks appear at the same nesting position).
fn collect_rules(
    rules: Vec<ParsedRule>,
    parent_selector: Option<&str>,
    current_layer: Option<&str>,
    anon_layer_counter: &mut u32,
    sheet: &mut Stylesheet,
) {
    for rule in rules {
        match rule {
            ParsedRule::Qualified(q) => {
                let resolved_selector = match parent_selector {
                    None => parser::serialize(&q.prelude),
                    Some(parent) => resolve_nested_selector(parent, &q.prelude),
                };
                // `q.block` moves into `parse_style_block` (not borrowed)
                // -- see that function's docs on why cloning it here would
                // be quadratic in nesting depth.
                let (declarations, nested_rules) = Parser::parse_style_block(q.block);
                sheet.rules.push(Rule {
                    selector: resolved_selector.clone(),
                    declarations,
                    layer: current_layer.map(str::to_string),
                });
                collect_rules(
                    nested_rules,
                    Some(&resolved_selector),
                    current_layer,
                    anon_layer_counter,
                    sheet,
                );
            }
            ParsedRule::At(at) if at.name.eq_ignore_ascii_case("layer") => {
                match at.block {
                    // `@layer name { ... }` / `@layer { ... }`: a block
                    // form, giving the (possibly-anonymous) layer rules.
                    Some(block) => {
                        let own_name = parser::serialize(&at.prelude);
                        let own_name = if own_name.is_empty() {
                            *anon_layer_counter += 1;
                            format!("<anonymous-{anon_layer_counter}>")
                        } else {
                            own_name
                        };
                        let path = layer_path(current_layer, &own_name);
                        register_layer(sheet, &path);
                        match parent_selector {
                            Some(parent) => {
                                let (declarations, nested_rules) = Parser::parse_style_block(block);
                                if !declarations.is_empty() {
                                    sheet.rules.push(Rule {
                                        selector: parent.to_string(),
                                        declarations,
                                        layer: Some(path.clone()),
                                    });
                                }
                                collect_rules(
                                    nested_rules,
                                    Some(parent),
                                    Some(&path),
                                    anon_layer_counter,
                                    sheet,
                                );
                            }
                            None => {
                                // Not re-serialized back to text and
                                // re-parsed -- see `parse_style_block`'s
                                // docs; the same quadratic-in-depth (here,
                                // actually worse: re-tokenizing text at
                                // every nesting level is its own extra
                                // O(depth) cost per level) trap applies to
                                // deeply nested at-rules just as much as
                                // deeply nested style rules, and doing so
                                // also silently defeated the parser's
                                // depth guard by resetting it on every
                                // fresh re-parse. `.1` discards any bare
                                // declarations directly inside this block
                                // (a parse error at rule-list level, same
                                // as the old text-round-trip path silently
                                // dropped them too).
                                let (_, inner_rules) = Parser::parse_style_block(block);
                                collect_rules(
                                    inner_rules,
                                    None,
                                    Some(&path),
                                    anon_layer_counter,
                                    sheet,
                                );
                            }
                        }
                    }
                    // `@layer name1, name2;`: a statement form, just
                    // registering names' cascade order with no rules yet.
                    // Per spec this can't declare an anonymous layer (no
                    // block to give it), so an empty prelude is ignored.
                    None => {
                        for part in parser::split_top_level_commas(&at.prelude) {
                            let name = parser::serialize(&part);
                            if !name.is_empty() {
                                register_layer(sheet, &layer_path(current_layer, &name));
                            }
                        }
                    }
                }
            }
            ParsedRule::At(at) => {
                if RULE_LIST_AT_RULES.contains(&at.name.to_ascii_lowercase().as_str()) {
                    if let Some(block) = at.block {
                        match parent_selector {
                            // Nested inside a style rule: the at-rule's
                            // block can mix bare declarations (which apply
                            // to the enclosing selector, as if wrapped in
                            // `& { ... }`) with further-nested rules --
                            // see `Parser::parse_style_block`'s docs.
                            Some(parent) => {
                                let (declarations, nested_rules) = Parser::parse_style_block(block);
                                if !declarations.is_empty() {
                                    sheet.rules.push(Rule {
                                        selector: parent.to_string(),
                                        declarations,
                                        layer: current_layer.map(str::to_string),
                                    });
                                }
                                collect_rules(
                                    nested_rules,
                                    Some(parent),
                                    current_layer,
                                    anon_layer_counter,
                                    sheet,
                                );
                            }
                            None => {
                                // See the `@layer` branch's identical note
                                // above on why this moves through
                                // `parse_style_block` instead of
                                // re-serializing to text and re-parsing.
                                let (_, inner_rules) = Parser::parse_style_block(block);
                                collect_rules(
                                    inner_rules,
                                    None,
                                    current_layer,
                                    anon_layer_counter,
                                    sheet,
                                );
                            }
                        }
                    }
                } else {
                    sheet.other_at_rules.push(AtRuleSummary {
                        prelude: parser::serialize(&at.prelude),
                        block: at.block.as_ref().map(|b| parser::serialize(b)),
                        name: at.name,
                    });
                }
            }
        }
    }
}

/// Resolves a nested rule's selector prelude against its `parent` selector
/// text, per <https://www.w3.org/TR/css-nesting-1/#nest-selector>: each
/// comma-separated complex selector in the nested prelude gets every `&`
/// replaced with `:is(parent)` (wrapping the parent selector in `:is()`
/// reproduces both its exact match set *and* its specificity as a whole,
/// which is precisely what `&` means); a complex selector with no `&` at
/// all is treated as if `&` were implicitly prepended (directly, with no
/// space, if it already starts with a combinator like `>`/`+`/`~` -- e.g.
/// `> .child` under `.a` means `.a > .child`, not a descendant of it).
fn resolve_nested_selector(parent: &str, nested_prelude: &[ComponentValue]) -> String {
    let wrapped_parent = format!(":is({parent})");
    parser::split_top_level_commas(nested_prelude)
        .iter()
        .map(|part| {
            let text = parser::serialize(part);
            if text.contains('&') {
                text.replace('&', &wrapped_parent)
            } else if text.starts_with(['>', '+', '~']) {
                format!("{wrapped_parent}{text}")
            } else {
                format!("{wrapped_parent} {text}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
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

    #[test]
    fn nesting_explicit_ampersand() {
        let sheet = parse_stylesheet(".a { color: red; & .b { color: blue; } }");
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.rules[0].selector, ".a");
        assert_eq!(sheet.rules[1].selector, ":is(.a) .b");
        assert_eq!(sheet.rules[1].declarations[0].value, "blue");
    }

    #[test]
    fn nesting_implicit_descendant_without_ampersand() {
        // No `&` at all: still nests, as if `& ` were prepended -- this is
        // the finalized CSS Nesting behavior real browsers ship (an
        // earlier draft required an explicit `&`).
        let sheet = parse_stylesheet(".a { .b { color: blue; } }");
        assert_eq!(sheet.rules[1].selector, ":is(.a) .b");
    }

    #[test]
    fn nesting_combinator_prepends_directly_no_space() {
        // `> .b` under `.a` means `.a > .b` (a child of `.a`), not a
        // descendant of `.a > .b` -- the combinator attaches directly.
        let sheet = parse_stylesheet(".a { > .b { color: green; } }");
        assert_eq!(sheet.rules[1].selector, ":is(.a)> .b");
    }

    #[test]
    fn nesting_comma_separated_nested_selector() {
        let sheet = parse_stylesheet(".a { &:hover, &:focus { color: purple; } }");
        assert_eq!(sheet.rules[1].selector, ":is(.a):hover, :is(.a):focus");
    }

    #[test]
    fn nesting_comma_separated_parent_selector() {
        let sheet = parse_stylesheet(".a, .b { & .c { color: orange; } }");
        assert_eq!(sheet.rules[1].selector, ":is(.a, .b) .c");
    }

    #[test]
    fn nesting_inside_nested_media_query() {
        // Bare declarations directly inside a nested @media apply to the
        // enclosing selector (as if wrapped in `& { ... }`); a further-
        // nested rule inside that @media still resolves against the same
        // enclosing selector.
        let sheet = parse_stylesheet(
            ".a { @media (min-width: 1px) { color: pink; &:hover { color: teal; } } }",
        );
        assert_eq!(sheet.rules.len(), 3);
        assert_eq!(sheet.rules[1].selector, ".a");
        assert_eq!(sheet.rules[1].declarations[0].value, "pink");
        assert_eq!(sheet.rules[2].selector, ":is(.a):hover");
        assert_eq!(sheet.rules[2].declarations[0].value, "teal");
    }

    #[test]
    fn nesting_resolved_selector_actually_matches() {
        // End-to-end: the resolved `:is(...)`-based selector text must be
        // real, matchable Selectors Level 4 syntax, not just plausible text.
        let doc =
            html::parse_document("<body><div class=\"a\"><span class=\"b\">x</span></div></body>");
        let sheet = parse_stylesheet(".a { color: red; & .b { color: blue; } }");
        let nested = selectors::parse_selector_list(&sheet.rules[1].selector).unwrap();
        let mut matched = 0;
        doc.walk(doc.root(), &mut |id, _| {
            if selectors::matches(&doc, id, &nested) {
                matched += 1;
            }
        });
        assert_eq!(matched, 1);
    }

    #[test]
    fn deeply_nested_rules_resolve_in_linear_not_quadratic_time() {
        // Regression test for a real perf bug the nesting implementation
        // itself introduced: resolving each nesting level used to clone
        // the *entire remaining* (still deeply nested) block contents,
        // making total work quadratic in nesting depth -- 20,000 levels of
        // `.a{.a{...` took over 7 seconds. `parse_style_block` moving
        // (rather than cloning) each level's block fixed it; this asserts
        // a depth well past the parser's own 256-level structural cap
        // still resolves promptly rather than hanging.
        let depth = 20_000;
        let mut css = String::new();
        for _ in 0..depth {
            css.push_str(".a{");
        }
        css.push_str("color:red;");
        for _ in 0..depth {
            css.push('}');
        }
        let start = std::time::Instant::now();
        let sheet = parse_stylesheet(&css);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "took {:?}, expected well under 5s",
            start.elapsed()
        );
        // The component-value parser's own depth guard (parser.rs) caps
        // structural nesting at 256 levels regardless of input depth.
        assert!(sheet.rules.len() <= 257);
    }

    #[test]
    fn deeply_nested_at_rules_resolve_promptly_and_respect_the_depth_guard() {
        // Regression test for a worse pre-existing bug `deeply_nested_
        // rules_resolve_in_linear_not_quadratic_time`'s fix didn't cover:
        // nested at-rules (`@layer`/`@media`/...) used to be re-serialized
        // back to CSS text and re-parsed from scratch at every nesting
        // level (`TokensAsParser`), which (a) silently reset the parser's
        // depth guard on every fresh re-parse, since a new `Parser` starts
        // its depth counter over at 0, and (b) cost O(remaining text
        // length) extra work *per level* on top of that -- 3,000 levels of
        // nested `@layer` took nearly 2 seconds and the depth guard never
        // engaged at all (1,000 real distinct layers were created, not
        // capped at ~256). Fixed by processing the already-parsed
        // `ComponentValue` tree directly instead of round-tripping
        // through text.
        let depth = 20_000;
        let mut css = String::new();
        for i in 0..depth {
            css.push_str(&format!("@layer l{i}{{"));
        }
        css.push_str("color:red;");
        for _ in 0..depth {
            css.push('}');
        }
        let start = std::time::Instant::now();
        let sheet = parse_stylesheet(&css);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "took {:?}, expected well under 5s",
            start.elapsed()
        );
        assert!(sheet.layer_order.len() <= 260);
    }
}
