//! Godot's engine-owned builtins: `Array`, `Dictionary`, `NodePath` and the `Packed*Array`
//! family.
//!
//! Unlike the math types, these own engine memory and must be constructed, copied and destroyed
//! through the engine. They share one shape -- an opaque buffer whose size comes from the API
//! dump, a copy constructor, and a destructor -- so a single macro defines all of them.
//!
//! Note that `Array` and `Dictionary` have reference semantics in Godot: cloning one gives
//! another handle to the same container, not a deep copy. The `Packed*Array` types are
//! copy-on-write values instead.

use super::variant::{FromGodot, ToGodot, Variant};
use crate::builtin::StringName;
use godot_sys as sys;
use std::mem::MaybeUninit;

/// Resolves a builtin's constructor by index. Index 0 is always the default constructor and
/// index 1 the copy constructor, for every type in this module.
unsafe fn constructor(
    ty: sys::GDExtensionVariantType,
    index: i32,
) -> sys::GDExtensionPtrConstructor {
    let ctor = sys::interface_fn!(variant_get_ptr_constructor)(ty, index);
    assert!(
        ctor.is_some(),
        "engine has no constructor {index} for this builtin"
    );
    ctor
}

/// Resolves a method on a builtin type, by name and signature hash.
pub(crate) unsafe fn builtin_method(
    ty: sys::GDExtensionVariantType,
    name: &str,
    hash: i64,
) -> sys::GDExtensionPtrBuiltInMethod {
    let name_sn = StringName::new(name);
    let method = sys::interface_fn!(variant_get_ptr_builtin_method)(
        ty,
        name_sn.as_ptr(),
        hash as sys::GDExtensionInt,
    );
    assert!(
        method.is_some(),
        "builtin method `{name}` (hash {hash}) not found -- \
         the engine's API does not match the one these bindings were generated from"
    );
    method
}

/// Defines an engine-owned builtin: construction, copying, destruction, and the conversions
/// that let it cross the FFI boundary.
macro_rules! engine_builtin {
    (
        $(#[$meta:meta])*
        $name:ident, $tag:ident, $size_const:ident
    ) => {
        $(#[$meta])*
        #[repr(C)]
        pub struct $name {
            opaque: [u8; sys::builtin_sizes::$size_const],
        }

        impl $name {
            const VARIANT_TYPE: sys::GDExtensionVariantType = sys::$tag;

            /// Constructs an empty value.
            pub fn new() -> Self {
                // SAFETY: the default constructor initializes the whole buffer.
                unsafe {
                    let mut opaque =
                        MaybeUninit::<[u8; sys::builtin_sizes::$size_const]>::uninit();
                    let ctor = constructor(Self::VARIANT_TYPE, 0).unwrap();
                    ctor(
                        opaque.as_mut_ptr() as sys::GDExtensionUninitializedTypePtr,
                        std::ptr::null(),
                    );
                    Self { opaque: opaque.assume_init() }
                }
            }

            pub fn as_ptr(&self) -> sys::GDExtensionConstTypePtr {
                self.opaque.as_ptr() as sys::GDExtensionConstTypePtr
            }

            pub fn as_mut_ptr(&mut self) -> sys::GDExtensionTypePtr {
                self.opaque.as_mut_ptr() as sys::GDExtensionTypePtr
            }

            /// Copies a value the engine owns.
            ///
            /// # Safety
            /// `ptr` must point to an initialized value of this type.
            pub unsafe fn from_sys_copy(ptr: sys::GDExtensionConstTypePtr) -> Self {
                let mut opaque = MaybeUninit::<[u8; sys::builtin_sizes::$size_const]>::uninit();
                let ctor = constructor(Self::VARIANT_TYPE, 1).unwrap();
                let args: [sys::GDExtensionConstTypePtr; 1] = [ptr];
                ctor(
                    opaque.as_mut_ptr() as sys::GDExtensionUninitializedTypePtr,
                    args.as_ptr(),
                );
                Self { opaque: opaque.assume_init() }
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl Clone for $name {
            fn clone(&self) -> Self {
                // SAFETY: `opaque` holds an initialized value of this type.
                unsafe { Self::from_sys_copy(self.as_ptr()) }
            }
        }

        impl Drop for $name {
            fn drop(&mut self) {
                // SAFETY: the engine's own destructor is the only correct way to release this.
                unsafe {
                    let destructor =
                        sys::interface_fn!(variant_get_ptr_destructor)(Self::VARIANT_TYPE)
                            .expect("engine returned no destructor for this builtin");
                    destructor(self.as_mut_ptr());
                }
            }
        }

        unsafe impl crate::ptrcall::PtrcallArg for $name {}

        unsafe impl crate::ptrcall::PtrcallRet for $name {
            unsafe fn from_ptrcall<F>(call: F) -> Self
            where
                F: FnOnce(sys::GDExtensionTypePtr),
            {
                // Zeroed rather than uninitialized: the engine assigns into the return slot,
                // releasing whatever it finds there first. See `ptrcall` for the full reasoning.
                let mut slot = MaybeUninit::<Self>::zeroed();
                call(slot.as_mut_ptr() as sys::GDExtensionTypePtr);
                slot.assume_init()
            }
        }

        impl ToGodot for $name {
            fn to_variant(&self) -> Variant {
                // SAFETY: `opaque` holds an initialized value of exactly this Variant type.
                unsafe {
                    Variant::from_builtin(
                        Self::VARIANT_TYPE,
                        self.as_ptr() as sys::GDExtensionTypePtr,
                    )
                }
            }
        }

        impl FromGodot for $name {
            fn try_from_variant(variant: &Variant) -> Option<Self> {
                if variant.get_type() != Self::VARIANT_TYPE {
                    return None;
                }
                // SAFETY: the type was just checked.
                unsafe {
                    let mut opaque =
                        MaybeUninit::<[u8; sys::builtin_sizes::$size_const]>::zeroed();
                    variant.to_builtin(
                        Self::VARIANT_TYPE,
                        opaque.as_mut_ptr() as sys::GDExtensionTypePtr,
                    );
                    Some(Self { opaque: opaque.assume_init() })
                }
            }
        }
    };
}

engine_builtin!(
    /// A path to a node in the scene tree, pre-parsed by the engine.
    NodePath,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_NODE_PATH,
    SIZE_NODEPATH
);

engine_builtin!(
    /// Godot's dynamically typed array. Reference semantics: cloning shares the container.
    VariantArray,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_ARRAY,
    SIZE_ARRAY
);

engine_builtin!(
    /// Godot's dictionary. Reference semantics: cloning shares the container.
    Dictionary,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_DICTIONARY,
    SIZE_DICTIONARY
);

engine_builtin!(
    /// A tightly packed array of bytes.
    PackedByteArray,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_BYTE_ARRAY,
    SIZE_PACKEDBYTEARRAY
);

engine_builtin!(
    /// A tightly packed array of 32-bit integers.
    PackedInt32Array,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_INT32_ARRAY,
    SIZE_PACKEDINT32ARRAY
);

engine_builtin!(
    /// A tightly packed array of 64-bit integers.
    PackedInt64Array,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_INT64_ARRAY,
    SIZE_PACKEDINT64ARRAY
);

engine_builtin!(
    /// A tightly packed array of 32-bit floats.
    PackedFloat32Array,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_FLOAT32_ARRAY,
    SIZE_PACKEDFLOAT32ARRAY
);

engine_builtin!(
    /// A tightly packed array of 64-bit floats.
    PackedFloat64Array,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_FLOAT64_ARRAY,
    SIZE_PACKEDFLOAT64ARRAY
);

engine_builtin!(
    /// A tightly packed array of strings.
    PackedStringArray,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_STRING_ARRAY,
    SIZE_PACKEDSTRINGARRAY
);

engine_builtin!(
    /// A tightly packed array of [`Vector2`](super::Vector2).
    PackedVector2Array,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_VECTOR2_ARRAY,
    SIZE_PACKEDVECTOR2ARRAY
);

engine_builtin!(
    /// A tightly packed array of [`Vector3`](super::Vector3).
    PackedVector3Array,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_VECTOR3_ARRAY,
    SIZE_PACKEDVECTOR3ARRAY
);

engine_builtin!(
    /// A tightly packed array of [`Color`](super::Color).
    PackedColorArray,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_COLOR_ARRAY,
    SIZE_PACKEDCOLORARRAY
);

engine_builtin!(
    /// A tightly packed array of [`Vector4`](super::Vector4).
    PackedVector4Array,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_VECTOR4_ARRAY,
    SIZE_PACKEDVECTOR4ARRAY
);

impl NodePath {
    /// Parses a path such as `"../Sibling/Child"` or `"Node:property"`.
    ///
    /// Named separately from `new`, which the macro defines as "empty value" for every builtin
    /// in this module.
    pub fn from_path(path: &str) -> Self {
        // Constructor 2 takes a String; going through GString keeps the encoding handling in
        // one place.
        let s = super::GString::new(path);
        unsafe {
            let mut opaque = MaybeUninit::<[u8; sys::builtin_sizes::SIZE_NODEPATH]>::uninit();
            let ctor = constructor(Self::VARIANT_TYPE, 2).unwrap();
            let args: [sys::GDExtensionConstTypePtr; 1] =
                [s.as_ptr() as sys::GDExtensionConstTypePtr];
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

impl From<&str> for NodePath {
    fn from(path: &str) -> Self {
        Self::from_path(path)
    }
}

/// Calls a builtin method that takes `args` and returns `R`.
///
/// # Safety
/// The signature must match the method identified by `name`/`hash`.
unsafe fn call_builtin<R>(
    ty: sys::GDExtensionVariantType,
    name: &str,
    hash: i64,
    base: sys::GDExtensionTypePtr,
    args: &[sys::GDExtensionConstTypePtr],
) -> R
where
    R: crate::ptrcall::PtrcallRet,
{
    let method = builtin_method(ty, name, hash).unwrap();
    let args_ptr = if args.is_empty() {
        std::ptr::null()
    } else {
        args.as_ptr()
    };

    R::from_ptrcall(|ret| {
        method(base, args_ptr, ret, args.len() as i32);
    })
}

/// The handful of operations that make a container usable from Rust without waiting for the
/// full builtin-method generator. Hashes come from the vendored API dump.
macro_rules! impl_len {
    ($name:ident, $size_hash:literal, $empty_hash:literal) => {
        impl $name {
            /// Number of elements.
            pub fn len(&self) -> i64 {
                unsafe {
                    call_builtin(
                        Self::VARIANT_TYPE,
                        "size",
                        $size_hash,
                        self.as_ptr() as sys::GDExtensionTypePtr,
                        &[],
                    )
                }
            }

            pub fn is_empty(&self) -> bool {
                unsafe {
                    call_builtin(
                        Self::VARIANT_TYPE,
                        "is_empty",
                        $empty_hash,
                        self.as_ptr() as sys::GDExtensionTypePtr,
                        &[],
                    )
                }
            }
        }
    };
}

// `size` and `is_empty` share one hash across every builtin, since the signature is identical.
impl_len!(VariantArray, 3173160232, 3918633141);
impl_len!(Dictionary, 3173160232, 3918633141);
impl_len!(PackedByteArray, 3173160232, 3918633141);
impl_len!(PackedInt32Array, 3173160232, 3918633141);
impl_len!(PackedInt64Array, 3173160232, 3918633141);
impl_len!(PackedFloat32Array, 3173160232, 3918633141);
impl_len!(PackedFloat64Array, 3173160232, 3918633141);
impl_len!(PackedStringArray, 3173160232, 3918633141);
impl_len!(PackedVector2Array, 3173160232, 3918633141);
impl_len!(PackedVector3Array, 3173160232, 3918633141);
impl_len!(PackedColorArray, 3173160232, 3918633141);
impl_len!(PackedVector4Array, 3173160232, 3918633141);

impl VariantArray {
    /// Appends a value.
    pub fn push(&mut self, value: &Variant) {
        unsafe {
            let args: [sys::GDExtensionConstTypePtr; 1] =
                [value.as_ptr() as sys::GDExtensionConstTypePtr];
            let _: () = call_builtin(
                Self::VARIANT_TYPE,
                "push_back",
                3316032543,
                self.as_mut_ptr(),
                &args,
            );
        }
    }

    /// Reads the element at `index`; out-of-range yields nil, as it does in GDScript.
    pub fn get(&self, index: i64) -> Variant {
        unsafe {
            let args: [sys::GDExtensionConstTypePtr; 1] =
                [&index as *const i64 as sys::GDExtensionConstTypePtr];
            call_builtin(
                Self::VARIANT_TYPE,
                "get",
                708700221,
                self.as_ptr() as sys::GDExtensionTypePtr,
                &args,
            )
        }
    }
}

impl Dictionary {
    /// Inserts or replaces a value.
    pub fn set(&mut self, key: &Variant, value: &Variant) {
        unsafe {
            let args: [sys::GDExtensionConstTypePtr; 2] = [
                key.as_ptr() as sys::GDExtensionConstTypePtr,
                value.as_ptr() as sys::GDExtensionConstTypePtr,
            ];
            let _: bool = call_builtin(
                Self::VARIANT_TYPE,
                "set",
                2175348267,
                self.as_mut_ptr(),
                &args,
            );
        }
    }

    /// Looks up `key`, returning `default` when absent.
    pub fn get_or(&self, key: &Variant, default: &Variant) -> Variant {
        unsafe {
            let args: [sys::GDExtensionConstTypePtr; 2] = [
                key.as_ptr() as sys::GDExtensionConstTypePtr,
                default.as_ptr() as sys::GDExtensionConstTypePtr,
            ];
            call_builtin(
                Self::VARIANT_TYPE,
                "get",
                2205440559,
                self.as_ptr() as sys::GDExtensionTypePtr,
                &args,
            )
        }
    }

    pub fn has(&self, key: &Variant) -> bool {
        unsafe {
            let args: [sys::GDExtensionConstTypePtr; 1] =
                [key.as_ptr() as sys::GDExtensionConstTypePtr];
            call_builtin(
                Self::VARIANT_TYPE,
                "has",
                3680194679,
                self.as_ptr() as sys::GDExtensionTypePtr,
                &args,
            )
        }
    }
}

impl PackedStringArray {
    /// Appends a string.
    pub fn push(&mut self, value: &super::GString) {
        unsafe {
            let args: [sys::GDExtensionConstTypePtr; 1] =
                [value.as_ptr() as sys::GDExtensionConstTypePtr];
            let _: bool = call_builtin(
                Self::VARIANT_TYPE,
                "push_back",
                816187996,
                self.as_mut_ptr(),
                &args,
            );
        }
    }

    pub fn get(&self, index: i64) -> super::GString {
        unsafe {
            let args: [sys::GDExtensionConstTypePtr; 1] =
                [&index as *const i64 as sys::GDExtensionConstTypePtr];
            call_builtin(
                Self::VARIANT_TYPE,
                "get",
                2162347432,
                self.as_ptr() as sys::GDExtensionTypePtr,
                &args,
            )
        }
    }
}

impl PackedByteArray {
    pub fn push(&mut self, value: i64) {
        unsafe {
            let args: [sys::GDExtensionConstTypePtr; 1] =
                [&value as *const i64 as sys::GDExtensionConstTypePtr];
            let _: bool = call_builtin(
                Self::VARIANT_TYPE,
                "push_back",
                694024632,
                self.as_mut_ptr(),
                &args,
            );
        }
    }

    pub fn get(&self, index: i64) -> i64 {
        unsafe {
            let args: [sys::GDExtensionConstTypePtr; 1] =
                [&index as *const i64 as sys::GDExtensionConstTypePtr];
            call_builtin(
                Self::VARIANT_TYPE,
                "get",
                4103005248,
                self.as_ptr() as sys::GDExtensionTypePtr,
                &args,
            )
        }
    }
}

// `()` is the return type of builtin methods that return nothing.
unsafe impl crate::ptrcall::PtrcallRet for () {
    unsafe fn from_ptrcall<F>(call: F) -> Self
    where
        F: FnOnce(sys::GDExtensionTypePtr),
    {
        call(std::ptr::null_mut());
    }
}
