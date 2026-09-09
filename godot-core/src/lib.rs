//! Core runtime for Godot 4 GDExtension bindings.
//!
//! Targets the Godot version recorded in `godot-sys/gdextension/VERSION`.

// Generated code refers to this crate as `::godot_core`, the same way dependent crates do.
// This alias makes those paths resolve inside the crate itself, so the code generator does not
// need to know whether its output lands here or in godot-bindings.
extern crate self as godot_core;

pub mod builtin;
pub mod init;
pub mod logging;
pub mod method;
pub mod obj;
pub mod ptrcall;
pub mod registry;
pub mod signal;
pub mod virtuals;

pub use godot_sys as sys;

pub use logging::{godot_error, godot_print, godot_print_err, godot_warn};
