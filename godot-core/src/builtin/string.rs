use super::variant::{FromGodot, ToGodot, Variant};
use godot_sys as sys;
use std::ffi::CString;
use std::mem::MaybeUninit;

/// Godot's `String` type.
///
/// Named `GString` to avoid colliding with [`std::string::String`] in a prelude import.
/// Opaque, like every builtin: the size comes from `extension_api.json`.
#[repr(C)]
pub struct GString {
    opaque: [u8; sys::builtin_sizes::SIZE_STRING],
}

impl GString {
    /// # Panics
    /// If `s` contains an interior NUL byte.
    pub fn new(s: &str) -> Self {
        let c_string = CString::new(s).expect("GString must not contain interior NUL bytes");

        unsafe {
            let mut opaque = MaybeUninit::<[u8; sys::builtin_sizes::SIZE_STRING]>::uninit();
            sys::interface_fn!(string_new_with_utf8_chars)(
                opaque.as_mut_ptr() as sys::GDExtensionUninitializedStringPtr,
                c_string.as_ptr(),
            );
            Self {
                opaque: opaque.assume_init(),
            }
        }
    }

    pub fn as_ptr(&self) -> sys::GDExtensionConstStringPtr {
        self.opaque.as_ptr() as sys::GDExtensionConstStringPtr
    }

    pub fn as_mut_ptr(&mut self) -> sys::GDExtensionStringPtr {
        self.opaque.as_mut_ptr() as sys::GDExtensionStringPtr
    }

    /// Copies the contents out as a Rust string.
    pub fn to_rust_string(&self) -> String {
        unsafe {
            // Called with a null buffer, the engine reports the length it would write.
            let len =
                sys::interface_fn!(string_to_utf8_chars)(self.as_ptr(), std::ptr::null_mut(), 0);

            if len <= 0 {
                return String::new();
            }

            let mut buf = vec![0u8; len as usize];
            sys::interface_fn!(string_to_utf8_chars)(
                self.as_ptr(),
                buf.as_mut_ptr() as *mut std::os::raw::c_char,
                len,
            );

            String::from_utf8_lossy(&buf).into_owned()
        }
    }
}

impl Drop for GString {
    fn drop(&mut self) {
        unsafe {
            let destructor = sys::interface_fn!(variant_get_ptr_destructor)(
                sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_STRING,
            )
            .expect("engine returned no destructor for String");

            destructor(self.opaque.as_mut_ptr() as sys::GDExtensionTypePtr);
        }
    }
}

impl From<&str> for GString {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl std::fmt::Display for GString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_rust_string())
    }
}

impl ToGodot for GString {
    fn to_variant(&self) -> Variant {
        // SAFETY: `opaque` holds an initialized Godot String.
        unsafe {
            Variant::from_builtin(
                sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_STRING,
                self.as_ptr() as sys::GDExtensionTypePtr,
            )
        }
    }
}

impl FromGodot for GString {
    fn try_from_variant(variant: &Variant) -> Option<Self> {
        if variant.get_type() != sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_STRING {
            return None;
        }

        // SAFETY: the Variant type was just checked to be String.
        unsafe {
            let mut opaque = MaybeUninit::<[u8; sys::builtin_sizes::SIZE_STRING]>::uninit();
            variant.to_builtin(
                sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_STRING,
                opaque.as_mut_ptr() as sys::GDExtensionTypePtr,
            );
            Some(Self {
                opaque: opaque.assume_init(),
            })
        }
    }
}

impl ToGodot for &str {
    fn to_variant(&self) -> Variant {
        GString::new(self).to_variant()
    }
}

impl Clone for GString {
    fn clone(&self) -> Self {
        // SAFETY: constructor 1 is the copy constructor for every builtin; `opaque` holds an
        // initialized value.
        unsafe {
            let mut opaque = MaybeUninit::<[u8; sys::builtin_sizes::SIZE_STRING]>::uninit();
            let ctor = crate::builtin::macros::constructor(
                sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_STRING,
                1,
            )
            .expect("engine has no copy constructor for GString");
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
