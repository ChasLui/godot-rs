//! Rust bindings for the Godot 4 game engine, via GDExtension.

pub use godot_core::{builtin, init, method, obj, ptrcall, registry, signal, sys};

/// Generated bindings to Godot's own classes.
pub use godot_bindings::{classes, global, GODOT_PRECISION, GODOT_VERSION};

pub use godot_async as task;
pub use godot_macros::godot_api;

/// Re-exported so `#[godot_api]` can name the runtime without the user depending on it directly.
#[doc(hidden)]
pub use godot_core;

/// Everything needed to write an extension.
pub mod prelude {
    pub use crate::builtin::{FromGodot, GString, StringName, ToGodot, Variant};
    pub use crate::godot_api;
    pub use crate::init::{ExtensionLibrary, InitLevel};
    pub use crate::method::{register_method, MethodDecl};
    pub use crate::obj::{Gd, GodotObject};
    pub use crate::registry::{register_class, unregister_class, GodotClass};
    pub use crate::signal::{register_property, register_signal};
    pub use crate::task::{frames, next_frame, AsyncRuntime};
    pub use godot_core::{godot_entry, godot_error, godot_print, godot_print_err, godot_warn};
}
