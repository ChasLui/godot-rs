//! Dispatch for Godot's virtual methods (`_ready`, `_process`, ...).
//!
//! Godot resolves a virtual once per class through `get_virtual_call_data_func`, caching whatever
//! opaque token is returned, and then passes that token to `call_virtual_with_data_func` on every
//! call. Returning null means "not overridden", which is how the engine skips classes that do not
//! implement a given hook -- notably `_process`, which would otherwise run every frame for
//! nothing.
//!
//! The alternative, `get_virtual_func`, would re-resolve by name on each call; this path exists
//! precisely to avoid that.

use crate::builtin::StringName;
use crate::registry::GodotClass;
use godot_sys as sys;

/// Tokens handed to Godot to identify which virtual was resolved.
///
/// Godot treats the value as opaque and only ever passes it back, so small non-null integers
/// serve as discriminants without needing statics to point at.
mod token {
    pub const READY: usize = 1;
    pub const PROCESS: usize = 2;
    pub const PHYSICS_PROCESS: usize = 3;
}

/// Godot's name for each hook, paired with its token.
const VIRTUAL_TABLE: &[(&str, usize)] = &[
    ("_ready", token::READY),
    ("_process", token::PROCESS),
    ("_physics_process", token::PHYSICS_PROCESS),
];

pub(crate) unsafe extern "C" fn get_virtual_call_data<T: GodotClass>(
    _class_userdata: *mut std::ffi::c_void,
    name: sys::GDExtensionConstStringNamePtr,
    _hash: u32,
) -> *mut std::ffi::c_void {
    let name = StringName::from_sys_copy(name).to_rust_string();

    for (godot_name, tok) in VIRTUAL_TABLE {
        if *godot_name == name && T::OVERRIDDEN_VIRTUALS.contains(godot_name) {
            return *tok as *mut std::ffi::c_void;
        }
    }

    // Null tells Godot this class does not override the method, so it stops asking.
    std::ptr::null_mut()
}

pub(crate) unsafe extern "C" fn call_virtual_with_data<T: GodotClass>(
    instance: sys::GDExtensionClassInstancePtr,
    _name: sys::GDExtensionConstStringNamePtr,
    userdata: *mut std::ffi::c_void,
    args: *const sys::GDExtensionConstTypePtr,
    _ret: sys::GDExtensionTypePtr,
) {
    if instance.is_null() {
        return;
    }
    let this = &mut *(instance as *mut T);

    match userdata as usize {
        token::READY => this.ready(),
        token::PROCESS => {
            // Virtual arguments arrive in ptrcall form: a pointer to the native `double`.
            let delta = *(*args as *const f64);
            this.process(delta);
        }
        token::PHYSICS_PROCESS => {
            let delta = *(*args as *const f64);
            this.physics_process(delta);
        }
        _ => {}
    }
}
