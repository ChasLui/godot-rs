//! Rust bindings for the Godot 4 game engine, via GDExtension.
//!
//! # Getting started
//!
//! A class is an ordinary Rust type plus an `impl` block marked [`macro@godot_api`]. Registering
//! it makes it a first-class engine class, instantiable from GDScript and the editor.
//!
//! ```no_run
//! use godot::prelude::*;
//!
//! struct MyLibrary;
//!
//! impl ExtensionLibrary for MyLibrary {
//!     fn on_level_init(level: InitLevel) {
//!         // Node types can only be registered once the scene classes exist.
//!         if level == InitLevel::Scene {
//!             unsafe { register_class::<Player>(); }
//!         }
//!     }
//!
//!     fn on_level_deinit(level: InitLevel) {
//!         if level == InitLevel::Scene {
//!             unsafe { unregister_class::<Player>(); }
//!         }
//!     }
//! }
//!
//! struct Player {
//!     health: i64,
//! }
//!
//! #[godot_api(base = Node)]
//! impl Player {
//!     fn init() -> Self {
//!         Self { health: 100 }
//!     }
//!
//!     /// Exported to GDScript; arguments and return values convert automatically.
//!     #[func]
//!     fn take_damage(&mut self, amount: i64) -> i64 {
//!         self.health -= amount;
//!         self.health
//!     }
//!
//!     /// Shows up in the Inspector, backed by the accessor pair.
//!     #[prop(set = set_health)]
//!     fn get_health(&mut self) -> i64 {
//!         self.health
//!     }
//!
//!     #[func]
//!     fn set_health(&mut self, value: i64) {
//!         self.health = value;
//!     }
//!
//!     /// Declared, not implemented: only the name and argument names are registered.
//!     #[signal]
//!     fn died() {}
//!
//!     /// An engine hook. The macro also tells Godot the class overrides it.
//!     #[godot_virtual]
//!     fn ready(&mut self) {
//!         godot_print("Player ready");
//!
//!         // Engine methods are called on the handle; `get_name` comes from Node, which this
//!         // class inherits.
//!         if let Some(node) = Gd::<godot::classes::Node>::new() {
//!             node.set_name(&StringName::new("Spawned"));
//!             unsafe { node.free() };
//!         }
//!     }
//! }
//!
//! godot_entry!(my_library_init, MyLibrary);
//! ```
//!
//! The crate must be a `cdylib`, and the Godot project needs a `.gdextension` file whose
//! `entry_symbol` matches the name given to [`godot_core::godot_entry!`]. See the README for both.
//!
//! # Where things live
//!
//! - [`classes`] -- the engine's own classes, generated from its API description. Call methods
//!   on a [`Gd`](obj::Gd) handle: `node.add_child(&child)`.
//! - [`builtin`] -- `Variant` and the types it can hold: strings, vectors, arrays, callables.
//! - [`obj`] -- [`Gd<T>`](obj::Gd), the handle to an engine object.
//! - [`global`] -- engine enums and constants.
//! - [`task`] -- futures driven by the frame loop.
//!
//! # What is not here
//!
//! The README's Scope section lists this in full. In short: every non-editor engine class is
//! generated, every non-virtual method is callable, and any virtual can be overridden. Editor
//! classes are behind the `editor` feature.

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
    pub use crate::registry::{register_class, unregister_class, GodotClass, PropertyDesc};
    pub use crate::signal::{register_property, register_signal};
    pub use crate::task::{frames, next_frame, AsyncRuntime};
    pub use godot_core::{godot_entry, godot_error, godot_print, godot_print_err, godot_warn};
}
