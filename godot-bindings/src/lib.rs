//! Godot class bindings, generated at build time from `extension_api.json`.
//!
//! The generated surface is a subset of the engine API -- see `godot-codegen` for which classes
//! are included and why. The build prints how many methods were skipped.
//!
//! Lints are relaxed here because the contents are machine-written: names come from the engine,
//! and argument counts follow Godot's signatures rather than Rust style.

#![allow(clippy::too_many_arguments)]
#![allow(clippy::not_unsafe_ptr_arg_deref)]
#![allow(clippy::wrong_self_convention)]
#![allow(non_snake_case)]
#![allow(non_upper_case_globals)]

include!(concat!(env!("OUT_DIR"), "/classes.rs"));
