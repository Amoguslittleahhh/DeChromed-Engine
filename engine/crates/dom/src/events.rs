//! C2: real `Event`/`EventTarget` dispatch -- the actual capture/target/
//! bubble algorithm (<https://dom.spec.whatwg.org/#dispatching-events>),
//! not a simplified "call every listener once" stand-in.
//!
//! Reference: <https://dom.spec.whatwg.org/#events>.
//!
//! **Where listeners live.** They're *not* stored on [`crate::Document`]
//! itself (which would mean giving the core arena tree a callback-storage
//! field it doesn't otherwise need) -- [`EventListeners`] is a side table
//! that wraps a `&Document`'s tree shape the same way `css::style_engine::
//! StyleEngine` already wraps one for computed styles, an established
//! pattern in this codebase rather than a new one invented for events.
//!
//! **Known gaps:** no `Event.composed`/shadow-tree retargeting (no shadow
//! DOM exists in this crate at all), no passive-listener enforcement, and
//! dispatch order among same-node listeners is registration order only
//! (real DOM ties this down further for listeners added/removed *during*
//! dispatch -- this implementation snapshots the listener list per node
//! before invoking any of them, so a listener that adds another listener
//! to the same node during dispatch won't see it fire in that same pass,
//! a real but narrow simplification).

use crate::{Document, NodeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Real per-spec dispatch phases
/// (<https://dom.spec.whatwg.org/#dom-event-capturing_phase>).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventPhase {
    None,
    Capturing,
    AtTarget,
    Bubbling,
}

/// A real `Event` object: the mutable dispatch-time state
/// (`currentTarget`/`eventPhase`/propagation flags) alongside the
/// immutable type/`target`/`bubbles`/`cancelable` a listener reads.
#[derive(Debug, Clone)]
pub struct Event {
    pub event_type: String,
    pub bubbles: bool,
    pub cancelable: bool,
    pub target: Option<NodeId>,
    pub current_target: Option<NodeId>,
    pub phase: EventPhase,
    default_prevented: bool,
    propagation_stopped: bool,
    immediate_propagation_stopped: bool,
}

impl Event {
    pub fn new(event_type: impl Into<String>, bubbles: bool, cancelable: bool) -> Self {
        Event {
            event_type: event_type.into(),
            bubbles,
            cancelable,
            target: None,
            current_target: None,
            phase: EventPhase::None,
            default_prevented: false,
            propagation_stopped: false,
            immediate_propagation_stopped: false,
        }
    }

    /// `Event.preventDefault()`: a real no-op unless `cancelable` is set,
    /// matching the spec exactly (a listener can't cancel an event whose
    /// type doesn't support cancellation).
    pub fn prevent_default(&mut self) {
        if self.cancelable {
            self.default_prevented = true;
        }
    }

    pub fn default_prevented(&self) -> bool {
        self.default_prevented
    }

    /// `Event.stopPropagation()`: stops the event from reaching any
    /// *further* node in the capture/bubble path, but (per spec) doesn't
    /// stop other listeners already registered on the *current* node from
    /// running -- see [`stop_immediate_propagation`](Self::stop_immediate_propagation)
    /// for that.
    pub fn stop_propagation(&mut self) {
        self.propagation_stopped = true;
    }

    /// `Event.stopImmediatePropagation()`: stops propagation *and* skips
    /// any remaining listeners on the current node in this same pass.
    pub fn stop_immediate_propagation(&mut self) {
        self.propagation_stopped = true;
        self.immediate_propagation_stopped = true;
    }
}

type Listener = Rc<RefCell<dyn FnMut(&mut Event)>>;

/// A real listener registry + dispatch algorithm, wrapping (not stored
/// inside) a `Document` -- see module docs for why.
#[derive(Default)]
pub struct EventListeners {
    // (node, event type) -> capturing listeners, bubbling/at-target
    // listeners, in registration order within each list (per spec,
    // capturing-phase and non-capturing-phase listeners on the same node
    // are tracked and ordered independently).
    capture: HashMap<(NodeId, String), Vec<Listener>>,
    bubble: HashMap<(NodeId, String), Vec<Listener>>,
}

impl EventListeners {
    pub fn new() -> Self {
        EventListeners::default()
    }

    /// `EventTarget.addEventListener(type, listener, { capture })`.
    pub fn add_event_listener(
        &mut self,
        node: NodeId,
        event_type: impl Into<String>,
        capture: bool,
        listener: impl FnMut(&mut Event) + 'static,
    ) {
        let table = if capture {
            &mut self.capture
        } else {
            &mut self.bubble
        };
        table
            .entry((node, event_type.into()))
            .or_default()
            .push(Rc::new(RefCell::new(listener)));
    }

    /// The real dispatch algorithm
    /// (<https://dom.spec.whatwg.org/#concept-event-dispatch>): builds
    /// `target`'s real ancestor path via the live `doc` tree, then runs
    /// capturing listeners root-to-target (exclusive of `target`
    /// itself), at-target listeners (both capturing and bubbling lists
    /// fire here, per spec -- phase distinction doesn't apply *at* the
    /// target), and finally bubbling listeners target-to-root (again
    /// exclusive of `target`) if `event.bubbles`. Returns `true` if the
    /// event's default action should proceed (i.e. `!default_prevented`),
    /// matching real `dispatchEvent`'s own return value.
    pub fn dispatch_event(&self, doc: &Document, target: NodeId, event: &mut Event) -> bool {
        event.target = Some(target);

        let mut ancestors = Vec::new();
        let mut current = doc.parent(target);
        while let Some(node) = current {
            ancestors.push(node);
            current = doc.parent(node);
        }

        // Capturing phase: root-to-target, exclusive of target.
        event.phase = EventPhase::Capturing;
        for &node in ancestors.iter().rev() {
            if !self.invoke(&self.capture, node, event) {
                return !event.default_prevented();
            }
        }

        // At-target: both capturing- and bubbling-registered listeners on
        // the target itself fire here, in the order they were added
        // relative to each other isn't tracked across the two lists (a
        // real, narrow simplification) -- capturing-registered ones run
        // first, then bubbling-registered ones.
        event.phase = EventPhase::AtTarget;
        if !self.invoke(&self.capture, target, event) {
            return !event.default_prevented();
        }
        if !self.invoke(&self.bubble, target, event) {
            return !event.default_prevented();
        }

        // Bubbling phase: target-to-root, exclusive of target.
        if event.bubbles {
            event.phase = EventPhase::Bubbling;
            for &node in ancestors.iter() {
                if !self.invoke(&self.bubble, node, event) {
                    return !event.default_prevented();
                }
            }
        }

        event.phase = EventPhase::None;
        !event.default_prevented()
    }

    /// Invokes every listener registered for `(node, event.event_type)`
    /// in `table`, snapshotting the listener list first (see module docs'
    /// known-gap note). Returns `false` if propagation should stop after
    /// this node (i.e. `stopPropagation`/`stopImmediatePropagation` was
    /// called), `true` to continue to the next node in the path.
    fn invoke(
        &self,
        table: &HashMap<(NodeId, String), Vec<Listener>>,
        node: NodeId,
        event: &mut Event,
    ) -> bool {
        event.current_target = Some(node);
        let Some(listeners) = table.get(&(node, event.event_type.clone())) else {
            return !event.propagation_stopped;
        };
        for listener in listeners.clone() {
            listener.borrow_mut()(event);
            if event.immediate_propagation_stopped {
                return false;
            }
        }
        !event.propagation_stopped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Document, ElementData, NodeData};
    use std::cell::RefCell as StdRefCell;

    fn el(doc: &mut Document, parent: NodeId, tag: &str) -> NodeId {
        doc.append(parent, NodeData::Element(ElementData::html(tag, vec![])))
    }

    #[test]
    fn dispatch_runs_target_listener() {
        let mut doc = Document::new();
        let root = doc.root();
        let p = el(&mut doc, root, "p");
        let mut listeners = EventListeners::new();
        let fired = Rc::new(StdRefCell::new(false));
        let fired_clone = fired.clone();
        listeners.add_event_listener(p, "click", false, move |_e| {
            *fired_clone.borrow_mut() = true;
        });
        let mut event = Event::new("click", true, true);
        listeners.dispatch_event(&doc, p, &mut event);
        assert!(*fired.borrow());
    }

    #[test]
    fn capture_then_target_then_bubble_order() {
        let mut doc = Document::new();
        let root = doc.root();
        let parent = el(&mut doc, root, "div");
        let child = el(&mut doc, parent, "span");
        let mut listeners = EventListeners::new();
        let order = Rc::new(StdRefCell::new(Vec::<&'static str>::new()));

        let o1 = order.clone();
        listeners.add_event_listener(parent, "click", true, move |_| {
            o1.borrow_mut().push("parent-capture")
        });
        let o2 = order.clone();
        listeners.add_event_listener(child, "click", false, move |_| {
            o2.borrow_mut().push("child-target")
        });
        let o3 = order.clone();
        listeners.add_event_listener(parent, "click", false, move |_| {
            o3.borrow_mut().push("parent-bubble")
        });

        let mut event = Event::new("click", true, true);
        listeners.dispatch_event(&doc, child, &mut event);
        assert_eq!(
            *order.borrow(),
            vec!["parent-capture", "child-target", "parent-bubble"]
        );
    }

    #[test]
    fn non_bubbling_event_skips_bubble_phase() {
        let mut doc = Document::new();
        let root = doc.root();
        let parent = el(&mut doc, root, "div");
        let child = el(&mut doc, parent, "span");
        let mut listeners = EventListeners::new();
        let fired = Rc::new(StdRefCell::new(false));
        let f = fired.clone();
        listeners.add_event_listener(parent, "focus", false, move |_| *f.borrow_mut() = true);
        let mut event = Event::new("focus", false, true);
        listeners.dispatch_event(&doc, child, &mut event);
        assert!(!*fired.borrow());
    }

    #[test]
    fn stop_propagation_prevents_further_nodes_but_not_current_node_listeners() {
        let mut doc = Document::new();
        let root = doc.root();
        let parent = el(&mut doc, root, "div");
        let child = el(&mut doc, parent, "span");
        let mut listeners = EventListeners::new();
        let order = Rc::new(StdRefCell::new(Vec::<&'static str>::new()));

        let o1 = order.clone();
        listeners.add_event_listener(child, "click", false, move |e: &mut Event| {
            o1.borrow_mut().push("child-1");
            e.stop_propagation();
        });
        let o2 = order.clone();
        listeners.add_event_listener(child, "click", false, move |_| {
            o2.borrow_mut().push("child-2")
        });
        let o3 = order.clone();
        listeners.add_event_listener(parent, "click", false, move |_| {
            o3.borrow_mut().push("parent")
        });

        let mut event = Event::new("click", true, true);
        listeners.dispatch_event(&doc, child, &mut event);
        // Both listeners on the target node still ran (stopPropagation
        // doesn't cut off the current node), but the ancestor never fired.
        assert_eq!(*order.borrow(), vec!["child-1", "child-2"]);
    }

    #[test]
    fn stop_immediate_propagation_skips_remaining_current_node_listeners_too() {
        let mut doc = Document::new();
        let root = doc.root();
        let target = el(&mut doc, root, "div");
        let mut listeners = EventListeners::new();
        let order = Rc::new(StdRefCell::new(Vec::<&'static str>::new()));

        let o1 = order.clone();
        listeners.add_event_listener(target, "click", false, move |e: &mut Event| {
            o1.borrow_mut().push("first");
            e.stop_immediate_propagation();
        });
        let o2 = order.clone();
        listeners.add_event_listener(target, "click", false, move |_| {
            o2.borrow_mut().push("second")
        });

        let mut event = Event::new("click", true, true);
        listeners.dispatch_event(&doc, target, &mut event);
        assert_eq!(*order.borrow(), vec!["first"]);
    }

    #[test]
    fn prevent_default_only_takes_effect_when_cancelable() {
        let mut doc = Document::new();
        let root = doc.root();
        let target = el(&mut doc, root, "div");
        let mut listeners = EventListeners::new();
        listeners.add_event_listener(target, "click", false, |e: &mut Event| e.prevent_default());

        let mut cancelable_event = Event::new("click", true, true);
        let proceed = listeners.dispatch_event(&doc, target, &mut cancelable_event);
        assert!(!proceed);
        assert!(cancelable_event.default_prevented());

        let mut non_cancelable_event = Event::new("click", true, false);
        let proceed = listeners.dispatch_event(&doc, target, &mut non_cancelable_event);
        assert!(proceed);
        assert!(!non_cancelable_event.default_prevented());
    }

    #[test]
    fn dispatch_with_no_listeners_returns_true_and_does_not_panic() {
        let mut doc = Document::new();
        let root = doc.root();
        let target = el(&mut doc, root, "div");
        let listeners = EventListeners::new();
        let mut event = Event::new("click", true, true);
        assert!(listeners.dispatch_event(&doc, target, &mut event));
    }
}
