//! Dispatch for Godot's virtual methods (`_ready`, `_process`, `_input`, ...).
//!
//! Godot resolves a virtual once per class through `get_virtual_call_data_func`, caches whatever
//! opaque token comes back, and passes that token to `call_virtual_with_data_func` on every call.
//! Returning null means "not overridden", which is how the engine skips classes that do not
//! implement a hook -- `_process` in particular would otherwise run every frame for nothing.
//!
//! The token used here is a pointer to a *trampoline*: a small function, generated per
//! overridden method, that unpacks the ptrcall arguments into the types the user's method
//! declares. Dispatch is therefore a single indirect call, with no name comparison per
//! invocation and no central table listing every virtual the engine has.

use crate::builtin::StringName;
use crate::registry::GodotClass;
use godot_sys as sys;

/// Unpacks one ptrcall argument and invokes the user's method.
///
/// `instance` is the Rust-side object, `args` the engine's argument array, and `ret` storage for
/// the return value (null when the method returns nothing).
pub type VirtualTrampoline = unsafe fn(
    instance: sys::GDExtensionClassInstancePtr,
    args: *const sys::GDExtensionConstTypePtr,
    ret: sys::GDExtensionTypePtr,
);

/// Reads a ptrcall argument back into a Rust value.
///
/// The inverse of [`crate::ptrcall::PtrcallArg`]: the engine hands a pointer to the native
/// representation, and each type knows how to read itself out of it.
///
/// # Safety
/// The pointer must reference an initialized value of the corresponding engine type.
pub unsafe trait FromPtrcallArg {
    /// # Safety
    /// `ptr` must reference an initialized value of the corresponding engine type.
    unsafe fn from_arg(ptr: sys::GDExtensionConstTypePtr) -> Self;
}

/// Types whose Rust value is the engine's representation, read straight out of the pointer.
macro_rules! impl_from_arg_direct {
    ($($t:ty),* $(,)?) => {
        $(
            unsafe impl FromPtrcallArg for $t {
                unsafe fn from_arg(ptr: sys::GDExtensionConstTypePtr) -> Self {
                    std::ptr::read(ptr as *const Self)
                }
            }
        )*
    };
}

impl_from_arg_direct!(bool, i8, i16, i32, i64, u8, u16, u32, u64, f32, f64);

/// Engine-owned builtins are copied rather than moved: the engine still owns the argument, so
/// taking it by value would free it twice.
macro_rules! impl_from_arg_cloned {
    ($($t:ty),* $(,)?) => {
        $(
            unsafe impl FromPtrcallArg for $t {
                unsafe fn from_arg(ptr: sys::GDExtensionConstTypePtr) -> Self {
                    (*(ptr as *const Self)).clone()
                }
            }
        )*
    };
}

impl_from_arg_cloned!(
    crate::builtin::GString,
    crate::builtin::StringName,
    crate::builtin::NodePath,
    crate::builtin::Variant,
    crate::builtin::VariantArray,
    crate::builtin::Dictionary,
    crate::builtin::Callable,
    crate::builtin::Signal,
    crate::builtin::PackedByteArray,
    crate::builtin::PackedInt32Array,
    crate::builtin::PackedInt64Array,
    crate::builtin::PackedFloat32Array,
    crate::builtin::PackedFloat64Array,
    crate::builtin::PackedStringArray,
    crate::builtin::PackedVector2Array,
    crate::builtin::PackedVector3Array,
    crate::builtin::PackedVector4Array,
    crate::builtin::PackedColorArray,
);

// `TypedArray` is generic, so the macro above cannot reach it, but it reads out the same way.
unsafe impl<T: crate::builtin::ArrayElement> FromPtrcallArg for crate::builtin::TypedArray<T> {
    unsafe fn from_arg(ptr: sys::GDExtensionConstTypePtr) -> Self {
        (*(ptr as *const Self)).clone()
    }
}

// The flat maths types are Copy, so reading them out is a plain load.
impl_from_arg_direct!(
    crate::builtin::Vector2,
    crate::builtin::Vector2i,
    crate::builtin::Vector3,
    crate::builtin::Vector3i,
    crate::builtin::Vector4,
    crate::builtin::Color,
    crate::builtin::Rect2,
    crate::builtin::Rect2i,
    crate::builtin::Transform2D,
    crate::builtin::Transform3D,
    crate::builtin::Basis,
    crate::builtin::Quaternion,
    crate::builtin::AABB,
    crate::builtin::Plane,
    crate::builtin::Projection,
    crate::builtin::Rid,
);

// An object argument arrives as the object pointer itself, and the engine keeps owning it: the
// event handed to `_input` belongs to the caller, which drops it once the frame is over. Taking a
// count is what keeps this handle from freeing it early -- `InputEvent` is reference-counted, so
// without one the object dies the moment the virtual returns, in the middle of the engine's own
// use of it.
unsafe impl<T: crate::obj::GodotObject> FromPtrcallArg for Option<crate::obj::Gd<T>> {
    unsafe fn from_arg(ptr: sys::GDExtensionConstTypePtr) -> Self {
        let obj = std::ptr::read(ptr as *const sys::GDExtensionObjectPtr);
        crate::obj::Gd::from_borrowed_obj_ptr(obj)
    }
}

/// Writes a virtual method's return value into the engine's slot.
///
/// The slot holds a **default-constructed** value, so the write must *assign* -- releasing what
/// is already there -- rather than overwrite it. Godot's `GDVIRTUAL_CALL` declares the slot as
///
/// ```cpp
/// PtrToArg<m_ret>::EncodeT ret;          // default-constructed, not zeroed, not uninitialized
/// call_virtual_with_data(..., &ret);
/// ```
///
/// so `std::ptr::write` here would drop nothing and leak the engine's value. For a `GString`
/// that is an empty string with no allocation behind it, which is why this went unnoticed; for a
/// `Dictionary` or a `PackedStringArray` it is a real leak.
///
/// This is the mirror of the rule in [`crate::ptrcall`], where *the engine* assigns into *our*
/// slot and the slot must therefore be zeroed. Same rule, opposite directions.
///
/// # Safety
/// `ret` must be storage of the right size for `Self`.
pub unsafe trait IntoPtrcallRet {
    /// # Safety
    /// `ret` must reference an initialized value of `Self`, or be null when the engine wants no
    /// return value.
    unsafe fn into_ret(self, ret: sys::GDExtensionTypePtr);
}

unsafe impl<T> IntoPtrcallRet for T {
    unsafe fn into_ret(self, ret: sys::GDExtensionTypePtr) {
        if !ret.is_null() {
            *(ret as *mut T) = self;
        }
    }
}

pub(crate) unsafe extern "C" fn get_virtual_call_data<T: GodotClass>(
    _class_userdata: *mut std::ffi::c_void,
    name: sys::GDExtensionConstStringNamePtr,
    _hash: u32,
) -> *mut std::ffi::c_void {
    // Null already means "not overridden", so it doubles as the fallback: a class that cannot be
    // asked about a virtual simply does not get called for it, which is the quiet failure mode
    // rather than the loud one.
    crate::panics::catch(
        || format!("{}::get_virtual_call_data", T::CLASS_NAME),
        std::ptr::null_mut(),
        || {
            let name = StringName::from_sys_copy(name).to_rust_string();

            match T::virtual_trampoline(&name) {
                // The trampoline pointer *is* the token; Godot only ever hands it back.
                Some(f) => f as *mut std::ffi::c_void,
                // Null tells Godot this class does not override the method, so it stops asking.
                None => std::ptr::null_mut(),
            }
        },
    )
}

pub(crate) unsafe extern "C" fn call_virtual_with_data(
    instance: sys::GDExtensionClassInstancePtr,
    _name: sys::GDExtensionConstStringNamePtr,
    userdata: *mut std::ffi::c_void,
    args: *const sys::GDExtensionConstTypePtr,
    ret: sys::GDExtensionTypePtr,
) {
    if instance.is_null() || userdata.is_null() {
        return;
    }

    // SAFETY: `userdata` is the trampoline this class returned from `get_virtual_call_data`.
    let trampoline: VirtualTrampoline = std::mem::transmute(userdata);
    crate::panics::catch(
        || "a virtual method".to_string(),
        (),
        || trampoline(instance, args, ret),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `FromPtrcallArg` is what makes a type usable as a `#[godot_virtual]` argument. The outgoing
    /// direction has had every builtin since the ptrcall marshalling was written; this is the
    /// incoming one catching up, and the engine virtuals that take these live on the editor and
    /// server extension classes, which a headless run cannot reach. So the assertion is that it
    /// compiles: drop one of the impls and this stops building.
    #[test]
    fn every_container_builtin_can_arrive_as_a_virtual_argument() {
        fn accepts<T: FromPtrcallArg>() {}

        accepts::<crate::builtin::PackedByteArray>();
        accepts::<crate::builtin::PackedInt32Array>();
        accepts::<crate::builtin::PackedInt64Array>();
        accepts::<crate::builtin::PackedFloat32Array>();
        accepts::<crate::builtin::PackedFloat64Array>();
        accepts::<crate::builtin::PackedStringArray>();
        accepts::<crate::builtin::PackedVector2Array>();
        accepts::<crate::builtin::PackedVector3Array>();
        accepts::<crate::builtin::PackedVector4Array>();
        accepts::<crate::builtin::PackedColorArray>();
        accepts::<crate::builtin::Callable>();
        accepts::<crate::builtin::Signal>();
        accepts::<crate::builtin::TypedArray<crate::builtin::GString>>();

        // The ones that were already there, so a refactor cannot quietly narrow the set.
        accepts::<crate::builtin::GString>();
        accepts::<crate::builtin::Variant>();
        accepts::<crate::builtin::VariantArray>();
        accepts::<crate::builtin::Dictionary>();
    }
}
