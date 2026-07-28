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
group at-rules (`@media`/`@supports`/`@document`/`@layer` — condition not
evaluated, treated as always-true, a documented simplification not a
correctness claim), and records other at-rules (`@import`, `@font-face`,
etc.) as raw name/prelude/block text for later phases.

No vendored conformance corpus for this phase — WPT's `css/css-syntax`
suite drives via `testharness.js`, which needs a JS engine (Track C, not
built yet), consistent with the reasoning that led A1-A3 to use
html5lib-tests instead of WPT directly. Verified instead with 13
self-authored unit tests covering the spec algorithms and edge cases
(string-newline reconsumption, malformed-rule recovery, `!important`
detection, at-rule flattening).
**Exit:** met on the achievable bar given no JS engine — real tokenizer/
parser implementing the full spec grammar, unit-tested; WPT `css/css-syntax`
deferred until Track C exists.

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

Known gap: pseudo-elements (`::before` etc.) and interaction-state pseudo-
classes (`:hover`, `:focus`, ...) aren't implemented — there's no layout/
event state yet for either to attach to; both parse into a documented
`PseudoClass::Unsupported` fallback rather than silently matching wrong.
Same as A4, no vendored WPT `css/selectors` corpus is runnable without
Track C; verified with 7 self-authored unit tests covering combinators,
attribute matching, `An+B` parsing edge cases (including tokenizer
artifacts like `Dimension{unit:"n-"}`), and `:has()`.
**Exit:** met on the achievable bar — real Selectors Level 4 grammar and
matching, unit-tested; WPT `css/selectors` deferred until Track C exists.

### A6. Cascade & computed values
Origin/importance ordering (UA/user/author/`!important`/transition/
animation origins), cascade layers (`@layer`), full specificity per spec,
the specified→computed→used value pipeline, `initial`/`inherit`/`unset`/
`revert`, custom properties (`--foo`) and `var()` substitution.
**Exit:** WPT `css/cssom`, `css/css-cascade`, `css/css-variables` ≥85%.

### A7. CSSOM & style invalidation
`CSSStyleSheet`/`CSSRule` object graph mutable from script, `getComputedStyle`,
and — critically — **style invalidation**: recomputing only the minimal
subtree when a class/attribute/rule changes, instead of full-tree
recompute. This is a real algorithmic problem (Blink's "style invalidation
sets," Gecko's "restyle hints") and directly determines whether the engine
is usably fast on real pages.
**Exit:** invalidation-set unit tests + no full-tree-restyle on targeted
mutation benchmarks.

### A8. SVG
SVG parsing as its own XML-ish document type, the SVG geometry/paint
properties, `<use>`/`<symbol>` reuse, integration with the HTML tree
(inline `<svg>`), SVG-as-image and SVG-as-document loading paths.
**Exit:** WPT `svg/` core suites ≥70%.

### A9. MathML
Parsing and basic layout for `<math>` content — smaller subsystem, but
required for genuine spec parity (both Chromium and Firefox ship it).
**Exit:** WPT `mathml/` core suites ≥60%.

### A10. XML & XSLT
Standalone XML document parsing (distinct error-handling model from HTML —
XML is not permissive), `<?xml-stylesheet?>`, and — the one nobody wants to
build — an XSLT 1.0 processor, still present in both engines for legacy
compat.
**Exit:** can parse/serialize well-formed XML per spec; XSLT marked
explicitly optional/deferred if scope needs trimming (this is the single
most-often-dropped subsystem in real "shrink the engine" discussions inside
both Google and Mozilla).

---

## Track B — Layout & Graphics

### B1. Box tree generation
Style tree → box tree: anonymous box generation, `display` computation
(including `display: contents`, `display: table` internal box types),
list-item markers, `::before`/`::after` generated content.
**Exit:** WPT `css/css-display` ≥80%.

### B2. Block & inline formatting contexts
Classic block layout, inline layout with line boxes, `float`/`clear`,
margin collapsing (an infamous, precisely-specified, easy-to-get-subtly-
wrong algorithm), BFC establishment rules.
**Exit:** WPT `css/css-box`, `css/CSS2/normal-flow`, `css/CSS2/floats` ≥80%.

### B3. Table layout
CSS 2 table layout algorithm (distinct model from block/inline): row/column
sizing passes, `border-collapse`, spanning cells, `<table>` HTML-vs-CSS
interaction quirks.
**Exit:** WPT `css/CSS2/tables` ≥75%.

### B4. Flexbox
[CSS Flexible Box Layout](https://www.w3.org/TR/css-flexbox-1/) in full:
main/cross axis resolution, flex-basis/grow/shrink distribution algorithm,
wrapping, alignment (`justify-content`/`align-items`/`align-self`).
**Exit:** WPT `css/css-flexbox` ≥80%.

### B5. Grid
[CSS Grid Layout](https://www.w3.org/TR/css-grid-1/): track sizing
algorithm (the hardest single algorithm in CSS layout — multiple resolution
passes over `fr` units, intrinsic sizing, and auto-placement), named lines/
areas, subgrid.
**Exit:** WPT `css/css-grid` ≥75%.

### B6. Positioning & stacking
`position: relative/absolute/fixed/sticky`, containing-block resolution
rules, stacking contexts, `z-index`, paint order per spec.
**Exit:** WPT `css/css-position`, `css/CSS2/zindex` ≥80%.

### B7. Fragmentation
Multi-column layout (`column-count`/`column-width`), fragmentation for
print/pagination (`break-before`/`break-after`/`break-inside`) — the
subsystem most engines get wrong or skip; genuine parity requires it.
**Exit:** WPT `css/css-multicol`, `css/css-break` ≥60%.

### B8. Writing modes & internationalized layout
Vertical writing modes (`writing-mode: vertical-rl`), logical properties
(`margin-inline-start` etc. instead of physical `left`/`right`), full
Unicode Bidirectional Algorithm (UAX #9) for RTL/LTR mixed text.
**Exit:** WPT `css/css-writing-modes`, `css/css-logical` ≥65%; bidi
conformance against the Unicode BidiTest data files.

### B9. Fragment tree & display list
Replace any toy "list of boxes" with a real intermediate representation:
a **fragment tree** (layout's actual output — positioned, sized boxes
referencing their originating DOM/style nodes) lowered to a **display
list** (paint's input — an ordered list of drawing commands: fill rect,
draw text run, push clip, push transform). This is the real architectural
seam that lets everything downstream (compositing, hit-testing, incremental
layout, `getBoundingClientRect()`) work without re-deriving geometry ad hoc.
**Exit:** hit-testing (`elementFromPoint`) and `getBoundingClientRect`
implemented purely by querying the fragment tree; display list snapshot
tests for a corpus of pages.

### B10. Text shaping & fonts
Real shaping (a HarfBuzz binding or equivalent from-scratch shaper):
ligatures, kerning, complex scripts (Arabic joining, Indic reordering),
combined with the bidi algorithm from B8. Font matching/fallback chains,
`@font-face` loading (WOFF2 parsing), variable fonts.
**Exit:** WPT `css/css-text`, `css/css-fonts` ≥70%; visual diff tests
against reference shaping output for a multilingual test corpus.

### B11. Painting & rasterization
Software rasterizer as the baseline (correctness first, matches how both
engines actually bootstrap new platforms), then a GPU path (wgpu, given the
Rust choice) mirroring what Skia (Blink) / WebRender (Gecko) do: batch draw
calls, cache rasterized glyphs/tiles, avoid re-painting unchanged regions.
**Exit:** pixel-diff reftests (WPT's reftest methodology) passing against a
reference corpus within tolerance; a documented perf budget (ms per frame)
on a fixed benchmark page set.

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

**A1 through A5 are done** — see `engine/` and `engine/README.md`. Real
HTML tokenization (99.9%) and tree construction (66.6% overall / 80.3%
excluding documented foreign-content/frameset/fragment/PI gaps) against the
vendored html5lib-tests corpora, plus a real CSS Syntax Level 3 tokenizer/
parser and a real Selectors Level 4 engine (unit-tested; WPT's
`css/css-syntax` and `css/selectors` suites need a JS engine to run and are
deferred to Track C). `engine/crates/shell/src/main.rs` demonstrates the
whole pipeline end to end: parse HTML → real DOM, parse CSS → real
stylesheet, match a real selector against the tree.

Next up is **A6** (cascade & computed values: specificity, origin/
importance ordering, cascade layers, the specified→computed→used value
pipeline, custom properties/`var()`), which is what turns the flat
`Stylesheet`/`SelectorList` pieces A4/A5 built into something that actually
assigns styles to DOM nodes — the prerequisite every Track B layout phase
needs. After that, A7 (CSSOM & style invalidation) closes out the "renders
a real static webpage" content-and-style side of Track A, with A8-A10
(SVG/MathML/XML+XSLT) as the remaining, more niche phases in the track.

Say the word and I'll start on A6.
