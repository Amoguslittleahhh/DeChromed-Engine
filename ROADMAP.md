# DeChromed Engine — Roadmap to a Real Browser Engine

## Ground truth, stated up front

Blink (Chromium) and Gecko (Firefox) are ~15-25 million lines of C++/Rust
each, built by hundreds of engineers over 20+ years, backed by full-time
security teams, spec editors sitting on the W3C/WHATWG/TC39 standards bodies
themselves, and release infrastructure most companies never build at all.
"One-on-one replica" is the goal stated here, so this document is written as
if that's genuinely the target — but the scope needs to be seen clearly
before committing to it:

- Mozilla's own from-scratch modern-engine attempt, **Servo** (Rust),
  consumed dozens of person-years and, over a decade in, still isn't a full
  consumer replacement for Gecko — it was eventually folded back in as a
  component supplier to Gecko rather than replacing it wholesale.
- **Ladybird** (independent from-scratch browser, started 2019) is the
  closest real precedent for exactly this ambition. It runs on a funded
  team plus a large open-source community, across years, and is still
  pre-1.0 with large spec gaps.
- Chromium alone ships **~30 separate subsystems** you'd need to match for
  true parity: not just HTML/CSS/JS/layout, but PDF rendering, DRM (Widevine
  EME), a sync backend, an extension platform, a password manager with
  breach-detection, Safe Browsing, WebGPU, a full accessibility tree bridged
  to four different OS accessibility APIs, and more — most of which have
  nothing to do with "rendering a webpage" and everything to do with "being
  a product people trust with their whole digital life."

None of that means don't do this — it means measure progress in **phases**,
each independently shippable and testable, not in "days until done." This
document numbers phases across tracks so you always know what "next" means,
and every phase has a concrete, checkable exit criterion instead of a vibe.
`chrome-engine.html` (Phase 0) is the only phase currently complete.

**Working rule for every phase below:** no phase starts until the previous
phase in its track has passing tests checked into CI. This is not
bureaucracy for its own sake — it's the actual reason Blink and Gecko are
usable today: both gate all merges on **web-platform-tests (WPT)**, the
cross-browser conformance suite at [wpt.fyi](https://wpt.fyi), and treat a
regression there as a shipped bug. Do the same from day one, at whatever
tiny scale you're at, or scope creep silently rots the codebase the way it
has killed nearly every "build a browser from scratch" hobby project that
never finished.

---

## How this document is organized

Real engines are not built as one linear pipeline — they're built as
several parallel subsystems (tracks) that occasionally need to sync up.
Phases are grouped into tracks; phases within a track are ordered and
dependent on each other, but different tracks can, once their prerequisites
land, be worked in parallel (exactly how Chromium and Firefox actually
staff these — separate teams own layout, JS, networking, security, etc.).

- **Track A — Content & Style**: parsing HTML/CSS/SVG into a styled tree.
- **Track B — Layout & Graphics**: turning the styled tree into pixels.
- **Track C — Script & Runtime**: JavaScript, WebAssembly, the DOM API surface.
- **Track D — Platform & Storage**: networking, storage, media, device APIs.
- **Track E — Security & Privacy**: sandboxing, origin isolation, safe browsing.
- **Track F — Product & Ops**: DevTools, extensions, updates, telemetry, UX chrome.

A rough dependency map: **A → B** (can't lay out what you haven't parsed/
styled), **A/B → C** (DOM needs a tree to expose, JS needs a DOM to touch),
**C → D** (fetch/storage APIs are JS-surfaced), **B/C/D → E** (nothing to
sandbox until there's something running), **all → F** (nothing to inspect,
extend, or ship until it exists).

---

## Track A — Content & Style

### A0. Toy pipeline — *done*
`chrome-engine.html`: hand-rolled tokenizer → DOM → CSSOM → cascade →
block/inline layout → canvas paint. ~10 CSS properties, no real HTML5
algorithm, no JS. Proves pipeline shape only.
**Exit:** met. Frozen as a demo artifact; real work starts at A1.

### A1. Project foundation — *done*
Real multi-crate Rust workspace under `engine/` (see "Language & repo
shape" below and `engine/README.md`): `dom`, `html`, `css`, `layout`,
`paint`, `js_bindings`, `net`, `media`, `a11y`, `devtools`, `shell`, plus
`html5lib_harness` — a conformance harness (html5lib-tests' JSON tokenizer
suite, not full WPT, since WPT needs a JS engine to run `testharness.js`
and Track C doesn't exist yet) wired end to end against a deliberately
unimplemented tokenizer. CI (`.github/workflows/engine-ci.yml`) runs
fmt/clippy/build/test/harness/smoke-test on every push touching `engine/`.
**Exit:** met — repo structured, CI green, harness executing and reporting
a real (near-zero, as expected) baseline: **0.2% (15/6487)** on
html5lib-tests tokenizer tests. That number is the metric A2 moves toward
95%.

### A2. WHATWG-spec HTML tokenizer — *done*
The real [~80-state tokenizer state machine](https://html.spec.whatwg.org/multipage/parsing.html#tokenization)
in `engine/crates/html/src/tokenizer.rs`: character references (numeric +
the full 2231-entry named-reference table, vendored from the spec's
`entities.json`), CDATA sections, doctype parsing (including public/system
identifiers and force-quirks), attribute quoting edge cases,
script/RAWTEXT/RCDATA modes with "appropriate end tag" matching, newline
normalization, and the Windows-1252 control-code remapping table for
numeric references.

Known gap: the ScriptData escaped/double-escaped states (`<script>`'s
`<!--`-inside-script-content mechanism) aren't implemented yet --
`escapeFlag.test` and two `domjs.test` cases fail because of it. Flagged
in the crate's own module docs rather than silently passing.

**Exit:** met — **99.9% (6708/6713)** on the vendored html5lib-tests
tokenizer suite, against a ≥95% bar. The remaining 5 failures are the
ScriptData-escaping gap above (2) plus 3 cases in `xmlViolation.test`,
which tests a separate XML5 character-validation mode that standard HTML
tokenization doesn't apply.

### A3. HTML tree construction — *done*
The real [insertion-mode state machine](https://html.spec.whatwg.org/multipage/parsing.html#tree-construction)
in `engine/crates/html/src/tree_builder.rs`, replacing A1/A2's flat-append
placeholder: the full **adoption agency algorithm** (outer loop, furthest-
block detection, bookmark-tracked active-formatting-element reinsertion),
active formatting elements with the Noah's Ark clause, implicit element
closing (`<p>` auto-close, implied end tags, scope-checking across
Default/ListItem/Button/Table/Select scopes), table foster parenting
(transient, scoped only to `InTable`/`InTableText`'s "anything else"
fallbacks per spec), the `<nobr>` self-adoption special case, and the
`InHeadNoscript` insertion mode. The tokenizer (`engine/crates/html/src/tokenizer.rs`)
was upgraded from "tokenize to completion" to an incremental, resumable
`next_token()`/`set_state()` API so the tree builder can drive RAWTEXT/
RCDATA state switches mid-stream, as the spec requires.

Known gaps, documented in the module's own doc comments rather than
silently passing: no foreign content (SVG/MathML namespace switching), no
`<frameset>` document handling, no fragment-context parsing, and the
ScriptData escaped/double-escaped states remain unimplemented (inherited
from A2). A `html5lib_harness::tree_construction` binary (vendored WPT
`html/syntax/parsing/resources/*.dat` corpus, 58 files) measures real
conformance.
**Exit:** met — **66.6% (1154/1734)** overall on the vendored html5lib-tests
tree-construction suite; **80.3% (1093/1361)** excluding the files that are
entirely foreign-content/frameset/fragment/PI-node-convention tests (out of
scope per the gaps above), against the ≥95%-on-in-scope-corpus bar.

### A4. CSS tokenizer & parser — *done*
The real [CSS Syntax Module Level 3](https://www.w3.org/TR/css-syntax-3/)
tokenizer (`engine/crates/css/src/tokenizer.rs`: idents, functions,
at-keywords, hashes, strings/bad-strings, urls/bad-urls, numeric tokens
with dimension/percentage suffixes, delimiters, CDO/CDC) and parser
(`engine/crates/css/src/parser.rs`: component values, qualified rules,
at-rules, simple blocks, declaration lists with `!important` handling).
`engine/crates/css/src/lib.rs`'s `parse_stylesheet()` flattens qualified
rules into a public `Stylesheet`, including ones nested inside conditional-
group at-rules (`@media`/`@supports`/`@document` — condition not
evaluated, treated as always-true, a documented simplification not a
correctness claim), and records other at-rules (`@import`, `@font-face`,
etc.) as raw name/prelude/block text for later phases.

Also implements the [CSS Nesting Module](https://www.w3.org/TR/css-nesting-1/):
native `&`-relative nested style rules (`&` substitution via wrapping the
parent selector in `:is()`, preserving both the match set and specificity-
as-a-whole), implicit descendant nesting when no `&` is present, direct
(no-space) combinator attachment (`> .b` under `.a` resolves to
`.a> .b`), and nested at-rules mixing bare declarations (implicitly
`& { ... }`) with further nested rules. `parse_style_block` (`parser.rs`)
takes its `Vec<ComponentValue>` by value and moves (never clones) nested
`Block` contents while walking — found necessary after an early by-
reference version cloned each nested block's full remaining subtree once
per recursion level, an O(N²) blowup on deeply-nested input (see the
regression test `deeply_nested_at_rules_resolve_promptly_and_respect_the_
depth_guard` in `lib.rs`).

No vendored conformance corpus for this phase — WPT's `css/css-syntax`
suite drives via `testharness.js`, which needs a JS engine (Track C, not
built yet), consistent with the reasoning that led A1-A3 to use
html5lib-tests instead of WPT directly. Verified instead with self-authored
unit tests covering the spec algorithms and edge cases (string-newline
reconsumption, malformed-rule recovery, `!important` detection, at-rule
flattening, and 8 more for CSS Nesting's `&` substitution/implicit nesting/
combinator attachment/nested at-rules), plus stress-test fuzzing (deeply
nested `.a{.a{.a{...` inputs up to 1,000,000 levels complete in single-digit
seconds without stack overflow, respecting the existing depth guard).
**Exit:** met on the achievable bar given no JS engine — real tokenizer/
parser implementing the full spec grammar plus CSS Nesting, unit-tested;
WPT `css/css-syntax`/`css/css-nesting` deferred until Track C exists.

### A5. Selectors — *done*
Full [Selectors Level 4](https://www.w3.org/TR/selectors-4/) grammar in
`engine/crates/css/src/selectors.rs` (~950 lines): combinators (descendant/
child/next-sibling/subsequent-sibling), attribute selectors (all 6 matchers
plus the case-insensitivity flag), structural pseudo-classes (`:nth-child`/
`:nth-of-type` families with full `An+B` micro-syntax parsing including the
`of <selector>` extension), and logical pseudo-classes (`:not()`, `:is()`,
`:where()`, `:has()` with relative-selector-list support). Matching walks
backward from the selector's rightmost (subject) compound against the
queried DOM node, per how UA selector matching actually works.

Also implements `:lang()` (BCP47 extended-filtering-style range match,
case-insensitive, `*` wildcard components, comma-separated range lists,
against the nearest `lang` attribute found walking up from the element)
and `:dir()` (HTML's directionality algorithm: explicit `dir=ltr`/`rtl`,
`dir=auto`/`<bdi>` via a first-strong-character heuristic, inheritance
from the parent, defaulting to `ltr` at the root).

Known gap: pseudo-elements (`::before` etc.) and interaction-state pseudo-
classes (`:hover`, `:focus`, ...) aren't implemented — there's no layout/
event state yet for either to attach to; both parse into a documented
`PseudoClass::Unsupported` fallback rather than silently matching wrong.
`:lang()` doesn't consult out-of-band/protocol-level language (HTTP
`Content-Language`, `<meta http-equiv>`), only the element tree; `:dir()`'s
first-strong-character heuristic approximates UAX#9's BidiClass table with
a handful of Unicode block ranges for common RTL scripts rather than a
full bidi-class lookup. Same as A4, no vendored WPT `css/selectors` corpus
is runnable without Track C; verified with self-authored unit tests
covering combinators, attribute matching, `An+B` parsing edge cases
(including tokenizer artifacts like `Dimension{unit:"n-"}`), `:has()`, and
6 more for `:lang()`/`:dir()` (inheritance, own-attribute override,
range-list matching, no-lang-anywhere, explicit dir, inherited dir,
`dir=auto` heuristic).
**Exit:** met on the achievable bar — real Selectors Level 4 grammar and
matching plus `:lang()`/`:dir()`, unit-tested; WPT `css/selectors` deferred
until Track C exists.

### A6. Cascade & computed values — *done*
The real cascade sort in `engine/crates/css/src/cascade.rs`: origin/
importance ordering (UA/user/author normal, then author/user/UA
`!important` in the spec's reversed priority), full Selectors-Level-4
specificity computation (including `:is()`/`:not()`/`:has()`'s
most-specific-argument rule, `:where()`'s zero specificity, and
`:nth-child(An+B of S)`'s added specificity), source-order tie-breaking,
CSS custom properties (`--foo`) with inheritance and recursive `var()`
substitution (fallback values, cycle detection per
<https://www.w3.org/TR/css-variables-1/#invalid-variables>), and the
`initial`/`inherit`/`unset`/`revert` defaulting keywords.

Also implements real [CSS Cascade Layers](https://www.w3.org/TR/css-cascade-5/#layering)
(`@layer`): named/anonymous/nested layers, both the statement
(`@layer a, b;`) and block forms, layer order tracked as first-declaration
order in `Stylesheet::layer_order`, and correct cascade priority — layer
priority overrides specificity entirely; unlayered beats every layer for
normal-importance declarations but loses to every layer for `!important`
(the one place `!important` inverts an ordering rather than just
reprioritizing origins); later-declared layer wins among normal-importance
layers, earlier-declared layer wins among `!important` layers. Nested
`@layer` at-rules resolve through `parser::parse_style_block` against the
already-parsed `ComponentValue` tree rather than re-serializing to text and
re-parsing — an earlier version used a text-round-trip adapter that both
cost O(remaining-length) extra work per nesting level (near-cubic blowup
observed on deeply-nested input) and silently reset the parser's
recursion-depth guard on every level (each fresh re-parse started a new
`Parser` at `depth: 0`), defeating stack-overflow protection; fixed by
processing the single top-level parse's tree directly, which respects the
original depth-256 guard (verified: 200,000 requested nesting levels
completes in under 0.5s, correctly capping at 258 layers).

Known gaps, documented in the module's own doc comments: no used-value
resolution (computed values stop short of resolving `em`/`%`/etc. against
layout, which needs Track B to exist), a small hand-curated property table
(~30 common longhands' inherited-ness/initial value, not the full CSS
property registry), `revert` collapsed to `unset`'s behavior (a correct
`revert` needs a full layered per-origin cascade re-run), no animation/
transition origins, and `layer_order` isn't merged across multiple
`StyleSource`s passed to one `cascade()` call (each source's own layer
order is honored, but cross-source layer interleaving isn't modeled). As
with A4/A5, WPT's `css/css-cascade`/`css/css-variables` need a JS engine to
run and are deferred to Track C; verified instead with self-authored unit
tests (specificity ordering, origin/importance precedence, inheritance vs.
non-inheritance, `var()` substitution/fallback/cycles, defaulting
keywords, plus 4 more for cascade layers: later-layer-wins, unlayered-
beats-any-layer, `!important` layer-order inversion, and layer-statement
order registration) and a perf/depth-guard regression test for deeply
nested `@layer`.
**Exit:** met on the achievable bar — real cascade and computed-value
pipeline plus cascade layers, unit-tested; WPT `css/cssom`/`css/css-cascade`/
`css/css-variables` deferred until Track C exists.

### A7. CSSOM & style invalidation — *done*
`engine/crates/css/src/cssom.rs`: a mutable `CssomSheet`/`CssomRule`
object graph (insert/delete rule, get/set/remove a declaration's property)
standing in for `CSSStyleSheet`/`CSSRule` at the Rust level.
`engine/crates/css/src/style_engine.rs`: `StyleEngine` ties that together
with A6's cascade into a real `getComputedStyle` equivalent
(`get_computed_style`) plus — the actual point of this phase — **targeted
style invalidation**: an `InvalidationIndex` buckets every selector's
class/id/attribute names by whether a match can only affect the element
itself, its descendants, or its following siblings (conservatively folding
`:has()`'s relative selectors into the descendant bucket), so
`notify_class_changed`/`notify_attribute_changed` recompute only the nodes
that could actually be affected by a given mutation instead of the whole
document.

Known gap, documented in the module's doc comments: structural mutations
(a node added/removed/reordered among siblings, which can change
`:nth-child`-family results for other siblings) aren't tracked — callers
that mutate tree structure call `StyleEngine::rebuild` instead of relying
on incremental correctness. The `CssomSheet`/`CssomRule` graph is also not
yet reachable *from script*, since that binding layer needs Track C8 (DOM↔JS
bindings), which doesn't exist; what's here is the mutable object graph a
JS binding would wrap. Verified with unit tests that assert the *exact*
set of nodes recomputed after a mutation for each invalidation category
(self-only, descendant-dependent, sibling-dependent), including one that
compares a targeted restyle's result against a full rebuild's to prove the
shortcut isn't silently wrong, not just "didn't touch the obviously
unrelated node."
**Exit:** met — invalidation-set unit tests proving targeted (not
full-tree) restyle on class-change mutations across all three
invalidation categories.

### A8. SVG — *done*
Real foreign-content integration for inline `<svg>` in HTML, in
`engine/crates/html/src/foreign_content.rs` + `tree_builder.rs`: namespace
switching (`dom::ElementData::namespace`, new this phase), the SVG tag-
name/attribute-name case-adjustment tables, the breakout-tag list, HTML/
MathML-text integration points, and CDATA sections only becoming real
content inside foreign content (the tokenizer's `in_foreign_content` flag,
set by the tree builder per token). Also closed two real A3-era scope bugs
this work exposed: `has_element_in_scope`/`pop_until_including` now require
an HTML-namespace match (not just tag-name equality) per spec, and the
default scope's boundary list now includes the SVG/MathML integration-
point elements. Standalone SVG documents parse via `engine/crates/xml`
(A10) plus a thin `xml::svg::parse_svg_document` wrapper that defaults
unresolved namespaces to SVG's. A handful of SVG presentation properties
(`fill`, `stroke`, `stroke-width`, ...) are recognized by `css::cascade`'s
property table so they cascade/inherit correctly.

Known gaps: `<use>`/`<symbol>` reference resolution, real geometry, and
SVG-as-image loading are all Track B/D concerns that don't exist yet;
nothing consumes a painted SVG since there's no painter. As with A4-A7, WPT
`svg/` needs a JS engine and is deferred to Track C; measured instead
against the same vendored html5lib-tests tree-construction corpus as A3
(which includes real SVG/MathML integration test files) plus unit tests.
**Exit:** met on the achievable bar -- real foreign-content parsing,
unit-tested and corpus-measured (see A9's shared metric below); WPT `svg/`
deferred until Track C exists.

### A9. MathML — *done*
Shares A8's foreign-content machinery (MathML namespace switching, the
`definitionurl`→`definitionURL` attribute adjustment, MathML text
integration points `mi`/`mo`/`mn`/`ms`/`mtext`, `annotation-xml`'s
HTML-integration-point condition). No separate MathML-specific parser
needed -- it's the same dispatcher and tag/attribute-adjustment pattern as
SVG, just a different namespace and table.

**Exit:** met, same corpus as A8/A3 -- the vendored html5lib-tests
tree-construction suite (which includes real WHATWG conformance cases
purpose-built around SVG/MathML foreign-content edge cases, e.g.
`tests9.dat`-`tests12.dat`, `namespace-sensitivity.dat`) went from
**66.2% (1148/1734)** before this phase to **77.5% (1344/1734)** overall
(measured immediately before/after implementing foreign content, same
harness run); **86.9% (1303/1499)** excluding the files genuinely out of
scope for A8/A9 (fragment-context parsing, `<template>` content, the
PI-node serialization convention -- unrelated pre-existing A3 gaps).
Known gap: "basic layout" for `<math>` content is a Track B concern,
nonexistent until layout itself exists.

### A10. XML & XSLT — *done, XSLT explicitly dropped*
A real standalone XML 1.0 parser in the new `engine/crates/xml` crate:
prolog/DOCTYPE recognition (internal subset bracket-skipped, not
validated), comments, processing instructions (a new
`dom::NodeData::ProcessingInstruction` variant), CDATA sections, numeric
and the 5 predefined character/entity references, and full Namespaces-in-
XML resolution (`xmlns`/`xmlns:prefix`, inherited through nested scopes).
Unlike A2/A3's deliberately permissive HTML parser, this one is XML's
required opposite: a fatal well-formedness error (mismatched end tag,
duplicate attribute, undeclared entity, multiple root elements, ...) stops
parsing rather than trying to recover. A `serialize()` function round-trips
a parsed document back to escaped XML text.

Known gaps, documented in the crate's module docs: not a validating parser
(no DTD content-model/attribute-list validation, so custom `<!ENTITY>`
declarations aren't honored -- using one is a correct well-formedness
error, not a silent wrong answer); no external entity/DTD fetching; no
`<?xml-stylesheet?>` special handling (parses as an ordinary processing
instruction, nothing consumes it yet since there's no XML-document-as-
webpage loading path). **XSLT is dropped entirely**, exactly as this
document's original A10 entry pre-authorized -- it remains the
single-most-often-cut subsystem in real "shrink the engine" discussions,
and building a legacy stylesheet-transformation language for zero current
consumers isn't a good use of scope here.
**Exit:** met -- parses/serializes well-formed XML per the constraints
above; verified with 11 unit tests (prolog/doctype/comment/PI, namespace
resolution, CDATA, entity/character references, all four well-formedness
fatal-error cases, and a parse→serialize→reparse round-trip). No WPT
suite exists for bare XML parsing (WPT's XML coverage lives inside
`html/` and other test suites, not a standalone `xml/` directory), so
there's no external corpus to additionally measure against here.

### Track A hardening pass: stress testing and crash fixes

After A1-A10 were all functionally complete, a dedicated stress-testing
pass (conformance-blind: it doesn't check *correctness* against an
expected answer, only that every parser returns *something* instead of
crashing or hanging) found and fixed six real bugs across the track,
plus added a permanent fuzzing tool
(`engine/crates/html5lib_harness/src/bin/stress_test.rs`) so this keeps
getting checked, not just checked once:

- **Three stack-overflow crashes** (process aborts, uncatchable by
  `catch_unwind`) from unbounded recursion on pathologically deep input:
  A10's `xml::parse_document` on deeply nested elements, A4's
  `css::parser` on deeply nested blocks/functions, and A5's
  `css::selectors` parser on deeply nested `:not()`/`:is()`/`:has()`.
  Fixed with depth guards (512/256/128 respectively) -- for XML and
  selectors, exceeding the guard is a well-formedness/parse error (both
  are already fail-fast parsers); for CSS's permissive parser, exceeding
  it falls back to flat, unstructured token consumption instead of
  further recursion, so the sheet still parses *something* rather than
  erroring where the spec expects tolerance.
- **A quadratic (`O(depth²)`) blowup** in A3's tree builder: scope-
  checking algorithms (`has_element_in_scope` and friends) scan from the
  top of the open-elements stack down to the nearest boundary tag, and on
  markup that's deeply nested without ever hitting one (plain nested
  `<div>`s with nothing else), that scan is `O(depth)` per token --
  20,000 nested `<div>`s alone took over 10 seconds, and 50,000 didn't
  finish in a minute. Fixed with the same style of depth cap (512 open
  elements), matching real precedent (both Blink and Gecko impose a
  similar nesting-depth safeguard) -- past the cap, further start tags
  still produce real DOM nodes but stop nesting deeper, becoming flat
  siblings instead. This also transitively fixes a stack-overflow risk in
  every *downstream* recursive tree-walker (`dom::Document::walk`,
  `Display`, A6's cascade, the html5lib serializer, A10's XML
  serializer) that itself never chose to recurse unboundedly.
- **A stale-index panic in the adoption agency algorithm** (A3): the
  formatting element's position in the active-formatting-elements list
  was captured once before the algorithm's inner reparenting loop, but
  that loop can remove *other* list entries at earlier positions,
  shifting every later index down -- reusing the stale position afterward
  could read the wrong entry or index out of bounds entirely. Found by a
  targeted fuzzer generating randomly-misnested `<a>`/`<b>`/`<div>` soup
  (the specific shape this algorithm exists for), fixed by re-looking up
  the position fresh by identity instead of trusting the earlier one.
- **An integer overflow panic in `An+B` parsing** (A5): a large-magnitude
  negative `B` value (e.g. `:nth-child(3n- -999...999)`) casts to
  `i32::MIN`, and negating `i32::MIN` directly overflows `i32` (its
  magnitude has no positive `i32` representation) -- a debug-build panic
  Rust's release-mode wrapping arithmetic would have silently hidden.
  Fixed with `saturating_neg()`.
- **A UTF-8 char-boundary panic in `var()` substitution** (A6): the
  substitution loop read plain text a raw *byte* at a time (`bytes[i] as
  char`) instead of decoding real Unicode scalar values, silently
  corrupting -- and desynchronizing the byte index of -- any multi-byte
  UTF-8 character, which then panicked on the next string slice. Fixed by
  properly decoding the character at each position instead of casting a
  lone byte.

All six are permanent regression tests now (one per bug, in the relevant
crate's own test module), and `stress_test` itself runs clean (zero
distinct failures) across 13 different seeds and roughly 700,000+ total
fuzzed inputs as of this pass -- see the binary's own doc comment for what
it covers (truncation fuzzing, mutation fuzzing, random-byte-soup fuzzing,
a targeted adoption-agency generator, and end-to-end HTML+CSS→cascade
fuzzing) and `engine/README.md` for how to run it.

### Track A tech-currency pass: edition, dependencies, corpora, new web-platform features

A follow-up pass bringing Track A's toolchain and CSS feature surface up to
date, done in the same "implement -> stress-test with adversarial input ->
fix real bugs -> permanent regression test" style as the hardening pass
above:

- **Rust edition 2021 -> 2024** (`engine/Cargo.toml`'s `[workspace.package]`;
  every crate inherits it via `edition.workspace = true`). Enabled clippy's
  newer `collapsible_if` lint, which prefers stable Rust's **let-chains**
  syntax (`if let X = y && let A = b { ... }`) over nested `if let`s --
  applied at ~12 call sites across `dom`, `html`, `css`, `xml`, and the
  harness/shell binaries, all mechanical with no behavior change (verified
  by the full test suite passing identically before/after).
- **Vendored html5lib-tests corpus refresh**: re-fetched from upstream,
  picking up a new tokenizer test file (`unicodeChars.test`) and three new
  `#script-on` tree-construction files (`scripted_adoption01.dat`,
  `scripted_ark.dat`, `scripted_webkit01.dat`). The `scripted_*.dat` files
  need live JS execution during parsing (`document.write`/`setAttribute`
  calls interleaved with tokenization) to produce their expected trees --
  out of scope without Track C, same reasoning as the pre-existing
  `foreign-fragment.dat`/`template.dat` gaps -- so the harness now skips
  any `scripted_*.dat` file under `HARNESS_SKIP_KNOWN_GAPS=1`. Pass rates
  unchanged from before the refresh: tokenizer 99.9% (7031/7036), tree
  construction 77.3% (1344/1738) overall / 86.9% (1303/1499) excluding
  known gaps.
- **[CSS Nesting Module](https://www.w3.org/TR/css-nesting-1/)**: see A4
  above.
- **Real [CSS Cascade Layers](https://www.w3.org/TR/css-cascade-5/#layering)
  (`@layer`)**: see A6 above.
- **`:lang()` and `:dir()` pseudo-classes**: see A5 above.

Two more bugs were found and fixed by the same stress-then-fix discipline
while building the Nesting/Layers features (both documented in more detail
in A4/A6 above): a self-introduced `O(N²)` clone-based blowup in
`parse_style_block` on deeply-nested `{ }` blocks, and a pre-existing (not
introduced by this pass, inherited from A4's original `@media` handling)
much worse polynomial blowup *and* depth-guard defeat from a text-
round-trip parsing adapter, found while stress-testing the new `@layer`
nesting path. Both are now permanent regression tests. `serde`/`serde_json`
were already pinned to their latest compatible `1`-series versions --
`cargo update` found nothing to bump there.

---

## Track B — Layout & Graphics

### B1. Box tree generation — *started*
Real `display` computation and anonymous-box generation in the new
`engine/crates/layout` crate (`box_tree.rs`, ~250 lines): `display: none`
generates no box; `display: contents` splices its children straight into
the parent's child list rather than generating a box of its own; a
`display: list-item` element gets a real marker box (`disc`/`circle`/
`square`/`decimal` `list-style-type` keywords, numbered by position among
same-parent list-item siblings); and — the CSS2.1 rule that actually makes
this phase non-trivial — a block container with a mix of block-level and
inline-level children gets every maximal run of inline-level children
wrapped in a synthetic anonymous block box, so a block box's children end
up either all block-level or all inline-level, never mixed (verified by
walking the actual generated tree shape, not just trusting the algorithm
by inspection).

Known gaps, documented in the crate's own module docs: no `::before`/
`::after` generated content yet (needs pseudo-element matching support in
`css::cascade` that doesn't exist -- `p::before` currently never matches
any real DOM node, see `css::selectors`' own docs); `display: table`/
`table-row`/`table-cell`/`flex`/`grid` all collapse to plain block-level
rather than their real internal box types (B3/B4/B5's own job); list
markers are always rendered "inside" rather than in the margin the way
`list-style-position: outside` (the initial value) actually requires, and
don't support `<ol start>`/`<li value>`; whitespace collapsing is minimal
(an all-whitespace text node produces no box, but internal/leading/
trailing whitespace within real text isn't otherwise collapsed).
**Exit not yet met** (no `::before`/`::after`, no real table/flex/grid box
types) -- WPT `css/css-display` needs a JS engine to run anyway (`Track
C`), so verified instead with 5 self-authored unit tests covering
`display: none`/`contents`, anonymous-block wrapping (both the "gets
wrapped" and "pure inline, doesn't get wrapped" cases), and list-item
marker numbering, plus the shared stress-test fuzzer (see B2 below).

### B2. Block & inline formatting contexts — *started*
Real box-model geometry and layout in `engine/crates/layout/src/flow.rs`
(~450 lines) and `values.rs` (the "used value" length/percentage/keyword
resolution A6/A7 explicitly deferred until a real layout phase existed to
consume it): margin/border/padding/content-box widths resolved from
computed-style strings, including `auto` width filling the remaining
containing-block space and percentage margins/padding resolved against
the containing block; a real block formatting context (children stacked
vertically) with the *actual* CSS2.1 8.3.1 margin-collapsing algorithm
(positive margins take the max, then a negative margin's magnitude is
subtracted from that max -- not just "biggest number wins"); a real
(if simplified, see below) inline formatting context that flattens text
across nested inline boxes into words and wraps them into line boxes at
the containing block's width; and `float: left|right`/`clear: left|right|
both`, correctly scoped per block formatting context (a float leaves
normal vertical stacking, `clear` pushes a later sibling below it) rather
than globally.

Known gaps, documented in the module's own doc comments: no real font
shaping/glyph-metrics engine exists yet (that's B10), so word/line widths
use a flat `font_size_px * 0.5` per-character heuristic -- every text-
layout number here is a rough visual approximation, not pixel-accurate;
no shrink-to-fit/intrinsic sizing (an auto-width `inline-block` or float
falls back to the same "fill remaining width" rule an ordinary auto-width
block uses, which isn't spec-correct for either); floats are correctly
taken out of flow and positioned to a side and `clear` correctly pushes
below them, but normal-flow siblings/line boxes don't yet get narrowed to
visually wrap *around* a float (that needs per-line float-aware width
tracking, a further refinement); no parent-child margin collapsing and no
collapsing-through-empty-boxes (only adjacent-sibling collapsing is
implemented); no `auto`-margin centering; and line height on a mixed-
font-size line is just the max resolved line-height among its items, with
no baseline alignment.
**Exit not yet met** (WPT `css/css-box`/`css/CSS2/normal-flow`/`css/CSS2/
floats` need a JS engine to run and are separately blocked on B1's
remaining gaps anyway) -- verified with 8 self-authored unit tests
(box-model geometry, auto-width resolution, sibling margin collapsing
including the real positive/negative-magnitude algorithm, text line-
wrapping at different containing widths, float positioning, and `clear`)
plus a new `layout::layout` fuzzer added to
`engine/crates/html5lib_harness/src/bin/stress_test.rs` (mutated HTML+CSS
built into a real box tree and laid out at several containing-block
widths, including pathologically narrow/zero ones) -- 0 distinct failures
across 50,000+ iterations on two seeds.
`engine/crates/shell/src/main.rs` now runs this pipeline for real (parse
→ style → box tree → layout) against a fixed 800px containing-block
width in place of the old placeholder call.

### B3. Table layout — *started*
A real, deliberately simplified table layout in `engine/crates/layout`:
`display: table`/`table-row`/`table-cell` get a genuine box structure
(`box_tree.rs`'s `BoxKind::Table`/`TableRow`/`TableCell`), with row-groups
(`<thead>`/`<tbody>`/`<tfoot>`, or any `display: table-row-group`/
`-header-group`/`-footer-group`) transparently spliced into the table's
own row list rather than getting a box of their own. `layout_table`
(`flow.rs`) determines column count and per-column widths (single-colspan
cells with an explicit `width` hint their column; the rest of the
available width splits evenly among unhinted columns), then lays out
every row's cells at their spanned column width (real `colspan` support:
a spanning cell's content width is the sum of the columns it spans) and
stacks rows top-to-bottom.

Known gaps, documented in both modules' own docs: this is a simplified
column-sizing heuristic, not CSS2.1's real automatic-table-layout
algorithm (no min/max-content sizing pass — that needs intrinsic sizing
this project doesn't have yet); `rowspan` isn't implemented at all (every
cell behaves as `rowspan="1"`); `<caption>` and table columns
(`<col>`/`<colgroup>`) aren't handled; `border-collapse`/`border-spacing`
aren't in `css::cascade`'s property table yet, so cells always lay out
flush together regardless of what a stylesheet declares for either;
non-row/non-cell stray content is dropped rather than wrapped via
CSS2.1's full "anonymous table object" generation algorithm.
**Exit not yet met** (no min/max-content sizing, `rowspan`, or
`border-collapse`/`-spacing`) — WPT `css/CSS2/tables` needs a JS engine to
run anyway (Track C); verified instead with 3 self-authored unit tests
(equal column-width splitting, explicit-width column hinting, `colspan`
spanning + row stacking) plus the shared stress-test fuzzer (a
table+flex-mixing HTML seed, 0 distinct failures across 50,000+
iterations on two seeds).

### B4. Flexbox — *started*
Real (if scoped) [CSS Flexible Box Layout](https://www.w3.org/TR/css-flexbox-1/)
in `layout::flow::layout_flex_container`: a single implementation works
in abstract main/cross-axis terms for both `flex-direction: row` and
`column`, covering `flex-basis`/`flex-grow`/`flex-shrink` (the real
CSS Flexible Box §9.7 weighted distribution — positive free space by
`flex-grow` weight, overflow by `flex-shrink × basis` weight),
`justify-content` (`flex-start`/`flex-end`/`center`/`space-between`/
`space-around`), `align-items`/`align-self` (`flex-start`/`flex-end`/
`center`/`stretch`, with `normal`/`auto` correctly resolving to `stretch`
per spec), `row-reverse`/`column-reverse`, and `flex-wrap` (multi-line).
B1's `box_tree.rs` "blockifies" every direct child of a flex container
into a real flex item, unconditionally wrapping stray inline-level
content into an anonymous item (unlike an ordinary block box's
conditional anonymous-block wrapping). `css::cascade::PROPERTY_TABLE`
gained the flexbox longhands (`flex-direction`/`flex-wrap`/`flex-grow`/
`flex-shrink`/`flex-basis`/`justify-content`/`align-items`/`align-self`/
`align-content`) so real stylesheet declarations actually reach this code,
not just hand-built test styles.

Known gaps, documented in the module's own doc comments: `flex-wrap` and
cross-axis `stretch` are only implemented for row direction (column
direction is always single-line, and its cross axis — width — never
stretches, since that would need a second content-reflow pass at a new
width, which this phase doesn't perform); shrunk items clamp at a `0`
floor rather than a real min-content size (no intrinsic sizing, the same
gap B2 already documents); `order`, `gap`/`row-gap`/`column-gap`, and
multi-line `align-content` spacing aren't implemented (`align-content`
packs lines tightly with no extra distribution).
**Exit not yet met** (row-only wrap/stretch, no `order`/`gap`) — WPT
`css/css-flexbox` needs a JS engine to run anyway (Track C); verified
instead with 7 self-authored unit tests (grow/shrink weighted
distribution, `justify-content: center`, `flex-wrap` line-breaking,
`flex-direction: column` stacking plus auto-height sizing, `align-items:
stretch`) plus the shared stress-test fuzzer.

### B5. Grid — *started*
A real but significantly scoped [CSS Grid Layout](https://www.w3.org/TR/css-grid-1/)
in `layout::flow::layout_grid_container`: `grid-template-columns` track
parsing (`<length>`, `<percentage>`, `fr`, `auto`) and `fr`-weighted width
distribution (the same math B4's flex-grow already uses); rows are always
implicit, sizing to the tallest item placed in them (or to a matching
`Fixed` `grid-template-rows` entry, if the row index falls within its
explicit track list); and a real, occupancy-tracking auto-placement
algorithm (`grid-column`'s bare line number or `span N` forms claim their
cells first, then unplaced items flow row-major into whatever's left).
B1's `flex_items` helper (renamed only in spirit, not code) blockifies
grid items identically to flex items, since the spec's blockification
rule is the same for both.

Known gaps, documented in both `box_tree.rs` and `flow.rs`'s own module
docs: no `repeat()`/`minmax()`/named lines/subgrid; `grid-template-rows`'
`fr`/`auto` tracks are effectively unused (no definite grid-container
height exists in general to distribute them against — the same
underlying "no intrinsic sizing" limitation B2/B4 already document);
placement only reads `grid-column` (`grid-row` isn't implemented at all,
and the `"start / end"` range syntax isn't either, only a bare line
number or `span N`).
**Exit not yet met** (no `repeat()`/`minmax()`/subgrid, no `grid-row`) —
WPT `css/css-grid` needs a JS engine to run anyway (Track C); verified
instead with 4 self-authored unit tests (`fr`-weighted column splitting
plus row-wrap, explicit-column placement leaving earlier cells for
auto-placed items, row height from tallest item, an explicit
`grid-template-rows` track overriding content height) plus the shared
stress-test fuzzer.

### B6. Positioning & stacking — *started*
`position: relative` is real and correctly scoped: the box stays fully
in normal flow for sizing/margin-collapsing/sibling-positioning purposes
(exactly as if unpositioned), and only its own final visual position
shifts by `top`/`left` afterward. `absolute`/`fixed` are taken out of
normal flow entirely (no margin collapsing, no vertical-stacking slot,
no float interaction — the same treatment `float` already gets) and
positioned via `top`/`left`, in `layout_block_children`.

Known gaps, documented in `flow.rs`'s module docs: `absolute`/`fixed`
are positioned relative to the **immediate parent's** content-box
origin, not the spec's real "nearest positioned ancestor" (which needs
ancestor-chain position-type tracking this phase doesn't implement), and
for `fixed`, not the viewport either (no distinct viewport/scroll-
container concept exists yet, so `fixed` behaves identically to
`absolute` here); only `top`/`left` are read (`right`/`bottom` aren't),
and only as pixel lengths (not percentages, which would need a definite
containing-block dimension this phase doesn't resolve in general);
`position: sticky` isn't implemented (behaves as `static`, since there's
no scroll container to stick within); `z-index`/stacking contexts/paint
order aren't addressed at all — there's no paint pipeline yet (B10-B12)
for a stacking order to actually affect, so this is explicitly out of
scope for this landing rather than faked.
**Exit not yet met** (containing-block resolution simplified, no
`sticky`, no `z-index`/stacking) — WPT `css/css-position`/`css/CSS2/
zindex` need a JS engine to run anyway; verified instead with 2
self-authored unit tests (`relative` offsetting without disturbing
siblings, `absolute` being out-of-flow and offset-positioned) plus the
shared stress-test fuzzer.

### B7. Fragmentation — *started*
A real multi-column implementation in `layout::flow::layout_multicol`:
`column-count`/`column-width` resolution (including the actual spec rule
for when both are set — the smaller of "however many `column-count`
columns" and "however many `column-width`-sized columns fit" wins), and
genuine height-based slicing of already-flowed content into real
side-by-side columns, honoring `break-before`/`break-after: always`/
`column` as forced breaks and defaulting to CSS's `column-fill: balance`
(the initial value) for the rest. Children are flowed once at the
resolved *column width* (so text/block content wraps at the correct
measure) and the resulting sequence is then sliced by height — this
works for both block-level and purely-inline content, since both
formatting contexts already return the same `Fragment` sequence shape.

Known gaps, documented in the module's own doc comments: no
fragmentation *within* a single child (a child taller than a column
target just overflows/extends past it, rather than being split — so
`break-inside: avoid` is trivially always honored, since nothing splits
inside a fragment regardless); `column-fill: auto` isn't implemented
(always balances); `column-rule` (the visible divider) isn't implemented
(no paint pipeline exists yet to draw it); `column-gap: normal` resolves
to a stated, reasonable `1em` choice rather than one true spec-mandated
value (there isn't one).
**Exit not yet met** (no within-child fragmentation, no `column-fill:
auto`/`column-rule`) — WPT `css/css-multicol`/`css/css-break` need a JS
engine to run anyway (Track C); verified instead with 2 self-authored
unit tests (even balancing across 3 columns, `break-before: always`
forcing a column regardless of natural balance) plus the shared
stress-test fuzzer.

### B8. Writing modes & internationalized layout — *started (logical properties + `direction: rtl` only)*
Real logical-property resolution in `layout::flow::resolve_box_model`:
`margin-inline-start/-end`, `margin-block-start/-end`,
`padding-inline-start/-end`, and `padding-block-start/-end` all resolve
to the correct physical side (block-start/-end always map to top/bottom,
since only `horizontal-tb` writing mode is supported; inline-start/-end
map to left/right according to `direction`) when a stylesheet doesn't
set the equivalent physical longhand directly. `direction: rtl` also has
a real effect on inline layout (`layout::flow::layout_inline_children`):
each already-built line is mirrored within the full containing width,
correctly reversing visual left/right order while preserving logical
(source) order, so the first word in reading order ends up rightmost, as
RTL requires.

**Vertical writing modes (`writing-mode: vertical-rl`/`vertical-lr`)
are explicitly not implemented at all in this landing** — real vertical
writing modes swap which axis is "block" and which is "inline"
throughout *every* formatting context this crate has (block, inline,
table, flex, grid), which needs the same kind of axis-agnostic rewrite
B4's flexbox algorithm already does for its own two axes, but applied
project-wide. That's a real architectural undertaking on its own, stated
plainly as future work rather than faked via a cosmetic post-hoc
rotation of otherwise-horizontal layout output. The full Unicode
Bidirectional Algorithm (UAX #9) also isn't implemented — mixed-
direction text within one inline formatting context isn't reordered
per-run, only the container's own `direction` is honored uniformly for
the whole context.
**Exit not yet met** (no vertical writing modes, no UAX #9 bidi) — WPT
`css/css-writing-modes`/`css/css-logical` need a JS engine to run
anyway, and bidi conformance against the Unicode BidiTest data files
needs the UAX #9 algorithm this phase doesn't implement; verified
instead with 2 self-authored unit tests (RTL inline-content mirroring,
logical-margin-to-physical-side mapping in both directions) plus the
shared stress-test fuzzer.

### B9. Fragment tree & display list — *started (query utilities + display-list lowering; no snapshot-test corpus)*
The fragment tree itself has been real since B1/B2 (`layout::Fragment` —
positioned, sized boxes referencing their originating DOM/style nodes, in a
single genuinely **absolute** coordinate space: `flow::layout()` anchors the
root's border box at `(0, 0)`, and every formatting-context function
positions a child via `reposition()`/`shift()`, which moves that child's
*entire already-built subtree* together, so descendant coordinates stay
correct once an ancestor is placed). This phase adds the two things that
were still missing:
- **Query utilities** (`layout::query`): `bounding_client_rect`/
  `element_from_point` — the real `getBoundingClientRect()`/
  `elementFromPoint()` equivalents — implemented purely by reading
  `content_rect`/`border_box()` off the tree, with zero re-derivation of
  geometry. `element_from_point` picks the deepest fragment under the point,
  breaking ties between overlapping siblings by later-in-tree-order-wins —
  an approximation of paint order, not a real stacking-context/`z-index`
  model (that's still B6's own documented gap).
- **Display-list lowering** (`paint::display_list::build_display_list`):
  turns a `Fragment` tree into an ordered `DisplayList` of `FillRect`/
  `DrawText` items, threading real `color` inheritance down through the walk
  (the same top-down pattern `layout::flow` already uses for `font-size`).
  Backed by a real (if modest) CSS `<color>` parser (`paint::color`): named
  colors, `#rgb`/`#rrggbb`/`#rrggbbaa` hex, `rgb()`/`rgba()` functional
  notation, `transparent`, with `currentcolor` deliberately left unresolved
  so it naturally falls back to the inherited color.

**Known gaps:** only `background-color` (fill) and `color` (text) are read
— no `border-color`/border painting, `background-image`, `box-shadow`,
`opacity` compositing, or clipping (`overflow: hidden` isn't implemented
anywhere yet), so there are no `PushClip`/`PushTransform` items yet either.
**Exit not yet met** — no display-list snapshot-test corpus (would need a
page corpus and a snapshot format); verified instead with 3 self-authored
unit tests for `query.rs` (nested-offset accumulation across padding+margin,
a node not present in the tree, deepest-fragment hit-testing), 3 for
`display_list.rs` (background fill, transparent background emits nothing,
text color inheritance), plus the shared stress-test fuzzer, which now also
runs `paint::build_display_list` over every fuzzed fragment tree.

### B10. Text shaping & fonts — *started (proportional character-width metrics only; no real shaping)*
`layout::values::char_width_em`/`text_width_px` replace the previous flat
`font_size_px * 0.5` per-character heuristic with a real, table-driven
proportional-width approximation — narrow characters (`i`, `l`, punctuation)
are genuinely narrower than wide ones (`m`, `w`, uppercase letters) now,
loosely modeled on typical Latin-alphabet proportional-font ratios. Used by
`layout::flow`'s inline line-breaking for real per-word/per-space widths
instead of a uniform constant.

**This is explicitly not real font shaping.** There's no glyph outline
data, no kerning, no ligatures, no font-specific metrics, no complex-script
support (Arabic joining, Indic reordering), no font matching/fallback
chains, no `@font-face`/WOFF2 loading, no variable fonts, and no bidi
integration (B8's own gap) — every number this produces is still a rough
visual approximation of *some* common sans-serif font, not a pixel-accurate
measurement of any real one.
**Exit not yet met** (WPT `css/css-text`/`css/css-fonts` need a JS engine
to run anyway, and there's no real shaper to visual-diff against a
reference corpus) — verified instead with 2 self-authored unit tests
(character widths are genuinely proportional; text width sums real
per-character widths) plus the shared stress-test fuzzer.

### B11. Painting & rasterization — *started (software rasterizer, FillRect only; no GPU path)*
`paint::raster`: a real software rasterizer — the same baseline both Blink
(Skia) and Gecko (WebRender) also bootstrap new platforms from before
adding a GPU path. Turns a `DisplayList` into a real RGBA8 pixel buffer
(`Canvas`), with genuine scanline rectangle fill and Porter-Duff "source-
over" alpha compositing (real per-channel blend math, not a stand-in) —
the pixels this module produces are pixel-accurate for what it draws.

**What it draws:** `DisplayItem::FillRect` only. `DisplayItem::DrawText`
items are correctly positioned and colored by B9's display-list lowering,
but the rasterizer doesn't paint them — there's no font outline data or
embedded bitmap glyph atlas yet (B10 only improved *measurement*, not
shaping/rendering), so drawing placeholder glyph shapes would overclaim
what's implemented; text regions are simply left unpainted, a documented
gap rather than a faked rendering.
**Known gaps:** no anti-aliasing (hard pixel-boundary edges), no clipping,
no GPU path (wgpu-based, mirroring Skia/WebRender's batching and tile/glyph
caching, is still entirely future work).
**Exit not yet met** (no pixel-diff reftest corpus, no perf budget) —
verified instead with 4 self-authored unit tests (exact-pixel opaque fill,
alpha-blended fill against the white background, an off-canvas rect that
doesn't panic, confirming `DrawText` items are left unrasterized) plus the
shared stress-test fuzzer, which now also runs `paint::rasterize` over
every fuzzed display list.

### B12. Compositing
Layer promotion (`transform`/`opacity`/`will-change`), a compositor thread
independent of the main thread — this is *the* thing that makes scrolling
and animations feel smooth in real browsers and is routinely the difference
between "demo" and "usable." Threaded scrolling, async transform animations.
**Exit:** scroll/animate a layered page at a sustained 60fps target on a
benchmark corpus; compositor operates without blocking on main-thread JS.

### B13. Canvas 2D & WebGL/WebGPU
`<canvas>` 2D context API (path filling/stroking, `ImageData`, compositing
operations) — its own spec surface distinct from CSS painting. WebGL
(bindings to an existing GL/Vulkan abstraction — nobody hand-writes a GPU
driver) and WebGPU for the modern API surface.
**Exit:** WPT `html/canvas` ≥60%; a handful of real-world WebGL demos (e.g.
three.js samples) rendering correctly.

---

## Track C — Script & Runtime

### C1. DOM Level tree API
The actual addressable DOM (`Node`, `Element`, live `NodeList`/
`HTMLCollection`, mutation algorithms per the DOM spec, not just an
internal tree) — this has to exist as a real spec-shaped API before JS
Track work is meaningful, since JS mostly *is* DOM manipulation in practice.
**Exit:** WPT `dom/nodes` ≥75%.

### C2. Event loop & event dispatch
Task queues/microtask queue ordering per the HTML spec (this ordering is
subtle and web-observable — get it wrong and real sites break in confusing
ways), `requestAnimationFrame` tied to the rendering pipeline, event
capture/target/bubble dispatch with proper default-action handling.
**Exit:** WPT `html/webappapis`, `dom/events` ≥70%.

### C3. ECMAScript parser & AST
Full ES2015+ grammar (this alone is bigger than most people expect: ASI
rules, destructuring, template literals, generators/async syntax, classes,
modules). State explicitly here whether the "one-on-one replica" intent
means writing this yourself vs. embedding V8/SpiderMonkey — it changes
every phase after this one (see "The JS engine question" below).
**Exit:** parses the full Test262 corpus's syntax without embedding-engine
help (if going purist), or bindings compile/run against an embedded engine
(if going pragmatic).

### C4. Interpreter (bytecode VM, tree-walk first)
A correctness-first tree-walking or simple bytecode interpreter — no JIT
yet. Scoping (`var`/`let`/`const` semantics, closures, `this` binding
rules), prototype chains, the full object model.
**Exit:** Test262 language-syntax + basic built-ins pass rate tracked as
the headline metric from here on.

### C5. Standard library / built-ins
`Object`/`Array`/`String`/`Map`/`Set`/`Promise`/`RegExp`/`Intl` — `Intl` in
particular is its own ICU-backed subsystem (locale-aware formatting,
collation) that's easy to underscope.
**Exit:** Test262 built-ins suite ≥70%.

### C6. Garbage collector
A real GC (generational, ideally — matches how both V8 and SpiderMonkey
are shaped, because short-lived object churn dominates real JS workloads),
integrated with DOM object lifetime (this cross-language GC-to-native-tree
integration, "wrapper tracing," is a notoriously hard correctness problem —
both engines have had serious security bugs here).
**Exit:** no leaks/no use-after-free across a stress-test corpus running
under a sanitizer (ASan/MSan-equivalent for Rust: Miri + fuzzing).

### C7. JIT tiers (optional, high-difficulty)
If going purist: an interpreter → baseline JIT → optimizing JIT pipeline
with deoptimization, matching the tiered architecture both V8 (Ignition/
Sparkplug/Maglev/TurboFan) and SpiderMonkey (Baseline/Ion) use. This is
realistically a **separate multi-year project** on its own; most from-
scratch browser efforts (including Ladybird) run an interpreter-only JS
engine for years before JIT work starts.
**Exit:** explicitly flagged as a stretch phase; interpreter-only C4/C5/C6
is a legitimate, shippable stopping point.

### C8. DOM↔JS bindings & Web IDL
The binding layer connecting the C1 DOM tree to the C3-C7 JS runtime,
generated from Web IDL definitions (as both engines do — hand-writing
thousands of bindings by hand doesn't scale and is where wrapper-lifetime
bugs live).
**Exit:** a real webpage using `document.querySelector`, `addEventListener`,
and basic DOM mutation from script runs correctly.

### C9. WebAssembly
A Wasm interpreter/compiler (binary format parsing, validation, execution)
— its own runtime, sharing some infrastructure with C6's GC/memory model
but a genuinely separate spec surface.
**Exit:** a Wasm conformance test suite (the official `testsuite` repo)
passing at a meaningful rate; a real compiled Wasm module (e.g. from
Rust/AssemblyScript) running.

### C10. Workers
Web Workers (separate JS global + event loop, `postMessage` structured
clone), Service Workers (install/activate lifecycle, fetch interception —
this is the foundation PWAs depend on and touches Track D's networking
directly).
**Exit:** WPT `workers/`, `service-workers/` core suites ≥50%.

---

## Track D — Platform & Storage

### D1. URL parsing
The [WHATWG URL spec](https://url.spec.whatwg.org/) precisely — this is
deceptively subtle (userinfo, IDNA/punycode for internationalized domains,
special-scheme handling) and a common source of security bugs (URL parsing
mismatches between browser and server are a real SSRF/auth-bypass vector).
**Exit:** WPT `url/` ≥90%.

### D2. Networking core
HTTP/1.1 client → HTTP/2 → HTTP/3(QUIC). TLS via an existing, audited crate
(rustls) — reimplementing TLS from scratch is a security liability, not an
engine feature, and no serious project does it. Connection pooling, caching
per HTTP cache-control semantics, redirect handling, cookie jar.
**Exit:** loads real HTTPS URLs end-to-end; HTTP caching semantics unit
tested against RFC 9111 conformance cases.

### D3. Resource loading pipeline
Preload scanner (speculative parsing to kick off fetches before the main
parser reaches a tag — a real, measurable performance feature both engines
have), priority scheduling, `fetch()`/`XMLHttpRequest`/Streams API bindings
once Track C exists.
**Exit:** WPT `fetch/`, `streams/` ≥60%; preload scanner measurably reduces
load time on an image/script-heavy benchmark page.

### D4. Storage APIs
`localStorage`/`sessionStorage`, IndexedDB (a full transactional
object-store database with its own spec), Cache API (Service Worker
backing store), Cookie Store API.
**Exit:** WPT `IndexedDB/`, `webstorage/` ≥60%.

### D5. Images & media containers
Image codecs (JPEG/PNG/GIF/WebP/AVIF — bind existing audited decoders,
don't hand-roll codecs, that's its own huge and security-sensitive
subsystem), `<img>`/`<picture>`/responsive images (`srcset`/`sizes`).
**Exit:** WPT `html/semantics/embedded-content` image suites ≥70%.

### D6. Audio/video playback
`<video>`/`<audio>` element behavior, Media Source Extensions (adaptive
streaming used by every major video site), codec integration (bind
existing decoders: h264/vp9/av1/opus/aac). Encrypted Media Extensions
(DRM/Widevine-equivalent) explicitly flagged as its own licensing-gated
subsystem real engines treat separately from the open-source core.
**Exit:** WPT `media/` core playback suites ≥50%; MSE-based playback works
against a real adaptive-streaming test stream.

### D7. WebRTC
Peer connection, ICE/STUN/TURN negotiation, media transport — a large,
mostly self-contained subsystem both engines source from a shared-ish
lineage (libwebrtc) rather than maintaining fully independently; a
pragmatic replica likely binds an existing implementation here rather than
rewriting the ICE/SRTP stack from scratch.
**Exit:** two instances of the engine can establish a peer connection and
exchange media/data in a controlled test.

### D8. Device & platform APIs
Geolocation, Web Bluetooth/USB/Serial, File System Access API, Clipboard
API, drag-and-drop, Notifications/Push API, Web Authentication (WebAuthn/
passkeys) — each individually small-to-medium, collectively a long tail
that's a large fraction of "why does site X not work" in practice.
**Exit:** WPT suites for each API tracked individually; prioritize by
real-world usage data rather than trying to do all of them at once.

### D9. Forms & autofill
Form validation (`:valid`/`:invalid`, constraint validation API), the
autofill heuristics (guessing field purpose from name/autocomplete
attributes) and password manager integration — genuinely its own ML/
heuristics-adjacent subsystem in real browsers, easy to underscope as
"just fill in a text box."
**Exit:** WPT `html/semantics/forms` ≥70%; autofill correctly identifies
field purpose on a corpus of real-world signup/login forms.

---

## Track E — Security & Privacy

### E1. Origin model & same-origin policy
Formal origin representation, same-origin-policy enforcement checked
consistently across *every* subsystem touched in A-D (DOM access across
frames, cookies, fetch/CORS, storage partitioning) — this needs to be
designed early and threaded through everything, not bolted on later; that's
precisely how real cross-origin security bugs happen in practice.
**Exit:** WPT `cors/`, `html/browsers/origin` ≥70%; a documented origin
model doc that every later subsystem is checked against in review.

### E2. Content Security Policy & mixed content
CSP directive parsing/enforcement, mixed-content blocking (HTTPS pages
loading HTTP subresources), Trusted Types.
**Exit:** WPT `content-security-policy/` ≥60%.

### E3. Sandboxing & process model
Multi-process architecture: separate renderer/browser/GPU/network
processes with IPC between them (this is *the* headline security feature
both Chromium's and Firefox's sandbox designs are built around), OS-level
sandboxing primitives (seccomp-bpf on Linux, the Windows sandbox APIs, the
macOS Seatbelt/App Sandbox), and **site isolation** — a renderer process
per origin so a compromised renderer can't read another site's data. This
is architecturally expensive to retrofit, so the IPC boundary should exist
conceptually from early on even before full multi-process lands.
**Exit:** a compromised/fuzzed renderer process cannot read another
origin's data or escape to touch the filesystem in a controlled test.

### E4. Fuzzing & continuous security testing
Structured fuzzing harnesses for the HTML/CSS/JS parsers and the image/
media codec bindings — this is genuinely how most real browser security
bugs get found before shipping (Chromium's ClusterFuzz, Mozilla's
oss-fuzz integration), not a nice-to-have.
**Exit:** fuzzers running continuously in CI against every parser/codec
boundary; a triage process for crashes.

### E5. Safe Browsing / malware & phishing protection
Reputation-list checking against known-malicious URLs, download scanning
integration — explicitly a "consumes an external threat-intel feed" feature
rather than something to build the intelligence for from scratch.
**Exit:** blocks a test corpus of known-bad URLs from a public phishing
test list.

### E6. Certificate transparency & transport security
HSTS enforcement, Certificate Transparency log checking, certificate
pinning infrastructure.
**Exit:** correctly rejects a test corpus of invalid/revoked/CT-violating
certificates.

### E7. Privacy features
Tracking protection / third-party cookie partitioning, fingerprinting
mitigation, private browsing mode data isolation — an area where Blink and
Gecko have historically diverged significantly in philosophy (Chromium's
Privacy Sandbox proposals vs. Firefox's Enhanced Tracking Protection), so
"replica" here means picking a stance, not just copying one engine.
**Exit:** documented privacy model; private-mode session leaves no
persistent state after close, verified by filesystem/storage inspection.

---

## Track F — Product & Ops

### F1. DevTools protocol & inspector
An inspector protocol (own, or compatible with Chrome DevTools Protocol /
Firefox Remote Protocol) exposing the DOM tree, computed styles, console,
network requests, and — once Track C JIT/runtime work exists — a JS
debugger (breakpoints, step execution, call stack inspection).
**Exit:** an external DevTools-style UI can attach and show live DOM +
styles + console + network waterfall.

### F2. Accessibility tree
A parallel accessibility tree derived from the DOM/style/layout trees
(ARIA role/state computation per the [AccName](https://www.w3.org/TR/accname-1.2/)
and [Core-AAM](https://www.w3.org/TR/core-aam-1.2/) specs), bridged to each
target OS's native accessibility API (UIA on Windows, AX API on macOS,
AT-SPI on Linux) — this is a full subsystem in real engines, not a
metadata afterthought, and is required for the browser to be usable with a
screen reader at all.
**Exit:** WPT accessibility test suites tracked; a real screen reader
(NVDA/VoiceOver/Orca) can navigate a rendered page correctly.

### F3. Extension platform
An extension API surface (manifest format, background scripts/service
workers, content scripts with isolated worlds, `declarativeNetRequest`-
style request modification) — its own significant API surface layered on
top of everything else.
**Exit:** a simple real-world extension (e.g. an ad-blocker or a DOM
inspector bookmarklet-equivalent) runs correctly.

### F4. Printing & PDF
Print layout (CSS `@page`, print-specific fragmentation from B7), a PDF
rendering path (either a bundled PDF renderer or binding an existing one)
for viewing PDFs directly in the browser — both engines ship this as a
first-class feature, not an afterthought handed to a plugin.
**Exit:** print-preview output matches a reference PDF within tolerance for
a benchmark page set.

### F5. Sync & profiles
Multi-profile support, an account-backed sync backend for bookmarks/
history/passwords/settings across devices — explicitly a "you need backend
infrastructure, not just client code" phase; scope/timeline depends
entirely on whether a server component is in scope at all.
**Exit:** two client instances converge on the same bookmark/history state
after syncing through a test backend.

### F6. Telemetry, crash reporting & update channel
Crash reporting infrastructure (symbolication, minidumps — Chromium's
Crashpad / Mozilla's Socorro are the real precedents), opt-in usage
telemetry, and a staged release channel model (canary/beta/stable) with an
auto-update mechanism.
**Exit:** a crash in a test build produces a symbolicated, actionable
report; an update rollout can be staged to a subset of a test fleet.

### F7. Browser UI shell
Everything `chrome-engine.html`'s toy UI faked — tabs, omnibox with search/
navigation suggestions, bookmarks, history, settings, profile switching —
rebuilt against the real engine underneath instead of hardcoded demo pages.
**Exit:** the shell drives the real D2/D3 networking + A-B rendering
pipeline for arbitrary real URLs, not a fixed demo set.

---

## The JS engine question — decide this explicitly before Track C

This is the single biggest fork in the whole roadmap, so it's called out on
its own rather than buried in C3:

- **Pragmatic path:** embed an existing, battle-tested engine (`rusty_v8`
  bindings to V8, or `mozjs` bindings to SpiderMonkey) and put all original
  effort into the DOM/layout/binding layers around it. This is what nearly
  every "build your own browser" project that actually ships chooses,
  because a from-scratch JIT-compiled JS engine is realistically its own
  multi-year, dedicated-team project — writing one is *harder* than
  everything in Tracks A+B combined.
- **Purist path ("truly one-on-one, nothing borrowed"):** write the parser,
  interpreter, GC, and eventually JIT tiers from scratch (C3-C7 as written
  above, including the explicitly-flagged-as-hard C7). This is the only way
  to hit "replica built entirely from scratch" as a *literal* claim, and it
  roughly doubles the total project's realistic timeline.

Neither answer is wrong — but the rest of the roadmap's shape (especially
how much of Track C is "phases" vs. "one integration phase") depends on
picking one before C3 starts.

---

## Language & repo shape

- **Rust workspace**, one crate per major subsystem, mirroring how Servo
  and Ladybird are structured: `html`, `css`, `dom`, `layout`, `paint`,
  `js` (or `js-bindings` if embedding), `net`, `media`, `a11y`, `devtools`,
  `shell`. Crate boundaries double as the ownership boundaries a real team
  would staff along.
- Pull in reference implementations to **test against**, not depend on
  directly, wherever the goal is "recreate" rather than "wrap" — e.g. run
  your own HTML tokenizer's output against html5ever's as a correctness
  oracle in CI, without shipping html5ever itself.
- TLS (rustls), image/media codecs, and ICU/Unicode data are the standing
  exceptions: bind existing audited libraries for these always. They are
  security-critical, spec-external (codecs aren't web specs, they're
  separate standards bodies' formats), and reimplementing them buys
  security risk with no engine-architecture learning in return.

---

## Reference architecture: what Blink and Gecko actually do

Every phase above is written against the *spec*. Specs describe required
behavior, not architecture — they don't tell you that a naive
implementation will be too slow to use, or which subproblem turns out to be
the load-bearing one in practice. This section is that missing layer:
concrete precedent from the two production engines named in this project's
goal, organized by phase, so each phase can be checked against "what did
the people who already built this find out the hard way" before writing
code, not after.

**How to use this section:** it is a reference to read *at* each phase, not
code to vendor in — see "Language & repo shape" above for why nothing gets
copied wholesale. Where a source directory is named, treat it as "go read
this when the phase's spec text is ambiguous," the same way the spec
compliance tests (WPT, html5lib-tests, Test262) are used as an oracle, not
a dependency.

### Track A — Content & Style

**HTML parsing (A2/A3).** Both engines learned the same lesson from
opposite directions: don't run the real parser as a single blocking pass.
Blink's `HTMLPreloadScanner` (`third_party/blink/renderer/core/html/parser/`)
runs a cheap, deliberately non-spec-compliant shadow tokenizer ahead of the
real `HTMLTreeBuilder` purely to fire off `<img>`/`<link>`/`<script src>`
fetches early — a naive spec-only implementation misses this and loses
real load-time performance. Gecko's `parser/html/` goes further: full
tokenization and tree-building run on a **dedicated parser thread**, with
**speculative parsing** past a blocking `<script>` tag that gets reconciled
or discarded depending on whether the script called `document.write`. Both
are optimizations layered *on top of* spec-correct A2/A3 — build spec
correctness first, but plan the module boundary so an off-main-thread/
speculative fast path can be added later without a rewrite.
([Blink parser dir](https://chromium.googlesource.com/chromium/src/+/master/third_party/blink/renderer/core/html/parser/),
[Gecko parser threading](https://udn.realityripple.com/docs/Mozilla/Gecko/HTML_parser_threading))

**CSS cascade & invalidation (A6/A7) — the most important precedent in
this whole document.** Gecko's *entire* modern style system was not built
inside Gecko. It was written from scratch in Rust as part of **Servo**
(Mozilla's separate research engine), using `rayon` for work-stealing
parallel cascade computation across every CPU core, then transplanted
wholesale into the C++ Gecko codebase in 2017 — replacing ~160k lines of
old C++ style code with ~85k lines of Rust ("Stylo," shipped as part of
Project Quantum, Firefox 57). Mozilla had tried to parallelize the old
C++ style system **twice before and failed both times**; it took a
memory-safe language's concurrency guarantees to make the parallel
rewrite tractable at all. Separately, Blink's answer to "don't recompute
style for the whole tree on every mutation" is `InvalidationSet`:
selectors are precompiled once into `RuleFeatureSet`, extracting which
DOM mutations *might* affect which elements, so a class/attribute change
only walks a provably-bounded subtree — deliberately over-invalidating
rather than chasing perfect minimality. **Takeaway for A6/A7:** treat the
cascade/invalidation engine as its own module from day one, specifically
because it's the one place both real engines found a straightforward
single-threaded implementation to be a genuine dead end, not just slow.
([Inside Quantum CSS](https://hacks.mozilla.org/2017/08/inside-a-super-fast-css-engine-quantum-css-aka-stylo/),
[Fearless Concurrency in Firefox Quantum](https://blog.rust-lang.org/2017/11/14/Fearless-Concurrency-In-Firefox-Quantum/),
[Blink style invalidation](https://chromium.googlesource.com/chromium/src/+/master/third_party/blink/renderer/core/css/style-invalidation.md))

### Track B — Layout & Graphics

**Fragment tree vs. frame tree (B1/B2/B9).** The two engines took opposite
architectural bets here, and both are worth knowing before choosing one.
Gecko's `layout/generic/` builds a **mutable frame tree** (`nsIFrame`
objects, roughly one-or-more per DOM node) and lays out via **reflow**:
dirty bits propagate down from reflow roots, each frame recomputes its own
box in place. Blink deliberately moved away from this exact model —
**LayoutNG** (now just "layout") produces an **immutable fragment tree**
per layout pass instead of mutating persistent frame state, specifically
because Blink's own old mutable-frame model (which was architecturally the
same idea as Gecko's) caused a long tail of invalidation and fragmentation
bugs that were hard to fix without this separation. This roadmap's B9
("fragment tree & display list") is written already assuming the LayoutNG
answer is the better one to start from — Gecko's design is the reason to
know why, and the counterexample to reach for if the immutable-fragment
approach hits a wall later.
([Blink LayoutNG doc](https://developer.chrome.com/docs/chromium/layoutng),
[Gecko Layout Overview](https://firefox-source-docs.mozilla.org/layout/LayoutOverview.html))

**Flexbox & Grid (B4/B5).** Both engines implement Grid's track-sizing as
a sibling of Flexbox's basis-resolution algorithm and **share the CSS Box
Alignment code** (`align-items`/`justify-content`/etc.) between the two
rather than reimplementing alignment per layout mode — worth mirroring:
don't scope B4 and B5 as fully independent phases for the alignment
subset, only for the sizing/placement algorithms that are genuinely
different.
([Igalia: CSS Grid on LayoutNG](https://blogs.igalia.com/jfernandez/2018/11/07/css-grid-on-layoutng-a-web-engines-hackfest-story/))

**Compositing & GPU paint (B11/B12) — the second Servo transplant.**
WebRender, Gecko's GPU renderer, has the *exact same origin story* as
Stylo: a from-scratch Rust rewrite prototyped standalone inside Servo,
merged into Gecko once it won on performance ("Quantum Render," also
Firefox 57). It restricts all GPU-device access to a single dedicated
render thread and pushes most rasterization/compositing/clipping work
onto the GPU rather than the CPU. Blink's independent answer to "how do
transforms/clips/scroll interact with compositing" is **property trees**
(introduced in RenderingNG) — transform/clip/effect/scroll state is kept
in its own tree, decoupled from a rigid layer hierarchy, so layerization
(which paint chunks get promoted to their own GPU-texture-backed layer) is
a separate, tunable decision made *after* painting rather than baked into
style resolution. Both conclusions point the same direction for B9-B12:
paint output (display list) and compositing decisions belong in clearly
separate stages, and (per the Stylo/WebRender pattern) compositing is a
second strong candidate — alongside the cascade — for prototyping in
isolation before integrating.
([WebRender](https://github.com/servo/webrender),
[RenderingNG architecture](https://developer.chrome.com/docs/chromium/renderingng-architecture))

### Track C — Script & Runtime

**Tiered execution, if going the from-scratch JS path (C4/C7).**
SpiderMonkey's real tiering ladder is Interpreter → **Baseline
Interpreter** (a hybrid: still bytecode-driven, but attaches Inline Caches
per call-site to speed up repeated ops without full compilation) →
**Baseline JIT** (compiles whole functions to native code, reusing the
same IC infrastructure) → **Warp** (the optimizing tier since Firefox 83,
which builds its optimizations by directly reading recorded IC data from
the lower tiers instead of running a separate profiling/recompilation
pass, unlike its predecessor IonMonkey). The concrete lesson for C7 if the
purist path is chosen: design the IC mechanism once, at the baseline tier,
and have every higher tier consume the *same* IC data rather than
re-deriving profile information independently per tier.
([Warp: improved JS performance](https://hacks.mozilla.org/2020/11/warp-improved-js-performance-in-firefox-83/))

**GC design (C6).** SpiderMonkey's GC is generational (separate nursery +
tenured heap, since most objects die young), incremental (sliced across
mutator execution so a GC pause doesn't stall the whole engine), and
occasionally compacting (rare, non-incremental, purely for
defragmentation) — a combination, not a single strategy. Blink's Oilpan
takes a different but related approach to the *DOM-wrapper* half of the
problem specifically: rather than refcounting across the JS/native
boundary (a classic cross-language leak source for cycles spanning both
heaps), Oilpan and V8 are **unified into one heap** with cross-component
tracing, so a cycle that spans a JS object and a C++ DOM node is still
collected correctly. If C8 (DOM↔JS bindings) is scoped with a boundary
GC at all, Oilpan's unified-heap answer is the one to copy the *shape* of,
even choosing a different concrete GC algorithm.
([SpiderMonkey GC docs](https://firefox-source-docs.mozilla.org/js/gc.html),
[Oilpan](https://v8.dev/blog/oilpan-library))

**DOM↔JS bindings (C8).** Blink doesn't hand-write bindings: every DOM/Web
API interface is declared in a **Web IDL** file, and a build-time code
generator emits all the V8↔C++ glue. Doing the same — schema-driven
codegen from IDL instead of hand-written bindings per interface — is what
makes C8 tractable at the hundreds-of-interfaces scale real spec parity
requires, instead of an ever-growing pile of one-off glue code.
([Web IDL in Blink](https://www.chromium.org/blink/webidl/))

### Track D — Platform & Storage

**Resource scheduling (D3).** Chromium's network stack (`net/`, fronted by
the sandboxed Network Service process) includes a **resource scheduler**
that actively reprioritizes and throttles requests during page load (e.g.
deferring low-priority requests, treating QUIC/HTTP2 streams differently
from plain HTTP) — this is easy to under-scope as "just fetch things" but
is a measurable, real performance feature worth its own attention in D3
rather than folding into D2's basic client.
([Network stack design doc](https://www.chromium.org/developers/design-documents/network-stack/))

**Embedding hard subproblems as isolated modules (D2/D6/D7).** Gecko's
HTTP/3 support runs through **neqo**, a separately-developed Rust QUIC
implementation embedded into the C++-majority `netwerk/` stack — the same
"isolated component, different language if it helps, integrate once
proven" pattern as Stylo/WebRender, just for a narrower subproblem. Same
logic applies to this roadmap's existing "bind, don't reimplement" stance
on TLS/codecs/WebRTC: it's not just a security-risk avoidance move, it's
the same architectural pattern the real engines use even for
non-security-critical hard subproblems.
([neqo](https://github.com/mozilla/neqo))

### Track E — Security & Privacy

**Process model (E3).** Firefox's evolution here is a useful staged
template: ship a single shared content process first ("Electrolysis"/e10s,
2016), then move to **Fission** (2021) — a separate OS process *per site*,
not just per tab, partly in direct response to Spectre/Meltdown-class
side-channel attacks making same-process cross-site data no longer safely
isolable in principle. Firefox also defines discrete **numbered sandbox
levels** (0 = least restrictive, increasing restriction per level) so
platform-specific hardening (seccomp-BPF on Linux, job objects/
AppContainer on Windows, Sandbox profiles on macOS) can roll out
incrementally without an all-or-nothing cutover — directly reusable as
E3's own rollout structure: ship unsandboxed, then ratchet up levels as
each restriction is verified not to break real content.
([Process Model](https://firefox-source-docs.mozilla.org/dom/ipc/process_model.html),
[Fission](https://hacks.mozilla.org/2021/05/introducing-firefox-new-site-isolation-security-architecture/),
[Chromium Linux sandboxing](https://chromium.googlesource.com/chromium/src/+/0e94f26e8/docs/linux_sandboxing.md))

### Track F — Product & Ops

**DevTools protocol (F1).** There are two real, divergent precedents, and
the choice matters beyond aesthetics. Chrome DevTools Protocol (CDP) is
JSON-RPC-like, organized into versioned **domains** (DOM, Debugger,
Network, Page, CSS...) with a stable subset plus an unstable
"tip-of-tree," and has a huge existing tool ecosystem (Puppeteer,
Playwright) that speaks it natively. Firefox's Remote Debugging Protocol
(RDP) predates CDP and takes an **actor-model** approach instead — every
debuggable object is an "actor" with its own protocol-defined request
types, packet-based over a socket. Implementing a CDP-compatible protocol
buys immediate compatibility with the existing automation-tool ecosystem;
implementing something RDP-shaped buys a cleaner internal architecture
with no external constraint. Decide explicitly rather than drifting into
a bespoke third design.
([CDP](https://chromedevtools.github.io/devtools-protocol/),
[Firefox RDP](https://firefox-source-docs.mozilla.org/devtools/backend/protocol.html))

**Accessibility tree (F2).** Both engines maintain the a11y tree as a
genuinely separate, incrementally-updated structure — not something
computed on demand from DOM+layout at query time. Blink's `AXObject` tree
is built via notification hooks threaded through DOM/layout/style code and
serialized cross-process to the browser (which builds a second,
platform-specific tree from it). Gecko's version is multi-process by
construction: each content process builds its own local a11y tree from
its DOM, and the parent process stitches all per-process local trees
(including cross-process iframes) into one coherent remote tree for
screen readers. **Directly relevant once E3's multi-process work lands:**
F2 should be scoped from the start as "a tree that gets built once per
process and stitched centrally," not retrofitted onto a single-process
assumption.
([Blink accessibility overview](https://chromium.googlesource.com/chromium/src/+/lkgr/docs/accessibility/overview.md),
[Gecko accessibility architecture](https://firefox-source-docs.mozilla.org/accessible/Architecture.html))

### The cross-cutting pattern worth internalizing before Track A even starts

Two of the highest-leverage precedents above aren't phase-specific at all:

1. **Multiple parallel trees, not one source of truth.** Real engines
   maintain DOM tree, fragment/frame tree, paint/property trees,
   compositor layer tree, and accessibility tree as independent,
   incrementally-updated structures, each with its own invalidation logic
   — never views recomputed on demand from a single tree. A toy engine
   that tries to derive everything from the DOM at read time (as
   `chrome-engine.html`, Phase A0, does) works at small scale and stops
   working the moment invalidation/incremental-update phases (A7, B9,
   F2) need to exist.
2. **The Stylo/WebRender pattern: prototype in isolation, integrate once
   proven.** Twice, Mozilla's answer to "this subsystem needs a
   fundamentally different approach than incremental patching of the
   existing C++ can achieve" was to build a clean-room rewrite as a
   standalone component in a different, safety/parallelism-friendly
   language, and only merge it in wholesale once it demonstrably beat the
   incumbent. For this project, the cascade/invalidation engine (A6/A7)
   and the compositor (B11/B12) are the two most likely candidates to hit
   that same wall — worth deciding up front whether either gets built as
   an isolated, swappable module for exactly that reason, rather than
   inline with the rest of its track.

---

## Milestone checkpoints (what "done enough to call it X" looks like)

- **"Renders a real static webpage correctly"** → A1-A7, B1-B11 done. No JS,
  no network beyond a basic fetch, but a saved real-world HTML/CSS page
  (e.g. a Wikipedia article) renders pixel-comparably to Firefox/Chrome.
- **"A usable minimal browser"** → adds D1-D3 (networking), C1-C2/C8 (DOM +
  bindings) + a JS engine (embedded or C3-C6), E1 (origin model). Can
  actually browse the interactive web, unsandboxed, single-process.
- **"Security-credible"** → adds E3 (real sandboxing/process isolation),
  E4 (fuzzing in CI), E6 (transport security). The point where "would I
  personally browse the untrusted web with this" becomes a fair question.
- **"Feature-parity replica"** → the rest of Tracks B/C/D (grid, workers,
  Wasm, media, device APIs) plus all of Track F. This is the "one-on-one"
  bar, and realistically the multi-year-team-effort end state the intro
  section is honest about.

---

## What to actually do next

**All of Track A (A1 through A10) is done** — see `engine/` and
`engine/README.md`. Real HTML tokenization (99.9%) and tree construction,
now including real SVG/MathML foreign-content namespace switching (66.2%
before A8/A9's foreign content → **77.5% overall / 86.9% excluding
documented gaps**) against the vendored html5lib-tests corpora; a real CSS
Syntax Level 3 tokenizer/parser and a real Selectors Level 4 engine; a
real cascade (origin/importance/specificity/source-order, custom
properties, `var()` substitution, defaulting keywords); a real mutable
CSSOM object graph with `getComputedStyle` and invalidation-set-driven
incremental restyling; and a real standalone XML 1.0 parser (`engine/crates/xml`,
new this phase) with full namespace resolution, used both directly and as
A8's standalone-SVG-document entry point (XSLT explicitly dropped, per
this document's own long-standing note that it's the most commonly cut
subsystem in real "shrink the engine" discussions). Unit-tested throughout;
WPT's JS-dependent suites (`css/*`, `svg/`, `mathml/`) are deferred to
Track C across the board, same reasoning since A4/A5.
`engine/crates/shell/src/main.rs` demonstrates the whole pipeline end to
end: parse HTML → real DOM, parse CSS → real stylesheet, match a real
selector, compute a real style, mutate the DOM and watch only the affected
node restyle, parse inline `<svg>` into real foreign content, and parse a
standalone XML document.

That's "renders a real static webpage correctly"'s content-and-style half
(Track A) fully done. Nothing in Track A blocks what comes next — Track B
(layout) only needs a styled tree, which has existed since A6 and is now
more complete with A8/A9's namespace-aware elements included.

**B1 (box tree generation) through B8 (writing modes & internationalized
layout) are all now started** in the new `engine/crates/layout` crate --
see their entries above for exactly what's real (display computation,
anonymous-box wrapping, list markers, real box-model geometry, margin
collapsing, line-breaking, float/clear, table row/column/colspan layout,
flexbox's grow/shrink/wrap/justify/align algorithms, grid's track sizing
+ occupancy-aware auto-placement, `position: relative`/`absolute`/
`fixed`, real multi-column balancing + forced breaks, and logical
margin/padding properties + `direction: rtl` inline mirroring) and what's
still a documented gap (`::before`/`::after`, font-shaping-accurate text
metrics, shrink-to-fit/intrinsic sizing, float-aware line narrowing,
table `rowspan`/`border-collapse`/`border-spacing`, flex `order`/`gap`/
column-direction wrap-and-stretch, grid `repeat()`/`minmax()`/subgrid/
`grid-row`, B6's simplified containing-block resolution plus no
`sticky`/`z-index`/stacking, B7's no within-child fragmentation/
`column-rule`, and B8's complete absence of vertical writing modes/UAX
#9 bidi -- the two biggest remaining Track B gaps by far). Remaining
work to fully close out B1-B8: pseudo-element matching in `css::cascade`
(needed for generated content), the float/line-narrowing refinement, the
several intrinsic-sizing-dependent gaps that recur across B2/B4/B5
(auto-width shrink-to-fit, flex's min-content shrink floor, table/grid's
min/max-content track sizing) -- worth tackling together once B10 (text
shaping) exists to actually measure content -- B6's real containing-block
resolution (needs ancestor position-type tracking threaded through the
layout recursion), and B8's vertical-writing-mode axis-agnostic rewrite
(a genuinely large undertaking, deferred rather than half-built or
faked).

**B9 (fragment tree & display list), B10 (text shaping & fonts), and B11
(painting & rasterization) are now all started too**, in a new
`engine/crates/paint` crate plus additions to `layout`. `layout::query`
adds real `getBoundingClientRect`/`elementFromPoint` equivalents that read
the fragment tree directly (B9), which first required fixing a real,
previously-latent bug: `flow::layout()` never anchored the *root*
fragment's own position, so a root element's own padding/border never got
folded into its or its descendants' coordinates — fixed by repositioning
the root at absolute `(0, 0)` before returning it, making the tree's
existing "single shared absolute coordinate space" design (already true
for every non-root fragment since B1/B2) actually hold for the whole tree.
`paint::display_list` lowers that fragment tree into a real `DisplayList`
(B9's other half), backed by a real CSS `<color>` parser. `layout::values`
adds a real proportional character-width table, replacing the old flat
per-character heuristic (B10, explicitly not real shaping — see its entry
above). `paint::raster` is a real software rasterizer producing an actual
RGBA8 `Canvas` with genuine scanline fill and Porter-Duff alpha
compositing, for `FillRect` items only — `DrawText` items are positioned/
colored correctly but not yet painted, since no glyph data exists (B11).
See each phase's own entry above for exactly what's real and what's a
documented gap. **B12 (compositing)** is the natural next Track B phase,
building on B9's display list and B11's rasterizer.
