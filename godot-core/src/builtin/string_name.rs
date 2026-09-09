use godot_sys as sys;
use std::ffi::CString;
use std::mem::MaybeUninit;

/// Godot's interned string type, used for every class, method, property and signal name.
///
/// The engine owns the representation; this is an opaque buffer whose size comes from
/// `extension_api.json` for the build configuration in use. Never assume a layout.
#[repr(C)]
pub struct StringName {
    opaque: [u8; sys::builtin_sizes::SIZE_STRINGNAME],
}

impl StringName {
    /// Builds a `StringName` from a Rust string.
    ///
    /// # Panics
    /// If `s` contains an interior NUL byte.
    pub fn new(s: &str) -> Self {
        let c_string = CString::new(s).expect("StringName must not contain interior NUL bytes");

        // SAFETY: the engine writes a fully initialized StringName into `opaque`. The buffer is
        // exactly the size the engine reports for this build configuration.
        unsafe {
            let mut opaque = MaybeUninit::<[u8; sys::builtin_sizes::SIZE_STRINGNAME]>::uninit();
            sys::interface_fn!(string_name_new_with_utf8_chars)(
                opaque.as_mut_ptr() as sys::GDExtensionUninitializedStringNamePtr,
                c_string.as_ptr(),
            );
            Self {
                opaque: opaque.assume_init(),
            }
        }
    }

    /// Copies a `StringName` the engine owns.
    ///
    /// The interface has no `string_name_new_copy`, so this goes through a Variant. That is
    /// fine for the places it is used -- virtual-method resolution, which happens once per
    /// class -- but it is not a hot path.
    ///
    /// # Safety
    /// `ptr` must point to an initialized `StringName`.
    pub unsafe fn from_sys_copy(ptr: sys::GDExtensionConstStringNamePtr) -> Self {
        use crate::builtin::FromGodot as _;

        let variant = crate::builtin::Variant::from_builtin(
            sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_STRING_NAME,
            ptr as sys::GDExtensionTypePtr,
        );

        Self::try_from_variant(&variant).expect("engine passed a non-StringName")
    }

    /// Pointer for passing this name to the engine as a `const StringName *`.
    pub fn as_ptr(&self) -> sys::GDExtensionConstStringNamePtr {
        self.opaque.as_ptr() as sys::GDExtensionConstStringNamePtr
    }

    /// Pointer for passing this name where a mutable `StringName *` is expected.
    pub fn as_mut_ptr(&mut self) -> sys::GDExtensionStringNamePtr {
        self.opaque.as_mut_ptr() as sys::GDExtensionStringNamePtr
    }
}

impl Drop for StringName {
    fn drop(&mut self) {
        // SAFETY: `opaque` holds an initialized StringName; the engine's destructor for that
        // Variant type is the only correct way to release it.
        unsafe {
            let destructor = sys::interface_fn!(variant_get_ptr_destructor)(
                sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_STRING_NAME,
            )
            .expect("engine returned no destructor for StringName");

            destructor(self.opaque.as_mut_ptr() as sys::GDExtensionTypePtr);
        }
    }
}

impl From<&str> for StringName {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl StringName {
    /// Copies the contents out as a Rust string.
    pub fn to_rust_string(&self) -> String {
        use crate::builtin::ToGodot;

        // Note: a StringName Variant has type STRING_NAME, not STRING, so converting it with
        // `GString::try_from_variant` fails the type check and silently yields "". Godot's own
        // stringify is the conversion that works across Variant types.
        let as_variant = ToGodot::to_variant(self);

        // SAFETY: the engine writes an initialized String into the zeroed slot; zeroing matters
        // for the same reason as in ptrcall returns -- stringify assigns into the destination.
        unsafe {
            let mut slot = std::mem::MaybeUninit::<crate::builtin::GString>::zeroed();
            sys::interface_fn!(variant_stringify)(
                as_variant.as_ptr(),
                slot.as_mut_ptr() as sys::GDExtensionStringPtr,
            );
            slot.assume_init().to_rust_string()
        }
    }
}

impl std::fmt::Display for StringName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_rust_string())
    }
}

impl crate::builtin::ToGodot for StringName {
    fn to_variant(&self) -> crate::builtin::Variant {
        // SAFETY: `opaque` holds an initialized Godot StringName.
        unsafe {
            crate::builtin::Variant::from_builtin(
                sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_STRING_NAME,
                self.as_ptr() as sys::GDExtensionTypePtr,
            )
        }
    }
}

impl crate::builtin::FromGodot for StringName {
    fn try_from_variant(variant: &crate::builtin::Variant) -> Option<Self> {
        // Godot converts a String Variant to StringName implicitly in most APIs; do the same
        // here so callers are not forced to distinguish the two on the way in.
        let ty = variant.get_type();
        if ty == sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_STRING {
            let s = crate::builtin::GString::try_from_variant(variant)?;
            return Some(StringName::new(&s.to_rust_string()));
        }
        if ty != sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_STRING_NAME {
            return None;
        }

        // SAFETY: the Variant type was just checked to be StringName.
        unsafe {
            let mut opaque = MaybeUninit::<[u8; sys::builtin_sizes::SIZE_STRINGNAME]>::uninit();
            variant.to_builtin(
                sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_STRING_NAME,
                opaque.as_mut_ptr() as sys::GDExtensionTypePtr,
            );
            Some(Self {
                opaque: opaque.assume_init(),
            })
        }
    }
}

impl Clone for StringName {
    fn clone(&self) -> Self {
        // SAFETY: constructor 1 is the copy constructor for every builtin; `opaque` holds an
        // initialized value.
        unsafe {
            let mut opaque = MaybeUninit::<[u8; sys::builtin_sizes::SIZE_STRINGNAME]>::uninit();
            let ctor = crate::builtin::collection::constructor(
                sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_STRING_NAME,
                1,
            )
            .expect("engine has no copy constructor for StringName");
            let args: [sys::GDExtensionConstTypePtr; 1] =
                [self.as_ptr() as sys::GDExtensionConstTypePtr];
            ctor(
                opaque.as_mut_ptr() as sys::GDExtensionUninitializedTypePtr,
                args.as_ptr(),
            );
            Self {
                opaque: opaque.assume_init(),
            }
        }
    }
}
