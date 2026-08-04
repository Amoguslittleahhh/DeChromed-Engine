//! C4: a real `document`/element binding -- JS code running in a
//! [`crate::Realm`] can genuinely read and mutate a `dom::Document`, not a
//! mocked/no-op stand-in.
//!
//! **How the Rust `Document` is reachable from V8 callbacks without
//! `unsafe`.** `Isolate::set_slot`/`get_slot` (both safe `pub fn` on
//! `v8::Isolate`) are the standard rusty_v8 mechanism for stashing
//! arbitrary `'static` host state on an isolate; this binds
//! `Rc<RefCell<dom::Document>>` there, so every `FunctionTemplate`
//! callback can borrow it back out via `scope.get_slot::<DocumentSlot>()`
//! without needing to close over anything itself (`FunctionCallback`'s
//! `extern "C" fn`-shaped signature has no room for capturing closures'
//! environments to survive V8's own calling convention).
//!
//! **What's actually exposed**, as plain functions on the `document`
//! global object (not yet real `Node`/`Element`/`Document` *prototypes*
//! with inheritance -- see gaps below):
//! - `document.getElementById(id)` -> an opaque element handle (a plain
//!   JS number: the node's [`dom::NodeId`] index) or `null`.
//! - `document.createElement(tagName)` -> a new detached element handle.
//! - `document.getAttribute(handle, name)` / `.setAttribute(handle, name,
//!   value)` / `.hasAttribute(handle, name)` / `.removeAttribute(handle,
//!   name)`.
//! - `document.textContent(handle)` / `.setTextContent(handle, value)`.
//! - `document.appendChild(parentHandle, childHandle)`.
//! - `document.tagName(handle)` -- `Node.nodeName` (real upper-casing
//!   behavior for HTML-namespace elements, per `dom::api`).
//!
//! **Known gaps** (real, and worth being honest about rather than
//! implying a fuller binding than exists): node handles are bare integer
//! `NodeId`s round-tripped through JS numbers, not real
//! `Node`/`Element`/`Document` JS *objects* with prototype-chained
//! methods (`el.getAttribute(...)` isn't callable -- only
//! `document.getAttribute(el, ...)` is) -- building real wrapper objects
//! needs `ObjectTemplate`/accessor properties, deferred rather than
//! bolted on partially; no event listener binding yet (`dom::events`
//! exists and is tested on the Rust side, but nothing here calls
//! `addEventListener` from JS); no `NodeList`/live collections returned
//! to JS (`getElementsByTagName` isn't bound); a `NodeId` handle that
//! doesn't exist in the document (e.g. a stale one from a different
//! `Realm`/`Document`) is handled by returning `undefined`/being a no-op
//! rather than throwing, since this crate has no JS exception type to
//! throw yet.

use dom::{Document, NodeId};
use std::cell::RefCell;
use std::rc::Rc;

/// The `Isolate::set_slot` payload: the shared, mutable `Document` every
/// bound function reaches back into.
struct DocumentSlot(Rc<RefCell<Document>>);

/// Real `NodeId` <-> JS-number round-tripping via `NodeId::as_u32`. Uses
/// the raw arena index rather than anything tree-position-derived,
/// because a freshly created but not-yet-attached node (e.g. straight out
/// of `Document::create_element`) has no tree position to identify it by
/// at all -- only the arena index is available for those.
fn node_id_to_f64(id: NodeId) -> f64 {
    id.as_u32() as f64
}

/// Binds `document` -- and, via [`crate::Realm::new_with_document`]'s slot
/// setup, this module's callbacks -- onto `scope`'s current context's
/// global object.
pub fn install(scope: &mut v8::HandleScope<'_>, doc: Rc<RefCell<Document>>) {
    scope.set_slot(DocumentSlot(doc));

    let document_key = v8::String::new(scope, "document").unwrap();
    let document_obj = v8::Object::new(scope);

    bind_method(scope, document_obj, "getElementById", get_element_by_id);
    bind_method(scope, document_obj, "createElement", create_element);
    bind_method(scope, document_obj, "getAttribute", get_attribute);
    bind_method(scope, document_obj, "setAttribute", set_attribute);
    bind_method(scope, document_obj, "hasAttribute", has_attribute);
    bind_method(scope, document_obj, "removeAttribute", remove_attribute);
    bind_method(scope, document_obj, "textContent", text_content);
    bind_method(scope, document_obj, "setTextContent", set_text_content);
    bind_method(scope, document_obj, "appendChild", append_child);
    bind_method(scope, document_obj, "tagName", tag_name);

    let global = scope.get_current_context().global(scope);
    global.set(scope, document_key.into(), document_obj.into());
}

fn bind_method(
    scope: &mut v8::HandleScope<'_>,
    object: v8::Local<v8::Object>,
    name: &str,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
) {
    let key = v8::String::new(scope, name).unwrap();
    let template = v8::FunctionTemplate::new(scope, callback);
    let function = template.get_function(scope).unwrap();
    object.set(scope, key.into(), function.into());
}

fn with_document<R>(scope: &mut v8::HandleScope, f: impl FnOnce(&mut Document) -> R) -> Option<R> {
    let doc_rc = scope.get_slot::<DocumentSlot>()?.0.clone();
    Some(f(&mut doc_rc.borrow_mut()))
}

fn node_id_arg(scope: &mut v8::HandleScope, value: v8::Local<v8::Value>) -> Option<NodeId> {
    let n = value.number_value(scope)? as u32;
    Some(NodeId::from_u32(n))
}

fn get_element_by_id(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let Some(id_str) = args.get(0).to_string(scope) else {
        return;
    };
    let id_str = id_str.to_rust_string_lossy(scope);
    let found = with_document(scope, |doc| doc.get_element_by_id(&id_str)).flatten();
    match found {
        Some(node_id) => retval.set_double(node_id_to_f64(node_id)),
        None => retval.set_null(),
    }
}

fn create_element(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let Some(tag) = args.get(0).to_string(scope) else {
        return;
    };
    let tag = tag.to_rust_string_lossy(scope);
    if let Some(node_id) = with_document(scope, move |doc| doc.create_element(&tag)) {
        retval.set_double(node_id_to_f64(node_id));
    }
}

fn get_attribute(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let Some(node_id) = node_id_arg(scope, args.get(0)) else {
        return retval.set_null();
    };
    let Some(name) = args.get(1).to_string(scope) else {
        return;
    };
    let name = name.to_rust_string_lossy(scope);
    let value = with_document(scope, move |doc| doc.get_attribute(node_id, &name)).flatten();
    match value {
        Some(v) => {
            let s = v8::String::new(scope, &v).unwrap();
            retval.set(s.into());
        }
        None => retval.set_null(),
    }
}

fn set_attribute(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let Some(node_id) = node_id_arg(scope, args.get(0)) else {
        return;
    };
    let (Some(name), Some(value)) = (args.get(1).to_string(scope), args.get(2).to_string(scope))
    else {
        return;
    };
    let name = name.to_rust_string_lossy(scope);
    let value = value.to_rust_string_lossy(scope);
    with_document(scope, move |doc| doc.set_attribute(node_id, &name, &value));
}

fn has_attribute(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let Some(node_id) = node_id_arg(scope, args.get(0)) else {
        return retval.set_bool(false);
    };
    let Some(name) = args.get(1).to_string(scope) else {
        return;
    };
    let name = name.to_rust_string_lossy(scope);
    let has = with_document(scope, move |doc| doc.has_attribute(node_id, &name)).unwrap_or(false);
    retval.set_bool(has);
}

fn remove_attribute(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let Some(node_id) = node_id_arg(scope, args.get(0)) else {
        return;
    };
    let Some(name) = args.get(1).to_string(scope) else {
        return;
    };
    let name = name.to_rust_string_lossy(scope);
    with_document(scope, move |doc| doc.remove_attribute(node_id, &name));
}

fn text_content(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let Some(node_id) = node_id_arg(scope, args.get(0)) else {
        return retval.set_null();
    };
    let value = with_document(scope, move |doc| doc.text_content(node_id)).flatten();
    match value {
        Some(v) => {
            let s = v8::String::new(scope, &v).unwrap();
            retval.set(s.into());
        }
        None => retval.set_null(),
    }
}

fn set_text_content(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let Some(node_id) = node_id_arg(scope, args.get(0)) else {
        return;
    };
    let Some(value) = args.get(1).to_string(scope) else {
        return;
    };
    let value = value.to_rust_string_lossy(scope);
    with_document(scope, move |doc| doc.set_text_content(node_id, &value));
}

fn append_child(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    _retval: v8::ReturnValue,
) {
    let (Some(parent), Some(child)) = (
        node_id_arg(scope, args.get(0)),
        node_id_arg(scope, args.get(1)),
    ) else {
        return;
    };
    with_document(scope, move |doc| doc.append_existing(parent, child));
}

fn tag_name(
    scope: &mut v8::HandleScope,
    args: v8::FunctionCallbackArguments,
    mut retval: v8::ReturnValue,
) {
    let Some(node_id) = node_id_arg(scope, args.get(0)) else {
        return retval.set_null();
    };
    let name = with_document(scope, move |doc| doc.node_name(node_id));
    match name {
        Some(n) => {
            let s = v8::String::new(scope, &n).unwrap();
            retval.set(s.into());
        }
        None => retval.set_null(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Realm;
    use dom::{ElementData, NodeData};

    fn realm_with_document(build: impl FnOnce(&mut Document)) -> (Realm, Rc<RefCell<Document>>) {
        let mut doc = Document::new();
        build(&mut doc);
        let doc = Rc::new(RefCell::new(doc));
        let realm = Realm::new_with_document(doc.clone());
        (realm, doc)
    }

    #[test]
    fn get_element_by_id_finds_a_real_element() {
        let (mut realm, doc) = realm_with_document(|doc| {
            let root = doc.root();
            doc.append(
                root,
                NodeData::Element(ElementData::html(
                    "div",
                    vec![("id".to_string(), "target".to_string())],
                )),
            );
        });
        let result = realm
            .run("document.getElementById('target') !== null")
            .unwrap();
        assert_eq!(result, "true");
        let missing = realm.run("document.getElementById('nope')").unwrap();
        assert_eq!(missing, "null");
        let _ = doc;
    }

    #[test]
    fn get_and_set_attribute_round_trip_through_real_dom_state() {
        let (mut realm, doc) = realm_with_document(|doc| {
            let root = doc.root();
            doc.append(
                root,
                NodeData::Element(ElementData::html(
                    "div",
                    vec![("id".to_string(), "el".to_string())],
                )),
            );
        });
        realm
            .run("var el = document.getElementById('el'); document.setAttribute(el, 'data-x', 'hello');")
            .unwrap();
        let value = realm.run("document.getAttribute(el, 'data-x')").unwrap();
        assert_eq!(value, "hello");

        let element_id = doc
            .borrow()
            .get_element_by_id("el")
            .expect("element should exist");
        assert_eq!(
            doc.borrow().get_attribute(element_id, "data-x").as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn create_element_and_append_child_mutate_the_real_tree() {
        let (mut realm, doc) = realm_with_document(|_| {});
        realm
            .run(
                "var el = document.createElement('span'); \
                 document.setTextContent(el, 'hi'); \
                 document.appendChild(0, el);",
            )
            .unwrap();
        let root = doc.borrow().root();
        let child_count = doc.borrow().children(root).len();
        assert_eq!(child_count, 1);
        let child = doc.borrow().children(root)[0];
        assert_eq!(doc.borrow().node_name(child), "SPAN");
        assert_eq!(doc.borrow().text_content(child).as_deref(), Some("hi"));
    }

    #[test]
    fn has_attribute_and_remove_attribute_reflect_real_state() {
        let (mut realm, _doc) = realm_with_document(|doc| {
            let root = doc.root();
            doc.append(
                root,
                NodeData::Element(ElementData::html(
                    "div",
                    vec![
                        ("id".to_string(), "el".to_string()),
                        ("data-x".to_string(), "1".to_string()),
                    ],
                )),
            );
        });
        let has_before = realm
            .run("document.hasAttribute(document.getElementById('el'), 'data-x')")
            .unwrap();
        assert_eq!(has_before, "true");
        realm
            .run("document.removeAttribute(document.getElementById('el'), 'data-x')")
            .unwrap();
        let has_after = realm
            .run("document.hasAttribute(document.getElementById('el'), 'data-x')")
            .unwrap();
        assert_eq!(has_after, "false");
    }

    #[test]
    fn tag_name_matches_real_node_name_upper_casing() {
        let (mut realm, _doc) = realm_with_document(|doc| {
            let root = doc.root();
            doc.append(
                root,
                NodeData::Element(ElementData::html(
                    "p",
                    vec![("id".to_string(), "el".to_string())],
                )),
            );
        });
        let result = realm
            .run("document.tagName(document.getElementById('el'))")
            .unwrap();
        assert_eq!(result, "P");
    }
}
