//! Registering an editor plugin from Rust.
//!
//! Godot's own addons are GDScript files listed in a `plugin.cfg`, but an extension does not
//! need one: `editor_add_plugin` takes a class name that is already in ClassDB and the editor
//! instantiates it itself. So a Rust `EditorPlugin` subclass is added the same way any other
//! class is registered -- there is no `.cfg`, no script file, and nothing under `addons/`.

use crate::builtin::StringName;
use crate::registry::GodotClass;
use godot_sys as sys;

/// Adds a registered class to the editor as a plugin.
///
/// Call this from [`InitLevel::Editor`](crate::init::InitLevel), *after* registering the class,
/// and only when `Engine::is_editor_hint()` is true -- the Editor level is a startup phase that
/// a running game goes through as well, not a mode.
///
/// # Safety
/// `T` must already be registered with ClassDB and must descend from `EditorPlugin`. Neither can
/// be checked here: `Inherits` is implemented for engine classes only, and `#[godot_api]` does
/// not generate it for user classes, so the base is just a name until the engine resolves it.
pub unsafe fn add_editor_plugin<T: GodotClass>() {
    let class_name = StringName::new(T::CLASS_NAME);
    sys::interface_fn!(editor_add_plugin)(class_name.as_ptr());
}

/// Removes a plugin added by [`add_editor_plugin`].
///
/// Should run *before* the class is unregistered: the editor holds a live instance of the
/// plugin, and unregistering the class first leaves it holding an instance of a class that no
/// longer exists.
///
/// Measured, rather than assumed: reversing the two in the test suite did *not* crash a headless
/// editor session, so the engine evidently tolerates it there. The order is still the one to
/// write -- it is the one the ABI implies -- but this is a convention, not something the tests
/// can catch going wrong.
///
/// # Safety
/// `T` must be a class previously passed to [`add_editor_plugin`].
pub unsafe fn remove_editor_plugin<T: GodotClass>() {
    let class_name = StringName::new(T::CLASS_NAME);
    sys::interface_fn!(editor_remove_plugin)(class_name.as_ptr());
}
