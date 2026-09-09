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

/// A Rust closure Godot can call, boxed behind the callable's userdata.
///
/// `FnMut` rather than `Fn`: a signal handler that accumulates state is the common case, and
/// Godot calls it from one thread at a time.
type BoxedClosure = Box<dyn FnMut(&[Variant]) -> Variant>;

/// Invoked by the engine for every call on the custom callable.
unsafe extern "C" fn closure_call(
    userdata: *mut std::ffi::c_void,
    args: *const sys::GDExtensionConstVariantPtr,
    arg_count: sys::GDExtensionInt,
    ret: sys::GDExtensionVariantPtr,
    error: *mut sys::GDExtensionCallError,
) {
    if userdata.is_null() {
        if !error.is_null() {
            (*error).error = sys::GDExtensionCallErrorType_GDEXTENSION_CALL_ERROR_INSTANCE_IS_NULL;
        }
        return;
    }

    let closure = &mut *(userdata as *mut BoxedClosure);

    // The engine owns the argument Variants for the duration of the call; copy them so the
    // closure works with ordinary owned values.
    let mut owned = Vec::with_capacity(arg_count as usize);
    for i in 0..arg_count as usize {
        owned.push(Variant::from_sys_copy(*args.add(i)));
    }

    let result = closure(&owned);
    result.move_into(ret);

    if !error.is_null() {
        (*error).error = sys::GDExtensionCallErrorType_GDEXTENSION_CALL_OK;
    }
}

/// Releases the boxed closure when Godot drops the last reference to the callable.
unsafe extern "C" fn closure_free(userdata: *mut std::ffi::c_void) {
    if !userdata.is_null() {
        drop(Box::from_raw(userdata as *mut BoxedClosure));
    }
}

unsafe extern "C" fn closure_is_valid(_userdata: *mut std::ffi::c_void) -> sys::GDExtensionBool {
    // A boxed closure stays callable for as long as the callable itself lives.
    true as sys::GDExtensionBool
}

impl Callable {
    /// Wraps a Rust closure as a callable Godot can invoke.
    ///
    /// The closure is owned by the callable and dropped with it, so it may capture. Unlike
    /// [`Callable::from_object_method`] there is no object behind it, which means Godot cannot
    /// disconnect it automatically when some object is freed -- the connection lives exactly as
    /// long as the callable does.
    pub fn from_closure<F>(closure: F) -> Self
    where
        F: FnMut(&[Variant]) -> Variant + 'static,
    {
        let boxed: BoxedClosure = Box::new(closure);
        // Double boxed on purpose: the outer box gives a thin pointer to hand the engine, since
        // `dyn FnMut` is unsized and its fat pointer does not fit in a `void*`.
        let userdata = Box::into_raw(Box::new(boxed));

        // SAFETY: every field the engine reads is filled in below, and `free_func` releases the
        // userdata exactly once.
        unsafe {
            let mut info: sys::GDExtensionCallableCustomInfo2 = std::mem::zeroed();
            info.callable_userdata = userdata as *mut std::ffi::c_void;
            // Identifies this extension; the engine uses it to drop callables on unload.
            info.token = sys::library();
            info.object_id = 0;
            info.call_func = Some(closure_call);
            info.is_valid_func = Some(closure_is_valid);
            info.free_func = Some(closure_free);

            let mut opaque = MaybeUninit::<[u8; sys::builtin_sizes::SIZE_CALLABLE]>::uninit();
            sys::interface_fn!(callable_custom_create2)(
                opaque.as_mut_ptr() as sys::GDExtensionUninitializedTypePtr,
                &mut info,
            );
            Self {
                opaque: opaque.assume_init(),
            }
        }
    }
}
