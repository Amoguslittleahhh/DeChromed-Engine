# engine/

The real (as opposed to `chrome-engine.html`'s toy) implementation, per
[`ROADMAP.md`](../ROADMAP.md). **Track A (A1-A10) is fully done**: the
real WHATWG HTML tokenizer (99.9% on html5lib-tests), the real
tree-construction insertion-mode state machine + adoption agency algorithm
+ SVG/MathML foreign content (77.5% overall / 86.9% excluding documented
gaps on html5lib-tests tree-construction), a real CSS Syntax Level 3
tokenizer/parser (also implementing the [CSS Nesting Module](https://www.w3.org/TR/css-nesting-1/)),
a real Selectors Level 4 engine (plus `:lang()`/`:dir()`), a real cascade +
computed-value pipeline (with real [Cascade Layers](https://www.w3.org/TR/css-cascade-5/#layering)
support), a real mutable CSSOM with `getComputedStyle` and
invalidation-set-driven incremental restyling, and a real standalone XML
1.0 parser (`crates/xml`) with namespace resolution (XSLT explicitly
dropped, per `ROADMAP.md`'s own long-standing note on it).

**Track B (layout) is now started**: B1 (box tree generation) through B8
(writing modes & internationalized layout) all have a real, tested first
landing in `crates/layout` -- `display` computation, anonymous-box
generation, list markers, real box-model geometry, CSS2.1 margin
collapsing, line-breaking, `float`/`clear`, table row/column/`colspan`
layout, a real (row + column direction) flexbox grow/shrink/wrap/
`justify-content`/`align-items` implementation, grid track sizing +
occupancy-aware auto-placement, `position: relative`/`absolute`/`fixed`,
real multi-column balancing with forced `break-before`/`-after`, and
logical margin/padding properties + `direction: rtl` inline-content
mirroring. See `ROADMAP.md`'s B1-B8 entries for exactly what's real vs.
a documented gap (`::before`/`::after` generated content, the several
gaps that trace back to "no intrinsic sizing yet", B6's simplified
containing-block resolution, and B8's complete absence of vertical
writing modes are the biggest ones -- **B8's other big gap, UAX #9 bidi,
is now real, see below**).

**B9 (fragment tree & display list) is started**, in a new `crates/paint`
crate plus additions to `crates/layout`. `layout::query` implements real
`getBoundingClientRect`/`elementFromPoint` equivalents by reading the
fragment tree's already-absolute coordinates directly; `paint::
display_list` lowers that tree into a real `DisplayList` of draw commands,
backed by a real CSS `<color>` parser.

**B8 (bidi/line-breaking), B10 (text shaping & fonts), and B11 (painting &
rasterization) were upgraded to real, externally-audited implementations**,
replacing their own earlier hand-rolled approximations. A new
`crates/text` does real font shaping via [`rustybuzz`](https://github.com/harfbuzz/rustybuzz)
(a complete Rust port of HarfBuzz) and real glyph outline extraction via
[`ttf-parser`](https://github.com/RazrFalcon/ttf-parser), against one
embedded, freely-licensed font (`assets/fonts/DejaVuSans.ttf`) --
`layout::values::text_width_px` now measures text with real shaped
advances instead of a per-character width-ratio table (B10).
`layout::flow` now runs the real Unicode Bidirectional Algorithm (via
`unicode-bidi`) and real UAX #14 line-breaking (via `unicode-linebreak`)
instead of "mirror the whole line if RTL" and "split only on whitespace"
(B8). `paint::raster` is a real software rasterizer producing an actual
RGBA8 pixel buffer with genuine scanline fill and Porter-Duff alpha
compositing, for `FillRect` items *and now real glyph rasterization* --
`DrawText` items are shaped and their real glyph outlines filled with the
same nonzero-winding algorithm `canvas2d::fill` uses for arbitrary paths
(B11, this phase's former headline gap, now closed). A `wgpu` GPU path
was evaluated for B11/B12 and confirmed infeasible in this sandboxed
environment (no GPU adapter available at all) rather than simply
unattempted -- see `ROADMAP.md`'s B11 entry.

**B12 (compositing) and B13 (Canvas 2D & WebGL/WebGPU) are started too.**
`paint::compositor` is a real (single-threaded, software) layer
compositor: a `Layer` bundles a `DisplayList` with a translate/uniform-
scale/opacity transform, and `Canvas::composite_over` (added to B11's own
`raster.rs`) does the real per-pixel compositing work -- rasterizing each
layer's display list exactly once, then compositing back-to-front with the
same real Porter-Duff blending `FillRect` already uses (B12; no compositor
thread or GPU path yet). `paint::canvas2d` implements real Canvas 2D
graphics primitives -- path building, scanline polygon fill using the
nonzero winding rule (so a donut/ring shape with an inner and outer
subpath wound opposite ways renders a real hole), Bresenham line
stroking, and `ImageData` get/put (B13's Canvas 2D half; WebGL/WebGPU
haven't started at all, and nothing here is wired to the `<canvas>` DOM
element or JS yet -- Track C's own JS binding is still ahead). See
`ROADMAP.md`'s B9-B13 entries for exactly what's real vs. a documented
gap in each. `crates/shell/src/main.rs` runs the real pipeline end to
end, including layout, fragment-tree queries, display-list lowering,
rasterization, compositing a layer, and a standalone Canvas 2D
fill/stroke.

**Track C (script & runtime) is started too: C1 (the addressable DOM
API) has a real first landing**, in `crates/dom`'s new `api.rs`/
`class_list.rs` modules plus `crates/css`'s new `query.rs`. Real
spec-shaped `Node`/`Element` methods (`nodeType`/`nodeName`, sibling/
element navigation, `textContent` get/set, `removeChild`/`replaceChild`,
`cloneNode(deep)`, `contains`/`isConnected`, `getElementById`/
`getElementsByTagName`), a real live `classList` backed directly by the
`class` attribute, and `querySelector`/`querySelectorAll`/`closest`
built on A5's existing selector matcher. Every query here takes
`&Document` explicitly and does a real, uncached traversal on each call
-- there's no separate `NodeList`/`HTMLCollection` caching layer to keep
in sync, because Rust's own borrow checker already forbids holding a
stale reference across a mutation, which reproduces the DOM spec's
"live" requirement without needing a mechanism to achieve it (see
`dom::api`'s own module doc for the full argument). Known gaps: no
`Range`/`Selection`/`MutationObserver`/cross-document `adoptNode`, and no
JS binding reaches any of this yet (that's C2/C3+). See `ROADMAP.md`'s
C1 entry for the rest.

The workspace targets Rust **edition 2024** (`engine/Cargo.toml`), using
stable let-chains (`if let X = y && let A = b { ... }`) where they read
better than nested `if let`s.

## Layout

```
engine/
  Cargo.toml                 workspace manifest
  crates/
    dom/                     tree representation (A2/A3+) plus C1's real addressable DOM API (api.rs/class_list.rs) -- started
    html/                    A2 (tokenizer) + A3 (tree construction) + A8/A9 (SVG/MathML foreign content) -- all done
    css/                     A4-A7 (tokenizer/parser, selectors, cascade, CSSOM) -- all done; C1's query.rs (querySelector/querySelectorAll/closest) also started
    xml/                     A10 (standalone XML 1.0 parser + namespace resolution) -- done; also A8's standalone-SVG-document entry point
    layout/                  B1-B8 (box tree, block/inline layout, tables, flexbox, grid, positioning, multi-column, logical properties/RTL + real UAX #9 bidi/UAX #14 line-breaking) started; B9's query.rs (fragment-tree queries) also started
    text/                    B10 (real font shaping via rustybuzz + glyph outlines via ttf-parser, one embedded font) -- new crate, shared by layout (measurement) and paint (glyph painting)
    paint/                   B9 (display-list lowering + color parsing), B11 (software rasterizer, now with real glyph rasterization), B12 (layer compositor), and B13 (Canvas 2D primitives) all started; B13's WebGL/WebGPU half not started (see ROADMAP.md's B11 entry for why -- no GPU adapter in this environment)
    js_bindings/              Track C's JS<->DOM binding layer -- placeholder, shape depends on "the JS engine question"; C1 itself (the DOM API this would bind to) now lives in dom/ and css/ instead
    net/                     D1-D3 (URL parsing, networking, resource loading) -- currently placeholders
    media/                   D5-D7 (images, audio/video, WebRTC) -- currently empty
    a11y/                    F2 (accessibility tree) -- currently placeholders
    devtools/                F1 (inspector protocol) -- currently placeholders
    shell/                   binary crate; a real HTML->DOM->CSS->selector-match->cascade->getComputedStyle->incremental-restyle->foreign-content->XML->box-tree->layout->fragment-tree-query->display-list->raster (incl. real text)->compositor pipeline smoke test, plus a standalone Canvas 2D demo
    html5lib_harness/        A2/A3/A8/A9's conformance harness (see below)
  assets/
    fonts/                   the one embedded font (DejaVu Sans) crates/text ships -- see its own README.md for licensing
```

Every placeholder crate's `lib.rs` doc comment says which roadmap phase
replaces it and links back to the relevant `ROADMAP.md` section — that's
intentional so `cargo doc --workspace --open` (or just reading the crate
docs) gives a phase-by-phase index of what's left to build, not just what's
here.

## Running things

```sh
cd engine

# Build everything
cargo build --workspace

# Run the unit tests every crate ships
cargo test --workspace

# Run the pipeline smoke test (html -> dom -> css -> selectors -> cascade
# -> getComputedStyle -> incremental restyle -> SVG foreign content ->
# standalone XML -> layout -> fragment-tree queries -> display list ->
# raster -> compositor, plus a standalone Canvas 2D fill/stroke demo --
# everything through A10 is real, B1-B13 have real first landings too;
# see ROADMAP.md for exactly what's still a documented gap in each)
cargo run -p shell

# Run the html5lib-tests tokenizer conformance harness
cargo run --release -p html5lib_harness

# Run the html5lib-tests tree-construction conformance harness
cargo run --release -p html5lib_harness --bin tree_construction_harness

# Run the cross-crate crash/hang stress-test fuzzer (see below)
cargo run --release -p html5lib_harness --bin stress_test
```

## The html5lib-tests harnesses

`crates/html5lib_harness` is **not** a full WPT (web-platform-tests) runner
— WPT tests are `testharness.js` scripts that need a working JS engine and
DOM to execute, and Track C doesn't exist yet. html5lib-tests' plain-JSON/
`.dat` tokenizer/tree-construction test formats don't need any of that,
which is exactly why A2/A3 use them as their exit criteria instead of WPT
directly. A real WPT harness becomes possible once C1 (DOM) and a JS engine
(C3-C8) exist to actually run `testharness.js` against.

Test files are vendored under `crates/html5lib_harness/vendor/tokenizer/`
and `crates/html5lib_harness/vendor/tree-construction/` (fetched from
[html5lib/html5lib-tests](https://github.com/html5lib/html5lib-tests) and
the [WPT mirror](https://github.com/web-platform-tests/wpt) it moved to,
respectively) so CI and local runs work offline and reproducibly, rather
than depending on GitHub being reachable at test time. The vendored
corpora were refreshed from upstream after A1-A10 landed; the only new
files that matter for scoping are `scripted_*.dat` tree-construction
files, which need live JS execution (`document.write`/`setAttribute`)
during parsing to produce their expected trees -- out of scope without
Track C, so `HARNESS_SKIP_KNOWN_GAPS=1` skips them alongside the
pre-existing known-gap files.

**Tokenizer: 99.9% (6708/6713) passing**, against A2's >=95% exit
criterion. The 5 remaining failures are a documented gap (ScriptData's
escaped/double-escaped states aren't implemented -- see `tokenizer.rs`'s
module docs) plus 3 cases in `xmlViolation.test` that test a separate XML5
character-validation mode standard HTML tokenization doesn't apply.

The tokenizer harness supports per-test `initialStates` (some tests must
run starting in RCDATA/RAWTEXT/PLAINTEXT/CDATA-section state rather than
the default Data state) and `lastStartTag` (primes the "appropriate end
tag" check for RCDATA/RAWTEXT/ScriptData), both used by
`html::tokenize_with()`.

**Tree construction: 77.5% (1344/1734) overall, 86.9% (1303/1499) excluding
known-gap files**, against A3/A8/A9's exit criteria (A8/A9's real
foreign-content implementation moved this from 66.2% overall when A3 alone
handled it). The remaining gaps are `<frameset>` documents, fragment-
context parsing (`foreign-fragment.dat`), `<template>` content documents
(`template.dat`), and the ProcessingInstruction-node serialization
convention (`processing-instructions.dat`) -- all documented in
`tree_builder.rs`'s module docs. The `tree_construction_harness` binary
(`crates/html5lib_harness/src/tree_construction.rs`) is controlled by
env vars:

- `HARNESS_DEBUG=1` -- print per-file and per-test-case progress to stderr
- `HARNESS_MAX_FAILURES=N` -- stop after N failures (default: no limit)
- `HARNESS_SKIP_KNOWN_GAPS=1` -- skip the known-gap `.dat` files
  (`foreign-fragment.dat`, `template.dat`, `processing-instructions.dat`,
  and any `scripted_*.dat` file) for a cleaner in-scope signal
- `HARNESS_ONLY_FILE=name.dat` -- run just one vendored file
- `HARNESS_PER_FILE=1` -- print a pass/fail breakdown per file

Each test case runs under `catch_unwind` (with a silenced panic hook) so
one bad case can't hide the aggregate pass rate.

## The XML crate

`crates/xml` (A10) is a real, non-validating XML 1.0 parser used two ways:
directly (`xml::parse_document`) for standalone XML documents, and via
`xml::svg::parse_svg_document` (A8) for standalone `.svg` files, which
defaults any element left with no resolved namespace to SVG's. It has no
vendored conformance corpus (WPT doesn't ship a standalone bare-XML-parsing
test suite the way it does for HTML/CSS) -- verified with 11 self-authored
unit tests instead, covering the well-formedness fatal-error cases XML's
parsing model requires (mismatched tags, duplicate attributes, undeclared
entities, multiple root elements) alongside the ordinary parsing/namespace-
resolution/round-trip-serialization cases.

## The stress-test fuzzer

`crates/html5lib_harness/src/bin/stress_test.rs` is a cross-crate,
conformance-blind fuzzer for Track A (A1-A10) and now Track B's
`layout::layout`, `paint::build_display_list`/`paint::rasterize` (which now
also exercises real glyph shaping/rasterization on every fuzzed page with
text), `paint::composite_layers`, `paint::canvas2d`'s fill/stroke/
`ImageData`, and `text::shape`/`text::glyph_outline` directly too: it
doesn't check parser *output* against an expected answer (that's the
tokenizer/tree-construction harnesses above), only that
`html::parse_document`, `css::parse_stylesheet`,
`css::selectors::parse_selector_list`, `xml::parse_document`, the
cascade/computed-style pipeline, `layout::build_box_tree`/`layout::layout`
(at several containing-block widths, including pathologically narrow/zero
ones), the same fragment trees lowered through `paint::build_display_list`
and rasterized via `paint::rasterize`, `paint::composite_layers` fed
randomized layer counts/offsets/scales/opacities (including deliberately
out-of-range values), `paint::canvas2d` fed randomized path point
sequences (mixed move/line/close, coordinates far outside the canvas), and
`text::shape`/`text::glyph_outline` fed random text across a much broader
Unicode range (Hebrew, Arabic, combining marks, CJK, emoji) than the
HTML/CSS fuzzers' own ASCII-biased alphabets reach, all return *something*
(or a graceful `Err`) instead of panicking, hanging, or aborting the
process, across:

- **Truncation fuzzing**: every prefix length of a handful of realistic
  seed documents -- a surprisingly effective way to hit boundary
  conditions (a tag cut off mid-attribute, a string cut off before its
  closing quote, ...).
- **Mutation fuzzing**: random insert/delete/replace edits applied
  repeatedly to those same seeds.
- **Random-byte-soup fuzzing**: fully random strings from an alphabet
  biased toward each format's own syntactically load-bearing characters.
- **A targeted adoption-agency generator**: randomly-misnested
  `<a>`/`<b>`/`<div>`-style soup, the specific shape that exercises A3's
  adoption agency algorithm (uniform random bytes rarely produce enough
  of this shape by chance).
- **End-to-end cascade fuzzing**: a mutated HTML seed parsed into a real
  DOM, a mutated CSS seed parsed into a real stylesheet, then the full
  cascade + computed-style pass run over every element.

A deterministic seeded PRNG (splitmix64, no external `rand` dependency)
makes every run reproducible from `STRESS_SEED` (default a fixed constant);
`STRESS_ITERATIONS` controls how many mutation/random-fuzz iterations run
per category (default 20,000). This tool found and helped fix 6 real bugs
during Track A's post-completion hardening pass -- see `ROADMAP.md`'s
"Track A hardening pass" section for what they were -- and now runs clean
(0 distinct failures) across many seeds; run it yourself after any change
to a Track A parser, especially anything touching recursion or index
arithmetic.

## Why placeholders instead of nothing

Every non-trivial crate here (`html`, `css`, `layout`, `paint`, `net`,
`a11y`, `devtools`) ships a type or function shaped like its eventual real
API, deliberately not implemented yet. Two reasons: (1) later crates that
depend on it (e.g. `layout` on `css`, `paint` on `layout`) have something
real to compile against instead of everything landing in one giant
first-implementation PR, and (2) it makes "what's actually here vs. what's
aspirational" impossible to blur — every placeholder says so in its own doc
comment, right next to the roadmap phase that replaces it.
