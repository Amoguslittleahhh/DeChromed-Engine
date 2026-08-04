//! Track C (Script & Runtime): "The JS engine question" in ROADMAP.md is
//! resolved -- this crate embeds real V8 (via the `v8`/rusty_v8 crate)
//! rather than writing an ES2015+ parser/interpreter from scratch. See
//! [`engine`] for the actual isolate/context/script-run wrapper (C3), and
//! [`dom_binding`] for exposing the real `dom::Document` (C1) to running JS
//! (C4).

pub mod dom_binding;
pub mod engine;

pub use engine::{JsError, Realm};
