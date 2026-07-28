# engine/

The real (as opposed to `chrome-engine.html`'s toy) implementation, per
[`ROADMAP.md`](../ROADMAP.md). A1 (project foundation) and A2 (the real
WHATWG HTML tokenizer) are done — the tokenizer passes 99.9% of the vendored
html5lib-tests suite. A3 (tree construction) is next.

## Layout

```
engine/
  Cargo.toml                 workspace manifest
  crates/
    dom/                     tree representation shared by html/css/layout/js_bindings (C1's future home)
    html/                    A2 (tokenizer, done) + A3 (tree construction, still a placeholder)
    css/                     A4-A7 (parser/cascade/CSSOM) -- currently placeholders
    layout/                  B1-B9 (box tree -> fragment tree) -- currently placeholders
    paint/                   B10-B12 (text shaping, rasterization, compositing) -- currently placeholders
    js_bindings/              Track C -- placeholder, shape depends on "the JS engine question"
    net/                     D1-D3 (URL parsing, networking, resource loading) -- currently placeholders
    media/                   D5-D7 (images, audio/video, WebRTC) -- currently empty
    a11y/                    F2 (accessibility tree) -- currently placeholders
    devtools/                F1 (inspector protocol) -- currently placeholders
    shell/                   binary crate; today just a CLI smoke test wiring every stage together
    html5lib_harness/        A2/A3's conformance harness (see below)
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

# Run the pipeline smoke test (html -> dom -> css -> layout -> paint --
# tokenizer is real now, everything after it is still a placeholder)
cargo run -p shell

# Run the html5lib-tests conformance harness
cargo run --release -p html5lib_harness
```

## The html5lib-tests harness

`crates/html5lib_harness` is **not** a full WPT (web-platform-tests) runner
— WPT tests are `testharness.js` scripts that need a working JS engine and
DOM to execute, and Track C doesn't exist yet. html5lib-tests' plain-JSON
tokenizer/tree-construction test format doesn't need any of that, which is
exactly why A2/A3 use it as their exit criterion (`>=95%` pass rate) instead
of WPT directly. A real WPT harness becomes possible once C1 (DOM) and a JS
engine (C3-C8) exist to actually run `testharness.js` against.

Test files are vendored under `crates/html5lib_harness/vendor/tokenizer/`
(fetched from
[html5lib/html5lib-tests](https://github.com/html5lib/html5lib-tests)) so
CI and local runs work offline and reproducibly, rather than depending on
GitHub being reachable at test time.

**Current: 99.9% (6708/6713) passing**, against A2's >=95% exit criterion.
The 5 remaining failures are a documented gap (ScriptData's escaped/
double-escaped states aren't implemented -- see `tokenizer.rs`'s module
docs) plus 3 cases in `xmlViolation.test` that test a separate XML5
character-validation mode standard HTML tokenization doesn't apply.

The harness supports per-test `initialStates` (some tests must run starting
in RCDATA/RAWTEXT/PLAINTEXT/CDATA-section state rather than the default
Data state) and `lastStartTag` (primes the "appropriate end tag" check for
RCDATA/RAWTEXT/ScriptData, which normally only matters once a tree builder
exists to track it), both used by `html::tokenize_with()`.

## Why placeholders instead of nothing

Every non-trivial crate here (`html`, `css`, `layout`, `paint`, `net`,
`a11y`, `devtools`) ships a type or function shaped like its eventual real
API, deliberately not implemented yet. Two reasons: (1) later crates that
depend on it (e.g. `layout` on `css`, `paint` on `layout`) have something
real to compile against instead of everything landing in one giant
first-implementation PR, and (2) it makes "what's actually here vs. what's
aspirational" impossible to blur — every placeholder says so in its own doc
comment, right next to the roadmap phase that replaces it.
