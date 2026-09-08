//! Core runtime for Godot 4 GDExtension bindings.
//!
//! Targets the Godot version recorded in `godot-sys/gdextension/VERSION`.

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
