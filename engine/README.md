# engine/

The real (as opposed to `chrome-engine.html`'s toy) implementation, per
[`ROADMAP.md`](../ROADMAP.md). **Track A (A1-A10) is fully done**: the
real WHATWG HTML tokenizer (99.9% on html5lib-tests), the real
tree-construction insertion-mode state machine + adoption agency algorithm
+ SVG/MathML foreign content (77.5% overall / 86.9% excluding documented
gaps on html5lib-tests tree-construction), a real CSS Syntax Level 3
tokenizer/parser, a real Selectors Level 4 engine, a real cascade +
computed-value pipeline, a real mutable CSSOM with `getComputedStyle` and
invalidation-set-driven incremental restyling, and a real standalone XML
1.0 parser (`crates/xml`) with namespace resolution (XSLT explicitly
dropped, per `ROADMAP.md`'s own long-standing note on it). B1 (box tree
generation, Track B's start) is the open next step -- see `ROADMAP.md`'s
"What to actually do next".

## Layout

```
engine/
  Cargo.toml                 workspace manifest
  crates/
    dom/                     tree representation shared by html/css/xml/layout/js_bindings (C1's future home)
    html/                    A2 (tokenizer) + A3 (tree construction) + A8/A9 (SVG/MathML foreign content) -- all done
    css/                     A4-A7 (tokenizer/parser, selectors, cascade, CSSOM) -- all done
    xml/                     A10 (standalone XML 1.0 parser + namespace resolution) -- done; also A8's standalone-SVG-document entry point
    layout/                  B1-B9 (box tree -> fragment tree) -- currently placeholders
    paint/                   B10-B12 (text shaping, rasterization, compositing) -- currently placeholders
    js_bindings/              Track C -- placeholder, shape depends on "the JS engine question"
    net/                     D1-D3 (URL parsing, networking, resource loading) -- currently placeholders
    media/                   D5-D7 (images, audio/video, WebRTC) -- currently empty
    a11y/                    F2 (accessibility tree) -- currently placeholders
    devtools/                F1 (inspector protocol) -- currently placeholders
    shell/                   binary crate; a real HTML->DOM->CSS->selector-match->cascade->getComputedStyle->incremental-restyle->foreign-content->XML pipeline smoke test (layout/paint stages are still placeholders)
    html5lib_harness/        A2/A3/A8/A9's conformance harness (see below)
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
# standalone XML -> layout -> paint -- everything through A10 is real now;
# layout/paint are still placeholders)
cargo run -p shell

# Run the html5lib-tests tokenizer conformance harness
cargo run --release -p html5lib_harness

# Run the html5lib-tests tree-construction conformance harness
cargo run --release -p html5lib_harness --bin tree_construction_harness
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
than depending on GitHub being reachable at test time.

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
  (`foreign-fragment.dat`, `template.dat`, `processing-instructions.dat`)
  for a cleaner in-scope signal
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

## Why placeholders instead of nothing

Every non-trivial crate here (`html`, `css`, `layout`, `paint`, `net`,
`a11y`, `devtools`) ships a type or function shaped like its eventual real
API, deliberately not implemented yet. Two reasons: (1) later crates that
depend on it (e.g. `layout` on `css`, `paint` on `layout`) have something
real to compile against instead of everything landing in one giant
first-implementation PR, and (2) it makes "what's actually here vs. what's
aspirational" impossible to blur — every placeholder says so in its own doc
comment, right next to the roadmap phase that replaces it.
