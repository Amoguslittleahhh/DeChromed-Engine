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
//! process exit, not per-`Realm` reuse); no DOM binding yet (that's C4,
//! layered on top of this module via `Isolate::set_slot`).

use std::sync::Once;

static V8_INIT: Once = Once::new();

fn ensure_v8_initialized() {
    V8_INIT.call_once(|| {
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
}
