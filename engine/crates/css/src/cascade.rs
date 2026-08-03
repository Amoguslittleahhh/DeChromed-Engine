//! A6: cascade & computed values.
//!
//! Reference: <https://www.w3.org/TR/css-cascade-4/>,
//! <https://www.w3.org/TR/css-variables-1/>.
//!
//! Implements the real cascade sort (origin, importance, layer, specificity,
//! source order per <https://www.w3.org/TR/css-cascade-4/#cascade-sort> and
//! <https://www.w3.org/TR/css-cascade-5/#layering>), full Selectors-Level-4
//! specificity computation (including `:is()`/`:where()`/`:not()`/`:has()`'s
//! special rules and `:nth-child(An+B of S)`), CSS custom properties
//! (`--foo`) with inheritance and `var()` substitution (including fallback
//! values and cycle detection per
//! <https://www.w3.org/TR/css-variables-1/#invalid-variables>), and the
//! `initial`/`inherit`/`unset`/`revert` defaulting keywords.
//!
//! **Cascade layers** (`@layer`, parsed in `lib.rs`): within the same
//! origin and importance, [`layer_rank`] gives unlayered normal-importance
//! declarations the highest priority, then layered ones ordered by
//! [`Stylesheet::layer_order`][crate::Stylesheet::layer_order] (later-
//! declared layer wins) -- and, for `!important` declarations, the exact
//! reverse (earlier-declared layer wins, unlayered loses to every layer),
//! which is the one place in the whole cascade where `!important` inverts
//! rather than just reprioritizes origins. See `lib.rs`'s module docs for
//! the nested-layer-ordering simplification this builds on.
//!
//! Known gaps, documented rather than silently wrong:
//! - **No used-value resolution.** "Computed value" here stops short of
//!   resolving relative units (`em`, `%`, ...) against layout, since that
//!   needs Track B (layout) to exist. What this module calls "computed"
//!   is the spec's computed value for the handful of keyword-like
//!   properties in [`PROPERTY_TABLE`], and the substituted-but-otherwise
//!   unparsed text for everything else -- genuinely correct value parsing
//!   per-property (`<length>`, `<color>`, shorthand expansion, ...) is
//!   its own large surface deferred to whichever layout/paint phase first
//!   needs a given property's real typed value.
//! - **Small property table.** [`PROPERTY_TABLE`] only knows a few dozen
//!   common longhands' inherited-ness and initial value, not the full CSS
//!   property registry (that's hundreds of properties across dozens of
//!   specs). Unknown properties default to non-inherited with an empty
//!   initial value -- an honest "don't know" rather than a guess.
//! - **`revert` is treated as `unset`.** A correct `revert` needs the
//!   full layered-origin cascade result (revert to the next lower origin's
//!   winning value), which requires re-running cascade per-origin; this
//!   collapses that to the inherit-or-initial behavior of `unset` instead.
//! - No animation/transition origins. Layer order is tracked per
//!   `Stylesheet`, not globally across every `StyleSource` passed to one
//!   cascade run -- see [`layer_rank`]'s docs.

use crate::Stylesheet;
use crate::selectors::{self, SelectorList};
use dom::{Document, NodeData, NodeId};
use std::collections::{HashMap, HashSet};

/// Per <https://www.w3.org/TR/selectors-4/#specificity-rules>: (id count,
/// class/attribute/pseudo-class count, type/pseudo-element count).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Specificity(pub u32, pub u32, pub u32);

impl Specificity {
    fn add(self, other: Specificity) -> Specificity {
        Specificity(self.0 + other.0, self.1 + other.1, self.2 + other.2)
    }
}

/// Which origin (<https://www.w3.org/TR/css-cascade-4/#origin>) a
/// stylesheet belongs to. Determines cascade priority together with
/// `!important`; see [`priority_bucket`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    UserAgent,
    User,
    Author,
}

/// A stylesheet plus the origin it cascades in. `sources` passed to
/// [`compute_document_styles`] should be given in the document's actual
/// stylesheet order (earlier entries lose ties within the same origin/
/// importance/specificity bucket, per source-order tie-breaking).
pub struct StyleSource<'a> {
    pub origin: Origin,
    pub sheet: &'a Stylesheet,
}

/// Cascade priority ordering, lowest to highest, per
/// <https://www.w3.org/TR/css-cascade-4/#cascade-origin>: normal-importance
/// origins from weakest to strongest, then `!important` origins in the
/// *reverse* order (user `!important` beats author `!important`, UA
/// `!important` beats both -- the one exception to "author wins" in the
/// whole cascade).
fn priority_bucket(origin: Origin, important: bool) -> u8 {
    match (origin, important) {
        (Origin::UserAgent, false) => 0,
        (Origin::User, false) => 1,
        (Origin::Author, false) => 2,
        (Origin::Author, true) => 3,
        (Origin::User, true) => 4,
        (Origin::UserAgent, true) => 5,
    }
}

struct Candidate {
    bucket: u8,
    layer_rank: u32,
    specificity: Specificity,
    order: u32,
    value: String,
}

/// A declaration's priority from its cascade layer, used as a tiebreaker
/// among declarations that already share the same [`priority_bucket`]
/// (same origin, same importance) -- higher wins, same as every other key
/// in the sort tuple `cascade()` builds.
///
/// Per <https://www.w3.org/TR/css-cascade-5/#layer-ordering>: for
/// *normal*-importance declarations, unlayered beats every layer, and
/// among layers, the one declared *later* (further along
/// [`Stylesheet::layer_order`][crate::Stylesheet::layer_order]) wins. For
/// `!important` declarations this inverts: unlayered loses to every
/// layer, and among layers the one declared *earlier* wins -- the single
/// spot in the whole cascade where `!important` doesn't just reprioritize
/// origins but flips a within-origin ordering too.
///
/// **Known gap:** `layer_order` is looked up on the declaration's own
/// `Stylesheet`, not merged across every `StyleSource` in one `cascade()`
/// call -- correct for the common case of one authored stylesheet (or
/// several treated as logically independent), not for two separate
/// `Stylesheet`s meant to interleave layers with each other.
fn layer_rank(rule_layer: Option<&str>, layer_order: &[String], important: bool) -> u32 {
    let layer_index = rule_layer.and_then(|name| layer_order.iter().position(|l| l == name));
    let layer_count = layer_order.len() as u32;
    match (layer_index, important) {
        (None, false) => layer_count + 1, // unlayered normal: highest
        (Some(i), false) => i as u32 + 1, // later-declared layer: higher
        (Some(i), true) => layer_count - i as u32, // earlier-declared layer: higher
        (None, true) => 0,                // unlayered important: lowest
    }
}

/// Runs the cascade for a single element against `sources`, returning the
/// winning (still-unsubstituted, still-un-defaulted) value text for every
/// property/custom-property that had at least one matching declaration.
/// Standard longhands and custom properties (`--foo`) are cascaded
/// identically -- the cascade algorithm itself doesn't distinguish them.
pub fn cascade(doc: &Document, node: NodeId, sources: &[StyleSource]) -> HashMap<String, String> {
    let mut candidates: HashMap<String, Vec<Candidate>> = HashMap::new();
    let mut order: u32 = 0;
    for source in sources {
        for rule in &source.sheet.rules {
            let Ok(list) = selectors::parse_selector_list(&rule.selector) else {
                continue;
            };
            let Some(specificity) = best_matching_specificity(doc, node, &list) else {
                continue;
            };
            for decl in &rule.declarations {
                order += 1;
                candidates
                    .entry(decl.property.clone())
                    .or_default()
                    .push(Candidate {
                        bucket: priority_bucket(source.origin, decl.important),
                        layer_rank: layer_rank(
                            rule.layer.as_deref(),
                            &source.sheet.layer_order,
                            decl.important,
                        ),
                        specificity,
                        order,
                        value: decl.value.clone(),
                    });
            }
        }
    }

    let mut winners = HashMap::new();
    for (property, mut cands) in candidates {
        cands.sort_by_key(|c| (c.bucket, c.layer_rank, c.specificity, c.order));
        if let Some(winner) = cands.pop() {
            winners.insert(property, winner.value);
        }
    }
    winners
}

/// If any complex selector in `list` matches `node`, returns the highest
/// specificity among the ones that do (a rule's selector list is shorthand
/// for one rule per comma-separated selector -- see module docs on
/// [`Rule`]/[`cascade`]).
fn best_matching_specificity(
    doc: &Document,
    node: NodeId,
    list: &SelectorList,
) -> Option<Specificity> {
    list.0
        .iter()
        .filter(|cs| selectors::matches_complex(doc, node, cs))
        .map(specificity_of_complex)
        .max()
}

fn specificity_of_complex(cs: &selectors::ComplexSelector) -> Specificity {
    cs.steps
        .iter()
        .map(|step| specificity_of_compound(&step.compound))
        .fold(Specificity::default(), Specificity::add)
}

fn specificity_of_compound(compound: &selectors::CompoundSelector) -> Specificity {
    let mut spec = match &compound.type_selector {
        Some(selectors::TypeSelector::Named(_)) => Specificity(0, 0, 1),
        _ => Specificity::default(),
    };
    for sub in &compound.subclasses {
        spec = spec.add(specificity_of_subclass(sub));
    }
    spec
}

fn specificity_of_subclass(sub: &selectors::SubclassSelector) -> Specificity {
    use selectors::{PseudoClass, SubclassSelector};
    match sub {
        SubclassSelector::Id(_) => Specificity(1, 0, 0),
        SubclassSelector::Class(_) | SubclassSelector::Attribute(_) => Specificity(0, 1, 0),
        SubclassSelector::PseudoElement(_) => Specificity(0, 0, 1),
        SubclassSelector::PseudoClass(pc) => match pc {
            // :where() is explicitly zero-specificity per spec.
            PseudoClass::Where(_) => Specificity::default(),
            // :is()/:not()/:has() take the specificity of their most
            // specific matching-eligible argument; approximated here as
            // the max over the whole argument list (matches spec for
            // :not()/:is(), a documented conservative approximation for
            // :has() since we don't track which relative selector inside
            // it actually matched).
            PseudoClass::Not(list) | PseudoClass::Is(list) | PseudoClass::Has(list) => list
                .0
                .iter()
                .map(specificity_of_complex)
                .max()
                .unwrap_or_default(),
            // :nth-child(An+B of S) adds the pseudo-class's own
            // specificity (like a class) plus S's specificity.
            PseudoClass::NthChild(_, Some(list)) | PseudoClass::NthLastChild(_, Some(list)) => {
                Specificity(0, 1, 0).add(
                    list.0
                        .iter()
                        .map(specificity_of_complex)
                        .max()
                        .unwrap_or_default(),
                )
            }
            _ => Specificity(0, 1, 0),
        },
    }
}

/// Inherited-ness and initial value for a small, honestly-scoped set of
/// common longhand properties. See module docs' "Small property table" gap.
const PROPERTY_TABLE: &[(&str, bool, &str)] = &[
    ("color", true, "canvastext"),
    ("font-family", true, "sans-serif"),
    ("font-size", true, "medium"),
    ("font-weight", true, "normal"),
    ("font-style", true, "normal"),
    ("line-height", true, "normal"),
    ("text-align", true, "start"),
    ("visibility", true, "visible"),
    ("white-space", true, "normal"),
    ("list-style-type", true, "disc"),
    ("cursor", true, "auto"),
    ("display", false, "inline"),
    ("position", false, "static"),
    ("float", false, "none"),
    ("clear", false, "none"),
    ("width", false, "auto"),
    ("height", false, "auto"),
    ("margin-top", false, "0"),
    ("margin-right", false, "0"),
    ("margin-bottom", false, "0"),
    ("margin-left", false, "0"),
    ("padding-top", false, "0"),
    ("padding-right", false, "0"),
    ("padding-bottom", false, "0"),
    ("padding-left", false, "0"),
    ("border-width", false, "medium"),
    ("border-style", false, "none"),
    ("border-color", false, "currentcolor"),
    ("background-color", false, "transparent"),
    ("opacity", false, "1"),
    ("z-index", false, "auto"),
    // A8: a handful of SVG presentation properties (inherited per
    // <https://www.w3.org/TR/SVG2/propidx.html>, unlike most CSS box
    // properties) -- enough for these to cascade/inherit correctly onto
    // `<svg>` subtree elements once a stylesheet matches them, though
    // nothing consumes their values for painting yet (Track B/paint).
    ("fill", true, "black"),
    ("stroke", true, "none"),
    ("stroke-width", true, "1"),
    ("fill-opacity", true, "1"),
    ("stroke-opacity", true, "1"),
    ("stroke-linecap", true, "butt"),
    ("stroke-linejoin", true, "miter"),
    ("stop-color", false, "black"),
    ("stop-opacity", false, "1"),
];

pub struct PropertyMeta {
    pub inherited: bool,
    pub initial: &'static str,
}

pub fn property_meta(name: &str) -> PropertyMeta {
    for (prop, inherited, initial) in PROPERTY_TABLE {
        if *prop == name {
            return PropertyMeta {
                inherited: *inherited,
                initial,
            };
        }
    }
    PropertyMeta {
        inherited: false,
        initial: "",
    }
}

pub fn is_custom_property(name: &str) -> bool {
    name.starts_with("--")
}

/// Substitutes `var(--name)` / `var(--name, fallback)` references in
/// `value` using `custom_props`, recursively (a custom property's value
/// may itself reference other custom properties). Returns `None` if a
/// reference doesn't resolve (unset property, no fallback) or if a cycle
/// is detected -- both cases make the value "invalid at computed-value
/// time" per spec, which callers treat as if the declaration didn't exist.
pub fn substitute_var(value: &str, custom_props: &HashMap<String, String>) -> Option<String> {
    let mut resolving = HashSet::new();
    substitute_var_inner(value, custom_props, &mut resolving)
}

fn substitute_var_inner(
    value: &str,
    custom_props: &HashMap<String, String>,
    resolving: &mut HashSet<String>,
) -> Option<String> {
    if !value.contains("var(") {
        return Some(value.to_string());
    }
    let mut out = String::new();
    let mut i = 0;
    while i < value.len() {
        if value[i..].starts_with("var(") {
            let args_start = i + 4;
            let close = find_matching_paren(value, args_start)?;
            let args = &value[args_start..close];
            let (name, fallback) = match args.split_once(',') {
                Some((n, f)) => (n.trim(), Some(f.trim())),
                None => (args.trim(), None),
            };
            let resolved = if let Some(v) = custom_props.get(name) {
                if !resolving.insert(name.to_string()) {
                    return None; // cycle
                }
                let r = substitute_var_inner(v, custom_props, resolving);
                resolving.remove(name);
                r
            } else {
                None
            };
            match resolved
                .or_else(|| fallback.and_then(|f| substitute_var_inner(f, custom_props, resolving)))
            {
                Some(s) => out.push_str(&s),
                None => return None,
            }
            i = close + 1;
        } else {
            // Found by fuzzing with multi-byte UTF-8 input: this used to
            // read `bytes[i] as char`, which reinterprets a single raw
            // UTF-8 *byte* as if it were already a full Unicode scalar
            // value -- wrong for anything outside ASCII, and it then
            // advances `i` by that bogus char's `len_utf8()` (not the real
            // character's), desynchronizing `i` from the string's actual
            // char boundaries. A later slice at the now-invalid `i` panics
            // with "byte index N is not a char boundary". Decoding the
            // real character at `i` (guaranteed to be a valid boundary
            // here) and advancing by its real length is always correct.
            let ch = value[i..].chars().next().expect("i < value.len()");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    Some(out)
}

fn find_matching_paren(s: &str, start: usize) -> Option<usize> {
    let mut depth = 1;
    for (offset, ch) in s[start..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + offset);
                }
            }
            _ => {}
        }
    }
    None
}

/// Computes the full style map (registered longhands + custom properties)
/// for `node`, given its cascaded winners and its parent's already-computed
/// style (`None` for the root). Applies `var()` substitution and the
/// `initial`/`inherit`/`unset`/`revert` defaulting keywords per
/// <https://www.w3.org/TR/css-cascade-4/#defaulting-keywords>.
pub fn compute_node_style(
    cascaded: &HashMap<String, String>,
    parent: Option<&HashMap<String, String>>,
) -> HashMap<String, String> {
    let empty = HashMap::new();
    let parent = parent.unwrap_or(&empty);

    // Custom properties are inherited by default and don't have a fixed
    // initial value (an unset one simply doesn't exist), so resolve them
    // first: they're the environment var() substitution runs against.
    let mut custom_props: HashMap<String, String> = parent
        .iter()
        .filter(|(k, _)| is_custom_property(k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for (name, value) in cascaded.iter().filter(|(k, _)| is_custom_property(k)) {
        match value.as_str() {
            "inherit" => {} // already seeded from parent above
            "initial" | "unset" => {
                custom_props.remove(name);
            }
            _ => {
                custom_props.insert(name.clone(), value.clone());
            }
        }
    }
    // Re-substitute var() inside custom property values themselves so a
    // property that references `--b` which references `--a` sees `--a`'s
    // final text, not `var(--a)` literally.
    let resolved_custom: HashMap<String, String> = custom_props
        .keys()
        .filter_map(|k| substitute_var(custom_props.get(k)?, &custom_props).map(|v| (k.clone(), v)))
        .collect();

    let mut style = HashMap::new();
    for (name, inherited, initial) in PROPERTY_TABLE {
        let meta_initial = *initial;
        let inherited_value = || {
            parent
                .get(*name)
                .cloned()
                .unwrap_or_else(|| meta_initial.to_string())
        };
        let value = match cascaded.get(*name) {
            None => {
                if *inherited {
                    inherited_value()
                } else {
                    meta_initial.to_string()
                }
            }
            Some(raw) => match raw.as_str() {
                "initial" => meta_initial.to_string(),
                "inherit" => inherited_value(),
                "unset" | "revert" => {
                    if *inherited {
                        inherited_value()
                    } else {
                        meta_initial.to_string()
                    }
                }
                _ => substitute_var(raw, &resolved_custom).unwrap_or_else(|| {
                    if *inherited {
                        inherited_value()
                    } else {
                        meta_initial.to_string()
                    }
                }),
            },
        };
        style.insert((*name).to_string(), value);
    }
    for (name, value) in &resolved_custom {
        style.insert(name.clone(), value.clone());
    }
    style
}

/// Convenience: cascades and computes styles for every element in `doc`,
/// walking top-down from the root so each node's parent style is already
/// known when it's computed (required for inheritance).
pub fn compute_document_styles(
    doc: &Document,
    sources: &[StyleSource],
) -> HashMap<NodeId, HashMap<String, String>> {
    let mut styles = HashMap::new();
    compute_subtree(doc, doc.root(), sources, None, &mut styles);
    styles
}

fn compute_subtree(
    doc: &Document,
    node: NodeId,
    sources: &[StyleSource],
    parent_style: Option<&HashMap<String, String>>,
    out: &mut HashMap<NodeId, HashMap<String, String>>,
) {
    let this_style = if matches!(doc.data(node), NodeData::Element(_)) {
        let cascaded = cascade(doc, node, sources);
        Some(compute_node_style(&cascaded, parent_style))
    } else {
        parent_style.cloned()
    };
    if matches!(doc.data(node), NodeData::Element(_)) {
        out.insert(node, this_style.clone().unwrap());
    }
    for &child in doc.children(node) {
        compute_subtree(doc, child, sources, this_style.as_ref(), out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_stylesheet;

    fn build_doc(html: &str) -> Document {
        html::parse_document(html)
    }

    fn find_first(doc: &Document, tag: &str) -> NodeId {
        let mut found = None;
        doc.walk(doc.root(), &mut |id, _| {
            if found.is_none()
                && let NodeData::Element(e) = doc.data(id)
                && e.local_name == tag
            {
                found = Some(id);
            }
        });
        found.expect("tag not found")
    }

    #[test]
    fn specificity_ordering() {
        let sheet = parse_stylesheet("#id{a:1} .a.b{a:2} div{a:3}");
        let mut specs: Vec<_> = sheet
            .rules
            .iter()
            .map(|r| {
                let list = selectors::parse_selector_list(&r.selector).unwrap();
                specificity_of_complex(&list.0[0])
            })
            .collect();
        specs.sort();
        assert_eq!(
            specs,
            vec![
                Specificity(0, 0, 1),
                Specificity(0, 2, 0),
                Specificity(1, 0, 0)
            ]
        );
    }

    #[test]
    fn author_important_beats_author_normal_and_specificity() {
        let doc = build_doc("<html><body><p id=\"x\" class=\"y\"></p></body></html>");
        let node = find_first(&doc, "p");
        let sheet = parse_stylesheet("#x { color: red; } .y { color: blue !important; }");
        let sources = [StyleSource {
            origin: Origin::Author,
            sheet: &sheet,
        }];
        let cascaded = cascade(&doc, node, &sources);
        assert_eq!(cascaded.get("color").unwrap(), "blue");
    }

    #[test]
    fn ua_important_beats_author_important() {
        let doc = build_doc("<html><body><p></p></body></html>");
        let node = find_first(&doc, "p");
        let ua = parse_stylesheet("p { color: black !important; }");
        let author = parse_stylesheet("p { color: red !important; }");
        let sources = [
            StyleSource {
                origin: Origin::UserAgent,
                sheet: &ua,
            },
            StyleSource {
                origin: Origin::Author,
                sheet: &author,
            },
        ];
        let cascaded = cascade(&doc, node, &sources);
        assert_eq!(cascaded.get("color").unwrap(), "black");
    }

    #[test]
    fn later_source_order_wins_ties() {
        let doc = build_doc("<html><body><p></p></body></html>");
        let node = find_first(&doc, "p");
        let sheet = parse_stylesheet("p { color: red; } p { color: green; }");
        let sources = [StyleSource {
            origin: Origin::Author,
            sheet: &sheet,
        }];
        let cascaded = cascade(&doc, node, &sources);
        assert_eq!(cascaded.get("color").unwrap(), "green");
    }

    #[test]
    fn later_declared_layer_wins_even_over_higher_specificity() {
        // #id in the earlier-declared layer "a" should still lose to a
        // bare type selector in the later-declared layer "b" -- layer
        // priority overrides specificity entirely, the whole point of
        // cascade layers.
        let doc = build_doc("<html><body><p id=\"x\"></p></body></html>");
        let sheet = parse_stylesheet(
            "@layer a, b; @layer a { #x { color: red; } } @layer b { p { color: blue; } }",
        );
        let node = find_first(&doc, "p");
        let sources = [StyleSource {
            origin: Origin::Author,
            sheet: &sheet,
        }];
        let cascaded = cascade(&doc, node, &sources);
        assert_eq!(cascaded.get("color").unwrap(), "blue");
    }

    #[test]
    fn unlayered_beats_any_layer_for_normal_importance() {
        let doc = build_doc("<html><body><p></p></body></html>");
        let sheet = parse_stylesheet("@layer a { p { color: red; } } p { color: green; }");
        let node = find_first(&doc, "p");
        let sources = [StyleSource {
            origin: Origin::Author,
            sheet: &sheet,
        }];
        let cascaded = cascade(&doc, node, &sources);
        assert_eq!(cascaded.get("color").unwrap(), "green");
    }

    #[test]
    fn important_inverts_layer_order_and_beats_unlayered() {
        // !important flips the usual layer priority: the *earlier*-
        // declared layer wins, and even that beats an unlayered
        // !important declaration.
        let doc = build_doc("<html><body><p></p></body></html>");
        let sheet = parse_stylesheet(
            "@layer a, b; \
             @layer a { p { color: red !important; } } \
             @layer b { p { color: blue !important; } } \
             p { color: green !important; }",
        );
        let node = find_first(&doc, "p");
        let sources = [StyleSource {
            origin: Origin::Author,
            sheet: &sheet,
        }];
        let cascaded = cascade(&doc, node, &sources);
        assert_eq!(cascaded.get("color").unwrap(), "red");
    }

    #[test]
    fn layer_statement_registers_order_before_any_rules() {
        // `@layer b, a;` fixes b before a even though `a`'s rule block
        // appears first in the source text.
        let doc = build_doc("<html><body><p></p></body></html>");
        let sheet = parse_stylesheet(
            "@layer b, a; @layer a { p { color: red; } } @layer b { p { color: blue; } }",
        );
        let node = find_first(&doc, "p");
        let sources = [StyleSource {
            origin: Origin::Author,
            sheet: &sheet,
        }];
        let cascaded = cascade(&doc, node, &sources);
        // "a" was declared after "b" in the @layer statement, so it wins.
        assert_eq!(cascaded.get("color").unwrap(), "red");
    }

    #[test]
    fn inheritance_and_initial() {
        let doc = build_doc("<html><body><div><p></p></div></body></html>");
        let sheet = parse_stylesheet("div { color: purple; display: flex; }");
        let sources = [StyleSource {
            origin: Origin::Author,
            sheet: &sheet,
        }];
        let styles = compute_document_styles(&doc, &sources);
        let p = find_first(&doc, "p");
        let div = find_first(&doc, "div");
        // color is inherited: <p> should see <div>'s purple.
        assert_eq!(styles[&p]["color"], "purple");
        // display is not inherited: <p> gets the initial value, not flex.
        assert_eq!(styles[&p]["display"], "inline");
        assert_eq!(styles[&div]["display"], "flex");
    }

    #[test]
    fn custom_properties_and_var_substitution() {
        let doc = build_doc("<html><body><div><p></p></div></body></html>");
        let sheet = parse_stylesheet("div { --brand: teal; } p { color: var(--brand); }");
        let sources = [StyleSource {
            origin: Origin::Author,
            sheet: &sheet,
        }];
        let styles = compute_document_styles(&doc, &sources);
        let p = find_first(&doc, "p");
        assert_eq!(styles[&p]["color"], "teal");
    }

    #[test]
    fn var_fallback_used_when_unset() {
        let doc = build_doc("<html><body><p></p></body></html>");
        let sheet = parse_stylesheet("p { color: var(--missing, hotpink); }");
        let sources = [StyleSource {
            origin: Origin::Author,
            sheet: &sheet,
        }];
        let styles = compute_document_styles(&doc, &sources);
        let p = find_first(&doc, "p");
        assert_eq!(styles[&p]["color"], "hotpink");
    }

    #[test]
    fn var_cycle_falls_back_to_inherited_or_initial() {
        let doc = build_doc("<html><body><p></p></body></html>");
        let sheet = parse_stylesheet("p { --a: var(--b); --b: var(--a); color: var(--a); }");
        let sources = [StyleSource {
            origin: Origin::Author,
            sheet: &sheet,
        }];
        let styles = compute_document_styles(&doc, &sources);
        let p = find_first(&doc, "p");
        // Cycle -> --a is invalid at computed-value time -> color falls
        // back to its (inherited) initial: canvastext.
        assert_eq!(styles[&p]["color"], "canvastext");
    }

    #[test]
    fn unset_on_inherited_property_inherits() {
        let doc = build_doc("<html><body><div><p></p></div></body></html>");
        let sheet = parse_stylesheet("div { color: orange; } p { color: unset; }");
        let sources = [StyleSource {
            origin: Origin::Author,
            sheet: &sheet,
        }];
        let styles = compute_document_styles(&doc, &sources);
        let p = find_first(&doc, "p");
        assert_eq!(styles[&p]["color"], "orange");
    }

    #[test]
    fn substitute_var_handles_multi_byte_utf8_around_the_reference() {
        // Found by fuzzing: `substitute_var_inner` used to read plain
        // ASCII-only chars a byte at a time outside `var(...)` spans,
        // desynchronizing its byte index on any multi-byte UTF-8 character
        // and panicking with "byte index N is not a char boundary" on the
        // next slice. Text with non-ASCII content both before and after a
        // `var()` reference exercises exactly that path.
        let mut custom_props = HashMap::new();
        custom_props.insert("--x".to_string(), "blue".to_string());
        let result = substitute_var("日本語 var(--x) \u{0BEF} 中文", &custom_props);
        assert_eq!(result.as_deref(), Some("日本語 blue \u{0BEF} 中文"));
    }
}
