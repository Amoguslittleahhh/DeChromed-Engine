# DeChromed Engine — Roadmap to a Real Browser Engine

## Ground truth, stated up front

Blink (Chromium) and Gecko (Firefox) are ~15-25 million lines of C++/Rust each,
built by hundreds of engineers over 20+ years. Mozilla's own from-scratch
attempt at a modern engine — Servo, in Rust — has consumed dozens of
person-years and still isn't a full consumer replacement for Gecko. The
[Ladybird](https://ladybird.org) project (an independent from-scratch browser,
started 2019) is the closest real-world precedent for "build a browser engine
from zero" and it runs on a paid team plus a large open-source community, over
multiple years, and is still pre-1.0.

So: this roadmap is real, but the honest unit of measurement is **phases
measured in weeks-to-months each**, not a single build. `chrome-engine.html`
is Phase 0 — a working toy that proves the pipeline shape. Everything below
replaces one toy piece at a time with a spec-correct one, in a real multi-file
project, until what's left resembles an actual engine architecture.

Treat each phase as a milestone with its own PR(s), tests, and a demo. Do not
start a phase before the previous one has working tests — this project dies
the moment "mostly working" piles up without verification, same as any real
engine team's approach (both Blink and Gecko live and die by their test
suites: web-platform-tests, WPT).

---

## Phase 0 — Toy pipeline (done)

`chrome-engine.html`: hand-rolled tokenizer → DOM → CSSOM → cascade → block/
inline layout → canvas paint, wrapped in a fake Chrome UI. No JS execution,
no networking, ~10 CSS properties, no real HTML5 parsing algorithm. Proves
the pipeline shape and nothing else. Good for demos, useless as a foundation
to build directly on top of (single file, no module boundaries, canvas
painting is not how real engines represent output).

**Exit criterion:** already met. Kept as a reference/demo artifact, not
extended further — Phase 1 starts a real project structure.

---

## Phase 1 — Project foundation & language choice

Decide the real substrate before writing more engine code.

- **Recommendation: Rust.** Memory safety without a GC (matches how a
  DOM/layout tree with lots of aliasing needs to behave), first-class WASM
  target if you ever want it running in-browser again as a demo, and it's
  the same choice Servo made for exactly this kind of project. C++ is the
  "authentic" choice (what Blink/Gecko are actually written in) but buys you
  a much larger footgun surface for no real benefit at this project's scale.
- Set up a real workspace: separate crates/modules for `html`, `css`, `dom`,
  `layout`, `paint`, `net`, `js` (stub for now) — mirrors how Servo and
  Ladybird are structured (`components/script`, `components/layout`,
  `components/style`, etc. in Servo's case).
- Pull in [html5ever](https://github.com/servo/html5ever) as a *reference*
  to test your own tokenizer against, not to depend on directly if the goal
  is "recreate," not "wrap."
- Set up **web-platform-tests (WPT)** harness early, even with near-zero
  pass rate. This is the industry-standard conformance suite both Chromium
  and Firefox gate on. Track pass-rate per phase as the actual metric of
  progress instead of vibes.

**Exit criterion:** empty-but-structured repo, CI running, WPT harness
executing (and failing almost everything) end to end.

---

## Phase 2 — Spec-compliant HTML parsing

Replace the toy tokenizer with the actual
[WHATWG HTML parsing algorithm](https://html.spec.whatwg.org/multipage/parsing.html):
a real tokenizer state machine (~80 states) and tree construction algorithm
with insertion modes, the "adoption agency algorithm" for malformed tag
soup, implicit tag closing rules, `<template>` handling, foreign content
(SVG/MathML) parsing rules.

- This is where most of "how does the browser handle broken HTML" lives —
  it's the single most under-estimated subsystem to reimplement correctly.
- Target: pass the `html5lib-tests` tree-construction test suite (the
  standard corpus both engines validate against).

**Exit criterion:** ≥95% pass rate on html5lib-tests tree construction.

---

## Phase 3 — Real CSS engine

- Full selector grammar: combinators (`>`, `+`, `~`, descendant), attribute
  selectors, pseudo-classes (`:hover`, `:nth-child`, `:not()`, ...),
  pseudo-elements (`::before`/`::after`).
- Real cascade: origin/importance ordering (user-agent, user, author,
  `!important`), cascade layers, specificity per spec, not the toy sum used
  in Phase 0.
- Computed-value pipeline: specified → computed → used values, proper
  inheritance rules per property, `initial`/`inherit`/`unset`.
- CSSOM: `CSSStyleSheet`, `CSSRule` object model so later JS integration
  (`document.styleSheets`) has something real to bind to.

**Exit criterion:** pass rate tracked against WPT's `css/cssom` and
`css/selectors` suites.

---

## Phase 4 — Layout (the actual hard part)

- Box generation from the styled tree (anonymous boxes, `display` table).
- Formatting contexts: block, inline (with proper line-breaking/BiDi later),
  float positioning, table layout.
- Modern layout modes: **flexbox**, then **grid** — each is its own
  multi-week sub-project; these are the two things that most differentiate
  "toy layout" from "web can actually render on this."
- Positioning schemes: relative, absolute, fixed, sticky, and stacking
  contexts (needed before paint order/z-index means anything).
- Replace the canvas-box-list output from Phase 0 with a proper **fragment
  tree** / **display list** — the actual intermediate representation real
  engines use between layout and paint (this is what lets you later add
  incremental layout, hit-testing, and a real compositor).

**Exit criterion:** WPT `css/css-flexbox` and `css/css-grid` pass rates
tracked; visual regression tests via reference-rendering comparison
(the same "reftest" approach Gecko/WPT use).

---

## Phase 5 — Text & painting

- Real text shaping (a HarfBuzz-equivalent or binding) — Phase 0's
  `ctx.measureText` word-wrap is not shaping; it doesn't handle ligatures,
  complex scripts, kerning, or BiDi.
- Font loading/matching (`@font-face`, system font fallback chains).
- A real paint backend: software rasterizer or GPU (wgpu, matching Rust
  choice) instead of directly drawing to an HTML canvas — the canvas
  dependency was a Phase-0 shortcut, not an architecture.
- Compositing: layers, transforms, opacity, basic filters.

**Exit criterion:** can render a real, non-trivial static webpage
(e.g. a saved Wikipedia article) pixel-comparably to a reference screenshot
from Firefox/Chrome within a tolerance threshold.

---

## Phase 6 — DOM & the event loop

- Implement the actual DOM spec (nodes, live `NodeList`s, mutation
  semantics) as an addressable API, not an internal-only tree.
- Event loop: microtasks/macrotasks, `requestAnimationFrame`, timers.
- Event dispatch: capture/target/bubble phases, default actions.

**Exit criterion:** WPT `dom/` and `html/webappapis/` suites passing at a
meaningful rate.

---

## Phase 7 — JavaScript

This is a full second engine-scale project on its own (V8 and SpiderMonkey
are each bigger than the rest of their respective browsers combined). Two
honest paths:

- **Pragmatic:** embed an existing engine (rusty_v8 bindings to V8, or
  SpiderMonkey via `mozjs`) and focus your original work on the DOM/layout
  binding layer. This is what almost every "build your own browser" project
  that wants to actually finish does.
- **Purist ("recreate from scratch"):** write your own ECMAScript
  interpreter (parser → AST → bytecode VM), no JIT initially. This alone is
  a multi-month-to-multi-year effort even for ES5-only support, before
  touching modern ES features, async/await, or a JIT tier. Flag explicitly
  if this is really the intent, since it changes the whole roadmap's shape.

**Exit criterion:** Test262 (the ECMAScript conformance suite) pass rate
tracked; DOM bindings enough to run something like a basic React counter
app.

---

## Phase 8 — Networking

- HTTP/1.1 client, then HTTP/2; TLS via an existing crate (rustls) — nobody
  reimplements TLS from scratch, that's a security liability, not a feature.
- Resource loading pipeline (HTML/CSS/JS/image fetch, caching headers,
  redirects), `fetch()`/XHR bindings once Phase 7 exists.
- Cookie jar, CORS enforcement, mixed-content blocking.

**Exit criterion:** can load a real URL end-to-end (network → parse → style
→ layout → paint) for a simple static site.

---

## Phase 9 — Security & process model

- Origin model, same-origin policy enforcement across every subsystem
  touched so far (DOM access, cookies, fetch, storage).
- Sandboxing / process isolation — this is where Chromium's actual security
  reputation comes from (site isolation), and it's an architectural decision
  that's very expensive to retrofit late, so at minimum design for it here
  even if full multi-process comes later.
- CSP enforcement.

**Exit criterion:** a documented threat model plus enforcement tests for
same-origin violations across DOM/net/storage.

---

## Phase 10 — DevTools & extensibility

- An inspector protocol (own or compatible with Chrome DevTools Protocol /
  Firefox's Remote Debugging Protocol) so the DOM/CSSOM/console are
  externally introspectable — genuinely useful dogfood target, and a much
  more scoped, finishable subproject than the rest.
- Console API bindings once JS exists.

**Exit criterion:** can attach an external inspector UI (even a basic one)
and see live DOM tree + computed styles.

---

## Phase 11 — Standards compliance & hardening as an ongoing practice

Not a final phase so much as the mode you're in forever after Phase 4: track
WPT pass rate as the top-line metric (this is literally the metric
Chromium/Firefox/WebKit teams publish and compete on at
wpt.fyi), fix regressions before adding features, fuzz the parser/CSS/layout
code continuously (this is how most real engine security bugs get found).

---

## What to actually do next

Phases 0-3 are realistically the only ones tractable as "a project you and I
iterate on together" without a team — that already gets you a real,
independently-useful HTML+CSS rendering library with spec-level HTML parsing
and a real cascade, which is genuinely more than most "toy browser engine"
projects on GitHub achieve. Phases 4+ are where scope needs a team or a
multi-year personal-project commitment; flagging that now rather than
pretending otherwise.

**Immediate next step, if you want to start today:** Phase 1 — stand up the
Rust workspace and WPT harness. Say the word and I'll scaffold it.
