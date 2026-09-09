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
pub(crate) unsafe fn constructor(
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

/// Rust-conventional names for the generated Godot ones, so container code reads naturally.
///
/// These are one-line forwards; the real implementations are generated from the API dump, which
/// is also where the signature hashes come from -- there are none hard-coded here.
macro_rules! rust_conventions {
    ($($name:ident),* $(,)?) => {
        $(
            impl $name {
                /// Number of elements. Godot spells this `size`.
                pub fn len(&self) -> i64 {
                    self.size()
                }
            }
        )*
    };
}

rust_conventions!(
    VariantArray,
    Dictionary,
    PackedByteArray,
    PackedInt32Array,
    PackedInt64Array,
    PackedFloat32Array,
    PackedFloat64Array,
    PackedStringArray,
    PackedVector2Array,
    PackedVector3Array,
    PackedColorArray,
    PackedVector4Array,
);

impl VariantArray {
    /// Appends a value. Godot spells this `push_back`.
    pub fn push(&mut self, value: &Variant) {
        self.push_back(value);
    }
}

impl PackedStringArray {
    /// Appends a string. Godot spells this `push_back`.
    pub fn push(&mut self, value: &super::GString) {
        self.push_back(value);
    }
}

impl PackedByteArray {
    /// Appends a byte. Godot spells this `push_back`.
    pub fn push(&mut self, value: i64) {
        self.push_back(value);
    }
}

/// A type that can be an element of a Godot typed array.
///
/// Godot stores an array's element type inside the container, so creating one from Rust means
/// telling the engine which type it holds -- hence `variant_type` and, for objects, `class_name`.
pub trait ArrayElement: ToGodot + FromGodot {
    /// The Variant type of the elements.
    fn variant_type() -> sys::GDExtensionVariantType;

    /// For object elements, the engine class name; empty for everything else.
    fn class_name() -> &'static str {
        ""
    }
}

macro_rules! impl_array_element {
    ($($t:ty => $tag:ident),* $(,)?) => {
        $(
            impl ArrayElement for $t {
                fn variant_type() -> sys::GDExtensionVariantType {
                    sys::$tag
                }
            }
        )*
    };
}

impl_array_element!(
    bool => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_BOOL,
    i64 => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_INT,
    f64 => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_FLOAT,
    super::GString => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_STRING,
    StringName => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_STRING_NAME,
    NodePath => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_NODE_PATH,
    Variant => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_NIL,
    VariantArray => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_ARRAY,
    Dictionary => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_DICTIONARY,
    super::Vector2 => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_VECTOR2,
    super::Vector3 => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_VECTOR3,
    super::Vector4 => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_VECTOR4,
    super::Color => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_COLOR,
    super::Plane => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PLANE,
    super::Rid => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_RID,
);

impl<T: crate::obj::GodotObject> ArrayElement for crate::obj::Gd<T> {
    fn variant_type() -> sys::GDExtensionVariantType {
        sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_OBJECT
    }

    fn class_name() -> &'static str {
        T::CLASS_NAME
    }
}

/// An `Array` whose elements the engine constrains to one type.
///
/// Laid out exactly like [`VariantArray`] -- Godot keeps the element type inside the container,
/// not in the handle -- so this is a zero-cost wrapper that adds typed access.
#[repr(transparent)]
pub struct TypedArray<T: ArrayElement> {
    inner: VariantArray,
    _marker: std::marker::PhantomData<T>,
}

impl<T: ArrayElement> TypedArray<T> {
    /// Creates an array the engine knows is typed as `T`.
    ///
    /// Constructor 2 is the one that attaches the element type; using the plain constructor
    /// would produce an untyped array, which the engine rejects where a typed one is expected.
    pub fn new() -> Self {
        let base = VariantArray::new();
        let elem_type = T::variant_type() as i64;
        let class_name = StringName::new(T::class_name());
        let script = Variant::nil();

        // SAFETY: the argument list matches constructor 2's signature
        // (Array base, int type, StringName class_name, Variant script).
        unsafe {
            let mut opaque = MaybeUninit::<[u8; sys::builtin_sizes::SIZE_ARRAY]>::uninit();
            let ctor = constructor(VariantArray::VARIANT_TYPE, 2).unwrap();
            let args: [sys::GDExtensionConstTypePtr; 4] = [
                base.as_ptr(),
                &elem_type as *const i64 as sys::GDExtensionConstTypePtr,
                class_name.as_ptr() as sys::GDExtensionConstTypePtr,
                script.as_ptr() as sys::GDExtensionConstTypePtr,
            ];
            ctor(
                opaque.as_mut_ptr() as sys::GDExtensionUninitializedTypePtr,
                args.as_ptr(),
            );

            Self {
                inner: std::mem::transmute::<[u8; sys::builtin_sizes::SIZE_ARRAY], VariantArray>(
                    opaque.assume_init(),
                ),
                _marker: std::marker::PhantomData,
            }
        }
    }

    pub fn len(&self) -> i64 {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Reads the element at `index`, or `None` if it is out of range or the wrong type.
    pub fn get(&self, index: i64) -> Option<T> {
        T::try_from_variant(&self.inner.get(index))
    }

    pub fn push(&mut self, value: &T) {
        self.inner.push(&value.to_variant());
    }

    /// Drops the element type, giving an untyped view of the same container.
    pub fn to_untyped(&self) -> VariantArray {
        self.inner.clone()
    }

    pub fn as_ptr(&self) -> sys::GDExtensionConstTypePtr {
        self.inner.as_ptr()
    }
}

impl<T: ArrayElement> Default for TypedArray<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ArrayElement> Clone for TypedArray<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _marker: std::marker::PhantomData,
        }
    }
}

// Same representation as VariantArray, so the marshalling is the same too.
unsafe impl<T: ArrayElement> crate::ptrcall::PtrcallArg for TypedArray<T> {}

unsafe impl<T: ArrayElement> crate::ptrcall::PtrcallRet for TypedArray<T> {
    unsafe fn from_ptrcall<F>(call: F) -> Self
    where
        F: FnOnce(sys::GDExtensionTypePtr),
    {
        Self {
            inner: VariantArray::from_ptrcall(call),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T: ArrayElement> ToGodot for TypedArray<T> {
    fn to_variant(&self) -> Variant {
        self.inner.to_variant()
    }
}

impl<T: ArrayElement> FromGodot for TypedArray<T> {
    fn try_from_variant(variant: &Variant) -> Option<Self> {
        // The engine guarantees the element type; only the container type is checked here.
        VariantArray::try_from_variant(variant).map(|inner| Self {
            inner,
            _marker: std::marker::PhantomData,
        })
    }
}
