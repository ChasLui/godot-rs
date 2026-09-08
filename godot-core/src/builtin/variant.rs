use godot_sys as sys;
use std::mem::MaybeUninit;

/// Godot's dynamically typed value.
///
/// The representation is opaque: its size comes from `extension_api.json` for the engine build
/// configuration in use, and every operation goes through the engine. Do not assume a layout.
#[repr(C)]
pub struct Variant {
    opaque: [u8; sys::builtin_sizes::SIZE_VARIANT],
}

impl Variant {
    /// The `null` value.
    pub fn nil() -> Self {
        unsafe {
            let mut opaque = MaybeUninit::<[u8; sys::builtin_sizes::SIZE_VARIANT]>::uninit();
            sys::interface_fn!(variant_new_nil)(
                opaque.as_mut_ptr() as sys::GDExtensionUninitializedVariantPtr
            );
            Self {
                opaque: opaque.assume_init(),
            }
        }
    }

    /// Which Godot type this value currently holds.
    pub fn get_type(&self) -> sys::GDExtensionVariantType {
        unsafe { sys::interface_fn!(variant_get_type)(self.as_ptr()) }
    }

    pub fn is_nil(&self) -> bool {
        self.get_type() == sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_NIL
    }

    pub fn as_ptr(&self) -> sys::GDExtensionConstVariantPtr {
        self.opaque.as_ptr() as sys::GDExtensionConstVariantPtr
    }

    pub fn as_mut_ptr(&mut self) -> sys::GDExtensionVariantPtr {
        self.opaque.as_mut_ptr() as sys::GDExtensionVariantPtr
    }

    /// Wraps a Variant the engine owns, copying it.
    ///
    /// # Safety
    /// `ptr` must point to a valid, initialized Variant.
    pub unsafe fn from_sys_copy(ptr: sys::GDExtensionConstVariantPtr) -> Self {
        let mut opaque = MaybeUninit::<[u8; sys::builtin_sizes::SIZE_VARIANT]>::uninit();
        sys::interface_fn!(variant_new_copy)(
            opaque.as_mut_ptr() as sys::GDExtensionUninitializedVariantPtr,
            ptr,
        );
        Self {
            opaque: opaque.assume_init(),
        }
    }

    /// Moves this value into engine-owned storage, leaving `self` consumed.
    ///
    /// # Safety
    /// `dest` must be uninitialized storage of Variant size, which the engine then owns.
    pub unsafe fn move_into(self, dest: sys::GDExtensionVariantPtr) {
        sys::interface_fn!(variant_new_copy)(
            dest as sys::GDExtensionUninitializedVariantPtr,
            self.as_ptr(),
        );
    }

    /// Builds a Variant from the memory representation of a builtin type.
    ///
    /// # Safety
    /// `value_ptr` must point to an initialized value of exactly the Godot type `ty`.
    pub(crate) unsafe fn from_builtin(
        ty: sys::GDExtensionVariantType,
        value_ptr: sys::GDExtensionTypePtr,
    ) -> Self {
        let constructor = sys::interface_fn!(get_variant_from_type_constructor)(ty)
            .expect("engine returned no Variant constructor for this type");

        let mut opaque = MaybeUninit::<[u8; sys::builtin_sizes::SIZE_VARIANT]>::uninit();
        constructor(
            opaque.as_mut_ptr() as sys::GDExtensionUninitializedVariantPtr,
            value_ptr,
        );
        Self {
            opaque: opaque.assume_init(),
        }
    }

    /// Extracts the memory representation of a builtin type out of this Variant.
    ///
    /// # Safety
    /// `out_ptr` must point to uninitialized storage large enough for Godot type `ty`, and this
    /// Variant must actually hold that type.
    pub(crate) unsafe fn to_builtin(
        &self,
        ty: sys::GDExtensionVariantType,
        out_ptr: sys::GDExtensionTypePtr,
    ) {
        let constructor = sys::interface_fn!(get_variant_to_type_constructor)(ty)
            .expect("engine returned no type constructor for this Variant type");

        constructor(
            out_ptr as sys::GDExtensionUninitializedTypePtr,
            self.as_ptr() as sys::GDExtensionVariantPtr,
        );
    }
}

impl Clone for Variant {
    fn clone(&self) -> Self {
        unsafe { Self::from_sys_copy(self.as_ptr()) }
    }
}

impl Drop for Variant {
    fn drop(&mut self) {
        unsafe {
            sys::interface_fn!(variant_destroy)(self.as_mut_ptr());
        }
    }
}

/// Converts a Rust value into a Godot [`Variant`].
pub trait ToGodot {
    fn to_variant(&self) -> Variant;
}

/// Recovers a Rust value from a Godot [`Variant`].
///
/// Returns `None` when the Variant holds a different type.
pub trait FromGodot: Sized {
    fn try_from_variant(variant: &Variant) -> Option<Self>;
}

/// Implements the conversions for a builtin type whose Rust representation is layout-compatible
/// with Godot's, i.e. the primitives Godot stores directly.
macro_rules! impl_variant_primitive {
    ($rust:ty, $godot_ty:expr) => {
        impl ToGodot for $rust {
            fn to_variant(&self) -> Variant {
                // SAFETY: `self` is exactly the representation Godot expects for $godot_ty.
                unsafe {
                    Variant::from_builtin(
                        $godot_ty,
                        self as *const $rust as sys::GDExtensionTypePtr,
                    )
                }
            }
        }

        impl FromGodot for $rust {
            fn try_from_variant(variant: &Variant) -> Option<Self> {
                if variant.get_type() != $godot_ty {
                    return None;
                }
                // SAFETY: the type was just checked, so the engine writes a valid $rust.
                unsafe {
                    let mut value = std::mem::MaybeUninit::<$rust>::uninit();
                    variant.to_builtin($godot_ty, value.as_mut_ptr() as sys::GDExtensionTypePtr);
                    Some(value.assume_init())
                }
            }
        }
    };
}

// Godot stores every integer as i64 and every float as f64; narrower Rust types must convert
// explicitly rather than reinterpret, so only the exact representations are implemented here.
impl_variant_primitive!(
    i64,
    sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_INT
);
impl_variant_primitive!(
    f64,
    sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_FLOAT
);

impl ToGodot for bool {
    fn to_variant(&self) -> Variant {
        // Godot's bool Variant is backed by a 64-bit integer, not a Rust `bool`.
        let as_int = *self as u8;
        unsafe {
            Variant::from_builtin(
                sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_BOOL,
                &as_int as *const u8 as sys::GDExtensionTypePtr,
            )
        }
    }
}

impl FromGodot for bool {
    fn try_from_variant(variant: &Variant) -> Option<Self> {
        if variant.get_type() != sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_BOOL {
            return None;
        }
        unsafe {
            let mut value: u8 = 0;
            variant.to_builtin(
                sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_BOOL,
                &mut value as *mut u8 as sys::GDExtensionTypePtr,
            );
            Some(value != 0)
        }
    }
}

impl ToGodot for Variant {
    fn to_variant(&self) -> Variant {
        self.clone()
    }
}

impl FromGodot for Variant {
    fn try_from_variant(variant: &Variant) -> Option<Self> {
        Some(variant.clone())
    }
}
