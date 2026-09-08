//! Frame-driven async for Godot extensions.
//!
//! Godot 3's GDNative bindings hooked into the engine's `yield` mechanism; Godot 4 has no such
//! hook for extensions, so futures are driven by the game loop instead: [`AsyncRuntime::poll_all`]
//! is called once per frame, and [`next_frame`] is the primitive every other combinator is built
//! from.
//!
//! The runtime is single-threaded on purpose. Godot calls into an extension from the main thread,
//! and an engine call from any other thread is unsound without explicit synchronisation, so
//! futures here are neither `Send` nor `Sync`.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

thread_local! {
    /// Futures waiting to make progress. `RefCell` rather than a lock: single-threaded by design.
    static TASKS: RefCell<Vec<Pin<Box<dyn Future<Output = ()>>>>> = const { RefCell::new(Vec::new()) };
}

/// Drives futures spawned onto the frame loop.
pub struct AsyncRuntime;

impl AsyncRuntime {
    /// Queues a future to be advanced once per frame.
    pub fn spawn(future: impl Future<Output = ()> + 'static) {
        TASKS.with(|tasks| tasks.borrow_mut().push(Box::pin(future)));
    }

    /// Advances every queued future by one step, dropping those that finished.
    ///
    /// Call this once per frame -- typically from a node's `process` hook. Nothing progresses
    /// otherwise; there is no background thread.
    pub fn poll_all() {
        // Futures are taken out of the shared list before polling, so a future that spawns
        // another one does not re-enter the borrow.
        let mut taken = TASKS.with(|tasks| std::mem::take(&mut *tasks.borrow_mut()));

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        taken.retain_mut(|task| task.as_mut().poll(&mut cx).is_pending());

        TASKS.with(|tasks| {
            let mut tasks = tasks.borrow_mut();
            // Anything spawned during polling is already in `tasks`; put the survivors back
            // ahead of it so ordering stays stable.
            taken.append(&mut tasks);
            *tasks = taken;
        });
    }

    /// How many futures are still queued. Useful for tests and for deciding to stop polling.
    pub fn pending_count() -> usize {
        TASKS.with(|tasks| tasks.borrow().len())
    }
}

/// Yields once, resuming on the next frame.
pub fn next_frame() -> NextFrame {
    NextFrame { polled: false }
}

/// Future returned by [`next_frame`].
pub struct NextFrame {
    polled: bool,
}

impl Future for NextFrame {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        if self.polled {
            Poll::Ready(())
        } else {
            // Pending now, ready on the next `poll_all` -- which is the next frame.
            self.polled = true;
            Poll::Pending
        }
    }
}

/// Waits for the given number of frames.
pub async fn frames(count: u32) {
    for _ in 0..count {
        next_frame().await;
    }
}

/// The runtime polls unconditionally every frame, so a waker has nothing to signal.
fn noop_waker() -> Waker {
    const VTABLE: RawWakerVTable = RawWakerVTable::new(
        |_| RawWaker::new(std::ptr::null(), &VTABLE),
        |_| {},
        |_| {},
        |_| {},
    );

    // SAFETY: every function in the vtable is a no-op and ignores the data pointer.
    unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) }
}
