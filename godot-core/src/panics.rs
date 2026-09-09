//! Stopping Rust panics at the FFI boundary.
//!
//! Every callback the engine invokes is an `extern "C"` function, and unwinding out of one is
//! undefined behaviour -- in practice the process aborts. A panic in ordinary user code, an
//! index out of range or an `unwrap` on `None`, would therefore take the whole editor down and
//! lose unsaved work.
//!
//! So each boundary catches instead: the panic is reported through Godot's error output, where
//! it shows up in the Errors panel with the rest, and the callback returns a fallback value.

use std::panic::AssertUnwindSafe;

/// Runs `body`, converting a panic into a Godot error and `fallback`.
///
/// `context` names the callback in the message, since the engine's own backtrace stops at the
/// FFI boundary and would not say which one it was.
///
/// `AssertUnwindSafe` is used deliberately. The alternative is requiring `UnwindSafe` of every
/// user method, which almost nothing satisfies once `&mut self` is involved. What that gives up
/// is the guarantee that state is consistent after a panic -- and since the object survives, a
/// later call can observe half-finished work. That is a worse outcome than an abort only if the
/// inconsistency is silent, which is why the panic is always reported rather than swallowed.
pub fn catch<R>(context: impl FnOnce() -> String, fallback: R, body: impl FnOnce() -> R) -> R {
    match std::panic::catch_unwind(AssertUnwindSafe(body)) {
        Ok(value) => value,
        Err(payload) => {
            // `context` is a closure so the message is built only when it is needed. Formatting
            // it eagerly would cost an allocation on every call, panic or not, and these sit on
            // the hot path between GDScript and Rust.
            crate::logging::godot_error(&format!(
                "Rust panic in {}: {}",
                context(),
                describe(&payload)
            ));
            fallback
        }
    }
}

/// Pulls the message out of a panic payload, which is a `String` or `&str` for the common cases.
fn describe(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic with a non-string payload".to_string()
    }
}
