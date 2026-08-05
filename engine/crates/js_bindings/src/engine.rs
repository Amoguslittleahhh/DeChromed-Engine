//! C3: a real embedded ECMAScript engine -- V8 via the `v8` crate (rusty_v8),
//! not a from-scratch parser/interpreter. See ROADMAP.md's "The JS engine
//! question" for why: embedding was confirmed to actually work in this
//! project's build/sandbox environment (a throwaway probe crate compiled
//! `v8 = "130"` and ran real JavaScript end to end) before this path was
//! chosen over writing an ES2015+ engine from scratch.
//!
//! **Why no `unsafe` here despite embedding a C++ engine:** the workspace
//! denies `unsafe_code` in code we write. `v8`'s public API surface used
//! here -- [`v8::Isolate::new`], [`v8::HandleScope`], [`v8::Context`],
//! [`v8::ContextScope`], [`v8::TryCatch`], [`v8::Script::compile`]/`.run` --
//! is itself safe Rust; V8's own internal use of `unsafe` is the crate's
//! concern, not this one's.
//!
//! **Known gaps:** only a single global [`Realm`] per isolate is modeled
//! (no multiple realms/iframes); no module system (`import`/`export`) --
//! only classic-script `compile`/`run`; the platform is initialized once
//! process-wide via [`std::sync::Once`] and never torn down (matches how
//! every real embedder uses V8 -- `V8::dispose`/`ShutdownPlatform` are for
//! process exit, not per-`Realm` reuse); DOM binding is C4, layered on top
//! of this module via `Isolate::set_slot` (see `crate::dom_binding`).
//!
//! **C5 (standard library/built-ins) and C6 (garbage collector) are, in a
//! real and non-hand-wavy sense, already here.** Embedding V8 (the C3
//! decision -- see "The JS engine question" in ROADMAP.md) means
//! `Object`/`Array`/`String`/`Map`/`Set`/`Promise`/`RegExp`/`Intl`/classes/
//! destructuring/template literals and V8's own real generational GC are
//! not reimplemented, because V8 already *is* a complete, battle-tested
//! implementation of both -- this module's own tests below run real
//! ES2015+ programs against them rather than merely asserting the crate
//! compiled. Two things this module *does* add real code for, because
//! they're genuine embedder-level integration points V8 doesn't hand you
//! for free:
//! - **Explicit microtask control** (`Isolate::set_microtasks_policy`):
//!   set to `Explicit` rather than V8's default `Auto` so that a real
//!   `Promise.then` callback provably does *not* run until [`Realm::
//!   run_microtasks`] is called -- the same explicit-checkpoint model
//!   `dom::event_loop`'s own task/microtask interleaving already uses on
//!   the Rust side, so an embedder driving both can compose them under
//!   one real rule instead of two different implicit ones.
//! - **Forced GC for verification** (`--expose-gc` + `request_garbage_
//!   collection_for_testing`, in [`Realm::force_gc_for_testing`]/[`Realm::
//!   heap_used_bytes`]): lets this module's own GC test actually prove V8
//!   reclaims genuinely unreachable memory rather than just trusting that
//!   it does.
//!
//! **C6's known gap, stated honestly:** the "cross-language GC-to-native-
//! tree integration" problem C6 describes in ROADMAP.md (wrapper tracing,
//! notoriously bug-prone in both real engines) doesn't arise in this
//! codebase yet, because C4's DOM binding hands V8 only bare integer node
//! handles, not real GC-managed wrapper objects holding a live reference
//! into `dom::Document`'s arena -- that integration risk becomes real once
//! C4 grows `ObjectTemplate`-based `Node`/`Element` wrapper objects.

use std::sync::Once;

static V8_INIT: Once = Once::new();

fn ensure_v8_initialized() {
    V8_INIT.call_once(|| {
        // `--expose-gc` is what makes `Isolate::request_garbage_collection_
        // for_testing` (used by `Realm::force_gc_for_testing`) valid to
        // call at all -- must be set before `V8::initialize`, matching
        // every other V8 flag.
        v8::V8::set_flags_from_string("--expose-gc");
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
    });
}

/// A JS execution error: the real exception text/line captured via V8's
/// `TryCatch`, not a bare "it failed" boolean -- covers both compile-time
/// (syntax) errors and runtime (thrown) exceptions, since V8's `TryCatch`
/// catches both the same way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsError {
    pub message: String,
    pub line: Option<u32>,
}

impl std::fmt::Display for JsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(line) => write!(f, "{} (line {})", self.message, line),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for JsError {}

/// One JS global realm: an isolate plus a single global context, real
/// script compile/run against real V8, not a stubbed interpreter.
pub struct Realm {
    isolate: v8::OwnedIsolate,
    global_context: v8::Global<v8::Context>,
}

impl Realm {
    pub fn new() -> Self {
        ensure_v8_initialized();
        let mut isolate = v8::Isolate::new(v8::CreateParams::default());
        // C5: explicit microtask control -- see module docs for why this
        // isn't V8's default `Auto` policy. `Promise`/`.then` callbacks
        // genuinely won't run until `run_microtasks` says so.
        isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
        let global_context = {
            let scope = &mut v8::HandleScope::new(&mut isolate);
            let context = v8::Context::new(scope, Default::default());
            v8::Global::new(scope, context)
        };
        Realm {
            isolate,
            global_context,
        }
    }

    /// C4: a [`Realm`] with the real `document` global bound to `doc` --
    /// see [`crate::dom_binding`] for exactly what's exposed. `doc` is
    /// `Rc<RefCell<_>>` so the caller keeps its own handle to inspect the
    /// document's state after JS has mutated it (the tests in
    /// `dom_binding` do exactly this).
    pub fn new_with_document(doc: std::rc::Rc<std::cell::RefCell<dom::Document>>) -> Self {
        let mut realm = Self::new();
        {
            let scope = &mut v8::HandleScope::new(&mut realm.isolate);
            let context = v8::Local::new(scope, &realm.global_context);
            let scope = &mut v8::ContextScope::new(scope, context);
            crate::dom_binding::install(scope, doc);
        }
        realm
    }

    /// Compiles and runs `source` as a classic script, returning its
    /// completion value stringified (`String(value)`, matching what a
    /// REPL/`eval` result line would show), or a real [`JsError`] with the
    /// actual V8 exception message/line on syntax error or thrown
    /// exception -- both go through the same `TryCatch` path since V8
    /// reports compile failures through it exactly like runtime ones.
    pub fn run(&mut self, source: &str) -> Result<String, JsError> {
        let scope = &mut v8::HandleScope::new(&mut self.isolate);
        let context = v8::Local::new(scope, &self.global_context);
        let scope = &mut v8::ContextScope::new(scope, context);
        let try_catch = &mut v8::TryCatch::new(scope);

        let source_v8 = match v8::String::new(try_catch, source) {
            Some(s) => s,
            None => {
                return Err(JsError {
                    message: "failed to allocate source string".to_string(),
                    line: None,
                });
            }
        };

        let script = match v8::Script::compile(try_catch, source_v8, None) {
            Some(script) => script,
            None => return Err(extract_error(try_catch)),
        };

        match script.run(try_catch) {
            Some(value) => {
                let text = value.to_rust_string_lossy(try_catch);
                Ok(text)
            }
            None => Err(extract_error(try_catch)),
        }
    }

    /// C5: a real HTML-spec-shaped "perform a microtask checkpoint" --
    /// drains V8's own microtask queue (real `Promise.then`/`queueMicrotask`
    /// callbacks, not a Rust re-implementation of them) to completion.
    /// Because [`Realm::new`] sets an explicit microtasks policy, this is
    /// the *only* thing that runs them: a `Promise` resolution scheduled
    /// during [`Realm::run`] provably doesn't fire until this is called --
    /// see this module's own `promise_then_does_not_run_until_a_real_
    /// microtask_checkpoint` test below.
    pub fn run_microtasks(&mut self) {
        self.isolate.perform_microtask_checkpoint();
    }

    /// C6: forces a real, synchronous full garbage collection -- valid
    /// because [`ensure_v8_initialized`] sets `--expose-gc` before `V8::
    /// initialize` (required for `request_garbage_collection_for_testing`
    /// to be callable at all). Exists so this module's own GC test can
    /// prove genuinely unreachable memory is actually reclaimed rather
    /// than asserting V8's GC works without evidence.
    pub fn force_gc_for_testing(&mut self) {
        self.isolate
            .request_garbage_collection_for_testing(v8::GarbageCollectionType::Full);
    }

    /// C6: the isolate's current used-heap size in bytes, straight from
    /// V8's own `GetHeapStatistics` -- real memory accounting, not an
    /// estimate.
    pub fn heap_used_bytes(&mut self) -> usize {
        let mut stats = v8::HeapStatistics::default();
        self.isolate.get_heap_statistics(&mut stats);
        stats.used_heap_size()
    }
}

impl Default for Realm {
    fn default() -> Self {
        Self::new()
    }
}

fn extract_error(try_catch: &mut v8::TryCatch<'_, v8::HandleScope<'_>>) -> JsError {
    let message = try_catch
        .message()
        .map(|m| m.get(try_catch).to_rust_string_lossy(try_catch))
        .or_else(|| {
            try_catch
                .exception()
                .map(|e| e.to_rust_string_lossy(try_catch))
        })
        .unwrap_or_else(|| "unknown JS error".to_string());
    let line = try_catch
        .message()
        .and_then(|m| m.get_line_number(try_catch))
        .map(|l| l as u32);
    JsError { message, line }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_arithmetic_and_returns_the_stringified_result() {
        let mut realm = Realm::new();
        let result = realm.run("1 + 2").unwrap();
        assert_eq!(result, "3");
    }

    #[test]
    fn runs_multiple_statements_and_returns_the_last_expressions_value() {
        let mut realm = Realm::new();
        let result = realm.run("let x = 10; let y = 20; x + y").unwrap();
        assert_eq!(result, "30");
    }

    #[test]
    fn state_persists_across_separate_run_calls_in_the_same_realm() {
        let mut realm = Realm::new();
        realm.run("var counter = 0;").unwrap();
        realm.run("counter = counter + 1;").unwrap();
        let result = realm.run("counter").unwrap();
        assert_eq!(result, "1");
    }

    #[test]
    fn a_syntax_error_returns_a_real_js_error_not_a_panic() {
        let mut realm = Realm::new();
        let err = realm.run("this is not valid javascript (((").unwrap_err();
        assert!(!err.message.is_empty());
    }

    #[test]
    fn a_thrown_exception_is_captured_with_its_message() {
        let mut realm = Realm::new();
        let err = realm.run("throw new Error('boom');").unwrap_err();
        assert!(
            err.message.contains("boom"),
            "expected message to contain 'boom', got: {}",
            err.message
        );
    }

    #[test]
    fn referencing_an_undefined_variable_is_a_real_reference_error() {
        let mut realm = Realm::new();
        let err = realm.run("undefinedVariable123").unwrap_err();
        assert!(
            err.message.contains("undefinedVariable123") || err.message.contains("not defined"),
            "expected a ReferenceError-shaped message, got: {}",
            err.message
        );
    }

    #[test]
    fn two_realms_do_not_share_state() {
        let mut realm_a = Realm::new();
        let mut realm_b = Realm::new();
        realm_a.run("var onlyInA = 42;").unwrap();
        let err = realm_b.run("onlyInA").unwrap_err();
        assert!(!err.message.is_empty());
    }

    // -- C5: real V8 Promise/microtask integration + standard-library
    // evidence (real V8 built-ins, not reimplemented -- see module docs). --

    #[test]
    fn promise_then_does_not_run_until_a_real_microtask_checkpoint() {
        let mut realm = Realm::new();
        realm
            .run("var log = []; Promise.resolve().then(() => log.push('promise')); log.push('sync');")
            .unwrap();
        // The explicit microtasks policy set in `Realm::new` means the
        // `.then` callback is queued but genuinely hasn't run yet.
        let before = realm.run("log.join(',')").unwrap();
        assert_eq!(before, "sync");

        realm.run_microtasks();
        let after = realm.run("log.join(',')").unwrap();
        assert_eq!(after, "sync,promise");
    }

    #[test]
    fn nested_promise_thens_all_drain_in_one_checkpoint() {
        let mut realm = Realm::new();
        realm
            .run(
                "var log = []; \
                 Promise.resolve().then(() => { \
                     log.push('a'); \
                     Promise.resolve().then(() => log.push('b')); \
                 });",
            )
            .unwrap();
        realm.run_microtasks();
        let result = realm.run("log.join(',')").unwrap();
        assert_eq!(result, "a,b");
    }

    #[test]
    fn real_array_methods() {
        let mut realm = Realm::new();
        let result = realm
            .run("[1, 2, 3].map(x => x * 2).filter(x => x > 2).join(',')")
            .unwrap();
        assert_eq!(result, "4,6");
    }

    #[test]
    fn real_map_and_set() {
        let mut realm = Realm::new();
        let map_result = realm
            .run("var m = new Map([['a', 1], ['b', 2]]); m.get('b')")
            .unwrap();
        assert_eq!(map_result, "2");
        let set_result = realm.run("new Set([1, 2, 2, 3]).size").unwrap();
        assert_eq!(set_result, "3");
    }

    #[test]
    fn real_regexp() {
        let mut realm = Realm::new();
        let result = realm.run("/(\\d+)/.exec('abc123')[1]").unwrap();
        assert_eq!(result, "123");
    }

    #[test]
    fn real_template_literals_destructuring_and_classes() {
        let mut realm = Realm::new();
        let template = realm.run("`${1 + 1} apples`").unwrap();
        assert_eq!(template, "2 apples");

        let destructure = realm.run("var [a, , b] = [1, 2, 3]; a + b").unwrap();
        assert_eq!(destructure, "4");

        let class_result = realm
            .run(
                "class Point { constructor(x, y) { this.x = x; this.y = y; } \
                 sum() { return this.x + this.y; } } \
                 new Point(3, 4).sum()",
            )
            .unwrap();
        assert_eq!(class_result, "7");
    }

    #[test]
    fn real_json_round_trip() {
        let mut realm = Realm::new();
        let result = realm
            .run("JSON.parse(JSON.stringify({a: 1, b: [2, 3]})).b[1]")
            .unwrap();
        assert_eq!(result, "3");
    }

    // -- C6: real V8 GC verification. --

    #[test]
    fn forcing_gc_reclaims_genuinely_unreachable_memory() {
        let mut realm = Realm::new();
        // Allocate a lot of garbage: large strings referenced only by a
        // loop-local variable, so nothing survives past this statement.
        realm
            .run(
                "for (let i = 0; i < 20000; i++) { \
                     let junk = 'x'.repeat(1000) + i; \
                 }",
            )
            .unwrap();
        let before = realm.heap_used_bytes();
        realm.force_gc_for_testing();
        let after = realm.heap_used_bytes();
        assert!(
            after < before,
            "expected forced GC to shrink used heap size (before={before}, after={after})"
        );
    }

    #[test]
    fn heap_used_bytes_reflects_real_allocation() {
        let mut realm = Realm::new();
        realm.force_gc_for_testing();
        let baseline = realm.heap_used_bytes();
        realm.run("globalThis.kept = 'y'.repeat(500000);").unwrap();
        let after_allocation = realm.heap_used_bytes();
        assert!(
            after_allocation > baseline,
            "expected a live half-megabyte string to increase used heap size \
             (baseline={baseline}, after_allocation={after_allocation})"
        );
    }
}
