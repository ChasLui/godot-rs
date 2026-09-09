//! Godot's engine-owned containers: `Array`, `Dictionary`, `NodePath` and the `Packed*Array`
//! family.
//!
//! `Array` and `Dictionary` have reference semantics in Godot: cloning one gives another handle
//! to the same container, not a deep copy. The `Packed*Array` types are copy-on-write values.

use super::variant::{FromGodot, ToGodot, Variant};
use crate::builtin::macros::constructor;
use crate::builtin::StringName;
use godot_sys as sys;
use std::mem::MaybeUninit;

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
    super::Vector2i => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_VECTOR2I,
    super::Vector3i => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_VECTOR3I,
    super::Rect2 => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_RECT2,
    super::Rect2i => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_RECT2I,
    super::Transform2D => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_TRANSFORM2D,
    super::Transform3D => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_TRANSFORM3D,
    super::Basis => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_BASIS,
    super::Quaternion => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_QUATERNION,
    super::AABB => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_AABB,
    super::Projection => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PROJECTION,
    super::Callable => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_CALLABLE,
    super::Signal => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_SIGNAL,
    PackedByteArray => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_BYTE_ARRAY,
    PackedInt32Array => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_INT32_ARRAY,
    PackedInt64Array => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_INT64_ARRAY,
    PackedFloat32Array => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_FLOAT32_ARRAY,
    PackedFloat64Array => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_FLOAT64_ARRAY,
    PackedStringArray => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_STRING_ARRAY,
    PackedVector2Array => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_VECTOR2_ARRAY,
    PackedVector3Array => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_VECTOR3_ARRAY,
    PackedColorArray => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_COLOR_ARRAY,
    PackedVector4Array => GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PACKED_VECTOR4_ARRAY,
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
