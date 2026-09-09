//! `Callable` and `Signal`: Godot's references to a method and to a signal on an object.
//!
//! These are what the engine's connection API is written in terms of, so without them a Rust
//! extension can define signals but not connect to them from Rust.

use super::variant::{FromGodot, ToGodot, Variant};
use crate::builtin::macros::constructor;
use crate::builtin::StringName;
use crate::obj::{Gd, GodotObject};
use godot_sys as sys;
use std::mem::MaybeUninit;

engine_builtin!(
    /// A reference to a method on an object, the value Godot's `connect` expects.
    Callable,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_CALLABLE,
    SIZE_CALLABLE
);

engine_builtin!(
    /// A reference to a signal on an object.
    Signal,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_SIGNAL,
    SIZE_SIGNAL
);

/// Builds one of these from `(object, name)`, which is constructor 2 for both types.
///
/// # Safety
/// `tag` and `size` must belong to the type being constructed.
unsafe fn from_object_and_name<const N: usize>(
    tag: sys::GDExtensionVariantType,
    object: sys::GDExtensionObjectPtr,
    name: &StringName,
) -> [u8; N] {
    let mut opaque = MaybeUninit::<[u8; N]>::uninit();
    let ctor = constructor(tag, 2).expect("engine has no (Object, StringName) constructor");

    // The constructor takes the object *by pointer to the handle*, not the handle itself.
    let args: [sys::GDExtensionConstTypePtr; 2] = [
        &object as *const sys::GDExtensionObjectPtr as sys::GDExtensionConstTypePtr,
        name.as_ptr() as sys::GDExtensionConstTypePtr,
    ];
    ctor(
        opaque.as_mut_ptr() as sys::GDExtensionUninitializedTypePtr,
        args.as_ptr(),
    );
    opaque.assume_init()
}

impl Callable {
    /// References `method` on `object`.
    ///
    /// The method must be one Godot knows about -- for a Rust class that means a `#[func]`.
    /// A name the object does not have yields a callable that is valid but not callable, which
    /// [`Callable::is_valid`] reports.
    pub fn from_object_method<T: GodotObject>(object: &Gd<T>, method: &StringName) -> Self {
        // SAFETY: the tag and size match this type, and `object` is a live handle.
        unsafe {
            Self {
                opaque: from_object_and_name(Self::VARIANT_TYPE, object.as_obj_ptr(), method),
            }
        }
    }
}

impl Signal {
    /// References the signal named `signal` on `object`.
    pub fn from_object_signal<T: GodotObject>(object: &Gd<T>, signal: &StringName) -> Self {
        // SAFETY: as above.
        unsafe {
            Self {
                opaque: from_object_and_name(Self::VARIANT_TYPE, object.as_obj_ptr(), signal),
            }
        }
    }
}
