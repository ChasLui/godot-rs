//! Marshalling for Godot's `ptrcall` convention.
//!
//! In Godot 4 a bound method is looked up by `(class, method, hash)` and invoked with an array
//! of pointers to the arguments' *native* representations -- not Variants. Every builtin wrapper
//! in this crate is `repr(C)` around exactly that native representation, and `Gd<T>` is
//! `repr(transparent)` around the object pointer, so taking an argument pointer is uniformly
//! "address of self".

use crate::builtin::{GString, StringName, Variant};
use crate::obj::{Gd, GodotObject};
use godot_sys as sys;
use std::mem::MaybeUninit;

/// A value that can be passed as a ptrcall argument.
///
/// # Safety
/// The implementing type must be `repr(C)`/`repr(transparent)` around the exact memory
/// representation Godot expects for the corresponding engine type.
pub unsafe trait PtrcallArg {
    fn arg_ptr(&self) -> sys::GDExtensionConstTypePtr {
        self as *const Self as sys::GDExtensionConstTypePtr
    }
}

/// A value that can be produced from a ptrcall return slot.
///
/// # Safety
/// `from_ptrcall` must hand the engine storage of exactly the right size and layout.
pub unsafe trait PtrcallRet: Sized {
    /// Hands `call` storage for the return value and takes what the engine wrote.
    ///
    /// # Safety
    /// `call` must be a ptrcall whose return type is exactly `Self`'s engine counterpart.
    unsafe fn from_ptrcall<F>(call: F) -> Self
    where
        F: FnOnce(sys::GDExtensionTypePtr);
}

/// For types whose Rust value *is* the engine's representation: hand the engine storage and
/// take what it writes.
///
/// The slot is **zeroed**, not merely uninitialized. Godot writes a ptrcall return value through
/// `PtrToArg<T>::encode`, which for the reference-counted builtins (`String`, `StringName`,
/// `Variant`, ...) *assigns* into the destination -- releasing whatever it believes is already
/// there. Handing it uninitialized stack memory makes it unref a garbage pointer. All-zero is
/// the empty/nil representation for these types, so releasing it is a no-op.
macro_rules! impl_ptrcall_direct {
    ($($t:ty),* $(,)?) => {
        $(
            unsafe impl PtrcallArg for $t {}

            unsafe impl PtrcallRet for $t {
                unsafe fn from_ptrcall<F>(call: F) -> Self
                where
                    F: FnOnce(sys::GDExtensionTypePtr),
                {
                    let mut slot = MaybeUninit::<Self>::zeroed();
                    call(slot.as_mut_ptr() as sys::GDExtensionTypePtr);
                    slot.assume_init()
                }
            }
        )*
    };
}

impl_ptrcall_direct!(bool, i8, i16, i32, i64, u8, u16, u32, u64, f32, f64);
impl_ptrcall_direct!(GString, StringName, Variant);

/// Methods that return nothing still go through the same path; the engine is handed a null
/// return slot.
unsafe impl PtrcallRet for () {
    unsafe fn from_ptrcall<F>(call: F) -> Self
    where
        F: FnOnce(sys::GDExtensionTypePtr),
    {
        call(std::ptr::null_mut());
    }
}

/// A raw pointer argument: ptrcall passes the address *of* the pointer, the same as it does for
/// an object handle, so the default `arg_ptr` is already right.
unsafe impl PtrcallArg for *const std::ffi::c_void {}

unsafe impl PtrcallRet for *const std::ffi::c_void {
    unsafe fn from_ptrcall<F>(call: F) -> Self
    where
        F: FnOnce(sys::GDExtensionTypePtr),
    {
        let mut slot = MaybeUninit::<Self>::zeroed();
        call(slot.as_mut_ptr() as sys::GDExtensionTypePtr);
        slot.assume_init()
    }
}

// `Gd<T>` is transparent over the object pointer, which is exactly what ptrcall passes and
// returns. A null pointer means "no object", hence the `Option`.
unsafe impl<T: GodotObject> PtrcallArg for Gd<T> {}

unsafe impl<T: GodotObject> PtrcallRet for Option<Gd<T>> {
    unsafe fn from_ptrcall<F>(call: F) -> Self
    where
        F: FnOnce(sys::GDExtensionTypePtr),
    {
        // Zeroed for the same reason as above, and so an engine that writes nothing leaves a
        // null pointer -- read as "no object" -- rather than a garbage one.
        let mut slot = MaybeUninit::<sys::GDExtensionObjectPtr>::zeroed();
        call(slot.as_mut_ptr() as sys::GDExtensionTypePtr);
        Gd::from_obj_ptr(slot.assume_init())
    }
}

/// A method bound in ClassDB, resolved once and cached.
///
/// Lookup is by `(class, method, hash)`; the hash pins the exact signature, which is how Godot 4
/// detects that an extension was built against an incompatible API.
pub struct MethodBind {
    ptr: sys::GDExtensionMethodBindPtr,
}

// The pointer is engine-owned, immutable, and valid for the process lifetime once resolved.
unsafe impl Send for MethodBind {}
unsafe impl Sync for MethodBind {}

impl MethodBind {
    /// # Safety
    /// Only valid after the extension is initialized and the class exists in ClassDB.
    pub unsafe fn resolve(class_name: &str, method_name: &str, hash: i64) -> Self {
        let class = StringName::new(class_name);
        let method = StringName::new(method_name);

        let ptr = sys::interface_fn!(classdb_get_method_bind)(
            class.as_ptr(),
            method.as_ptr(),
            hash as sys::GDExtensionInt,
        );

        assert!(
            !ptr.is_null(),
            "method {class_name}::{method_name} (hash {hash}) not found -- \
             the engine's API does not match the one these bindings were generated from"
        );

        Self { ptr }
    }

    /// # Safety
    /// `args` must match the method's signature, and `instance` must be of the right class.
    pub unsafe fn ptrcall<R: PtrcallRet>(
        &self,
        instance: sys::GDExtensionObjectPtr,
        args: &[sys::GDExtensionConstTypePtr],
    ) -> R {
        // An empty slice yields a dangling pointer. Godot reads the argument array even for
        // zero-argument methods, so it must be null instead.
        let args_ptr = if args.is_empty() {
            std::ptr::null()
        } else {
            args.as_ptr()
        };

        R::from_ptrcall(|ret| {
            sys::interface_fn!(object_method_bind_ptrcall)(self.ptr, instance, args_ptr, ret);
        })
    }

    /// Calls the method through the Variant ("varcall") path.
    ///
    /// This is the route for variadic methods -- `emit_signal`, `call`, ... -- which have no
    /// fixed native signature and therefore cannot go through ptrcall.
    ///
    /// # Safety
    /// `instance` must be of the right class.
    pub unsafe fn varcall(
        &self,
        instance: sys::GDExtensionObjectPtr,
        args: &[Variant],
    ) -> Result<Variant, sys::GDExtensionCallErrorType> {
        let arg_ptrs: Vec<sys::GDExtensionConstVariantPtr> =
            args.iter().map(|v| v.as_ptr()).collect();

        // Empty slices give dangling pointers; the engine must see null instead.
        let args_ptr = if arg_ptrs.is_empty() {
            std::ptr::null()
        } else {
            arg_ptrs.as_ptr()
        };

        let mut error: sys::GDExtensionCallError = std::mem::zeroed();
        let mut ret = std::mem::MaybeUninit::<Variant>::zeroed();

        sys::interface_fn!(object_method_bind_call)(
            self.ptr,
            instance,
            args_ptr,
            args.len() as sys::GDExtensionInt,
            ret.as_mut_ptr() as sys::GDExtensionUninitializedVariantPtr,
            &mut error,
        );

        if error.error != sys::GDExtensionCallErrorType_GDEXTENSION_CALL_OK {
            return Err(error.error);
        }

        Ok(ret.assume_init())
    }

    /// Same as [`Self::ptrcall`], for methods that return nothing.
    ///
    /// # Safety
    /// As [`Self::ptrcall`].
    pub unsafe fn ptrcall_void(
        &self,
        instance: sys::GDExtensionObjectPtr,
        args: &[sys::GDExtensionConstTypePtr],
    ) {
        let args_ptr = if args.is_empty() {
            std::ptr::null()
        } else {
            args.as_ptr()
        };

        sys::interface_fn!(object_method_bind_ptrcall)(
            self.ptr,
            instance,
            args_ptr,
            std::ptr::null_mut(),
        );
    }
}
