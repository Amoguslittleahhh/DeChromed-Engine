//! C2: the real HTML event loop's task/microtask interleaving rule
//! (<https://html.spec.whatwg.org/multipage/webappapis.html#event-loop-processing-model>)
//! and `requestAnimationFrame`.
//!
//! Reference: <https://html.spec.whatwg.org/multipage/webappapis.html#queue-a-microtask>,
//! <https://html.spec.whatwg.org/multipage/imagebitmap-and-animations.html#animation-frame-callback>.
//!
//! **The actual spec-observable, easy-to-get-wrong rule this implements:**
//! after *every* task runs, the microtask queue is drained to genuine
//! completion -- including microtasks queued by microtasks that ran
//! during that same drain -- before the next task is allowed to start.
//! This is why, in a real browser, `queueMicrotask` callbacks always run
//! before the next `setTimeout(fn, 0)` callback even if the timeout was
//! scheduled first: a plain FIFO queue with no task/microtask distinction
//! would get this wrong.
//!
//! **Known gaps:** no real timers (`setTimeout`/`setInterval` -- there's
//! no wall-clock/reactor driving this event loop yet, so "queue a task"
//! here is immediate-queueing only, not delay-scheduled); `request
//! AnimationFrame` callbacks run when [`EventLoop::run_animation_frame_
//! callbacks`] is called explicitly, not on an actual compositor
//! frame/vsync timer (there's no real render loop in Track F yet); no
//! separate per-task-source queues (the spec allows a user agent to
//! interleave *which* task queue's task runs next by task source --
//! everything here is one FIFO task queue).

use std::collections::VecDeque;

type Task = Box<dyn FnOnce(&mut EventLoop)>;
type AnimationFrameCallback = Box<dyn FnOnce(&mut EventLoop, f64)>;

/// A real (if simplified -- see module docs) task/microtask event loop,
/// plus `requestAnimationFrame` callback bookkeeping.
#[derive(Default)]
pub struct EventLoop {
    tasks: VecDeque<Task>,
    microtasks: VecDeque<Task>,
    animation_frame_callbacks: Vec<(u32, AnimationFrameCallback)>,
    next_animation_frame_id: u32,
}

impl EventLoop {
    pub fn new() -> Self {
        EventLoop::default()
    }

    /// Queues a task (`setTimeout(fn, 0)`-equivalent -- see module docs'
    /// "no real timers" gap for why there's no delay parameter). `f`
    /// receives `&mut EventLoop` so a task can itself queue further
    /// tasks/microtasks/rAF callbacks, exactly like real JS running in an
    /// event loop can.
    pub fn queue_task(&mut self, f: impl FnOnce(&mut EventLoop) + 'static) {
        self.tasks.push_back(Box::new(f));
    }

    /// `queueMicrotask(fn)`.
    pub fn queue_microtask(&mut self, f: impl FnOnce(&mut EventLoop) + 'static) {
        self.microtasks.push_back(Box::new(f));
    }

    /// A real "perform a microtask checkpoint": drains the microtask
    /// queue to genuine completion, including microtasks newly queued by
    /// microtasks that ran during this same call -- not a single
    /// drain-once pass, which would miss exactly the nested-microtask
    /// case real code relies on.
    pub fn run_microtasks(&mut self) {
        while let Some(microtask) = self.microtasks.pop_front() {
            microtask(self);
        }
    }

    /// Runs every currently-queued task, performing a full microtask
    /// checkpoint after each one before the next task is allowed to
    /// start -- the real interleaving rule. A task queued *during* this
    /// call (by an earlier task or one of its microtasks) still runs
    /// before this method returns, since the loop keeps polling `tasks`
    /// until it's genuinely empty.
    pub fn run_task_queue_until_empty(&mut self) {
        while let Some(task) = self.tasks.pop_front() {
            task(self);
            self.run_microtasks();
        }
    }

    /// `requestAnimationFrame(fn)`, returning a real cancellable id.
    pub fn request_animation_frame(
        &mut self,
        f: impl FnOnce(&mut EventLoop, f64) + 'static,
    ) -> u32 {
        let id = self.next_animation_frame_id;
        self.next_animation_frame_id += 1;
        self.animation_frame_callbacks.push((id, Box::new(f)));
        id
    }

    /// `cancelAnimationFrame(id)`.
    pub fn cancel_animation_frame(&mut self, id: u32) {
        self.animation_frame_callbacks
            .retain(|(cb_id, _)| *cb_id != id);
    }

    /// Runs every currently-registered `requestAnimationFrame` callback
    /// exactly once with `timestamp`, then clears the list -- the real
    /// one-shot-per-frame semantics (a callback that itself calls
    /// `requestAnimationFrame` registers for the *next* frame, not this
    /// one, since the list is captured before any callback runs).
    pub fn run_animation_frame_callbacks(&mut self, timestamp: f64) {
        let callbacks = std::mem::take(&mut self.animation_frame_callbacks);
        for (_, callback) in callbacks {
            callback(self, timestamp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn tasks_run_in_fifo_order() {
        let mut loop_ = EventLoop::new();
        let order = Rc::new(RefCell::new(Vec::new()));
        let o1 = order.clone();
        loop_.queue_task(move |_| o1.borrow_mut().push(1));
        let o2 = order.clone();
        loop_.queue_task(move |_| o2.borrow_mut().push(2));
        loop_.run_task_queue_until_empty();
        assert_eq!(*order.borrow(), vec![1, 2]);
    }

    #[test]
    fn microtasks_queued_by_a_task_run_before_the_next_task() {
        let mut loop_ = EventLoop::new();
        let order = Rc::new(RefCell::new(Vec::new()));

        let o1 = order.clone();
        loop_.queue_task(move |el| {
            o1.borrow_mut().push("task1");
            let o1b = o1.clone();
            el.queue_microtask(move |_| o1b.borrow_mut().push("microtask-from-task1"));
        });
        let o2 = order.clone();
        loop_.queue_task(move |_| o2.borrow_mut().push("task2"));

        loop_.run_task_queue_until_empty();
        assert_eq!(
            *order.borrow(),
            vec!["task1", "microtask-from-task1", "task2"]
        );
    }

    #[test]
    fn a_microtask_queued_by_a_microtask_still_drains_before_the_next_task() {
        // The case a naive "drain once" implementation gets wrong.
        let mut loop_ = EventLoop::new();
        let order = Rc::new(RefCell::new(Vec::new()));

        let o1 = order.clone();
        loop_.queue_task(move |el| {
            o1.borrow_mut().push("task1");
            let o1b = o1.clone();
            el.queue_microtask(move |el2| {
                o1b.borrow_mut().push("microtask-A");
                let o1c = o1b.clone();
                el2.queue_microtask(move |_| o1c.borrow_mut().push("microtask-B"));
            });
        });
        let o2 = order.clone();
        loop_.queue_task(move |_| o2.borrow_mut().push("task2"));

        loop_.run_task_queue_until_empty();
        assert_eq!(
            *order.borrow(),
            vec!["task1", "microtask-A", "microtask-B", "task2"]
        );
    }

    #[test]
    fn animation_frame_callbacks_run_once_and_then_clear() {
        let mut loop_ = EventLoop::new();
        let calls = Rc::new(RefCell::new(Vec::new()));
        let c1 = calls.clone();
        loop_.request_animation_frame(move |_, ts| c1.borrow_mut().push(ts));
        loop_.run_animation_frame_callbacks(16.0);
        assert_eq!(*calls.borrow(), vec![16.0]);
        // Second call with nothing re-registered: no callbacks fire.
        loop_.run_animation_frame_callbacks(32.0);
        assert_eq!(*calls.borrow(), vec![16.0]);
    }

    #[test]
    fn a_callback_that_requests_another_frame_is_not_run_in_the_same_pass() {
        let mut loop_ = EventLoop::new();
        let calls = Rc::new(RefCell::new(0u32));
        let c1 = calls.clone();
        loop_.request_animation_frame(move |el, _| {
            *c1.borrow_mut() += 1;
            let c1b = c1.clone();
            el.request_animation_frame(move |_, _| *c1b.borrow_mut() += 1);
        });
        loop_.run_animation_frame_callbacks(0.0);
        assert_eq!(*calls.borrow(), 1);
        loop_.run_animation_frame_callbacks(16.0);
        assert_eq!(*calls.borrow(), 2);
    }

    #[test]
    fn cancel_animation_frame_prevents_the_callback_from_running() {
        let mut loop_ = EventLoop::new();
        let fired = Rc::new(RefCell::new(false));
        let f = fired.clone();
        let id = loop_.request_animation_frame(move |_, _| *f.borrow_mut() = true);
        loop_.cancel_animation_frame(id);
        loop_.run_animation_frame_callbacks(0.0);
        assert!(!*fired.borrow());
    }
}
