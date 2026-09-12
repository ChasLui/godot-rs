//! Rust bindings for the Godot 4 game engine, via GDExtension.
//!
//! # Getting started
//!
//! A class is an ordinary Rust type plus an `impl` block marked [`macro@godot_api`]. Registering
//! it makes it a first-class engine class, instantiable from GDScript and the editor.
//!
//! ```no_run
//! use godot::classes::Node;
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
//!     /// The engine object this class is attached to, which is how it acts on itself.
//!     base: Base<Node>,
//! }
//!
//! #[godot_api(base = Node)]
//! impl Player {
//!     fn init() -> Self {
//!         Self { health: 100, base: Base::unset() }
//!     }
//!
//!     /// The engine hands over the object once, right after construction.
//!     fn on_base_ready(&mut self, base: godot::sys::GDExtensionObjectPtr) {
//!         // SAFETY: the engine passes the object this instance was just attached to.
//!         self.base = unsafe { Base::new(base) };
//!     }
//!
//!     /// Exported to GDScript; arguments and return values convert automatically.
//!     #[func]
//!     fn take_damage(&mut self, amount: i64) -> i64 {
//!         self.health -= amount;
//!         if self.health <= 0 {
//!             // Emitting one of its own signals is a call *on the object*, so it goes through
//!             // `base`. A `Gd` is not needed for this -- the base derefs to the class it names.
//!             let _ = self.base.emit_signal(&StringName::new("died"), &[]);
//!         }
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
//!         // `get_name` comes from Node, which this class inherits: inherited methods are called
//!         // on the base handle, not on `self`.
//!         godot_print(&format!("{} is ready", self.base.get_name().to_rust_string()));
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

pub use godot_core::{builtin, editor, init, method, obj, ptrcall, registry, signal, sys};

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
    pub use crate::obj::{Base, Gd, GodotObject};
    pub use crate::registry::{
        register_class, rust_instance, unregister_class, GodotClass, PropertyDesc,
    };
    pub use crate::signal::{register_property, register_signal};
    pub use crate::task::{frames, next_frame, AsyncRuntime};
    pub use godot_core::{godot_entry, godot_error, godot_print, godot_print_err, godot_warn};
}
