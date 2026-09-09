#[macro_use]
pub(crate) mod macros;

pub(crate) mod callable;
pub(crate) mod collection;
mod math;
mod string;
mod string_name;
mod variant;

pub use callable::{Callable, Signal};
pub use collection::{
    ArrayElement, Dictionary, NodePath, PackedByteArray, PackedColorArray, PackedFloat32Array,
    PackedFloat64Array, PackedInt32Array, PackedInt64Array, PackedStringArray, PackedVector2Array,
    PackedVector3Array, PackedVector4Array, TypedArray, VariantArray,
};
pub use math::{
    Basis, Color, Plane, Projection, Quaternion, Real, Rect2, Rect2i, Rid, Transform2D,
    Transform3D, Vector2, Vector2i, Vector3, Vector3i, Vector4, AABB,
};
pub use string::GString;
pub use string_name::StringName;
pub use variant::{FromGodot, ToGodot, Variant};

// Methods generated from `extension_api.json`, attached to the structs above.
include!(concat!(env!("OUT_DIR"), "/builtin_methods.rs"));
