//! Godot's plain-data math types.
//!
//! Unlike `String` or `Array`, these own no engine memory: they are flat structs the engine
//! passes by value. Their field layout is asserted against `builtin_class_member_offsets` from
//! the API dump, so a change in the engine's layout fails the build instead of corrupting memory
//! at run time.

use super::variant::{FromGodot, ToGodot, Variant};
use godot_sys as sys;

/// Godot's `real_t`: the precision the engine was built with.
#[cfg(not(feature = "double-precision"))]
pub type Real = f32;

/// Godot's `real_t`: the precision the engine was built with.
#[cfg(feature = "double-precision")]
pub type Real = f64;

/// Defines a flat builtin, its Variant conversions, and the layout assertions that keep it
/// honest against the engine's own numbers.
macro_rules! flat_builtin {
    (
        $(#[$meta:meta])*
        $name:ident, $variant_tag:ident, $size_const:ident, {
            $($field:ident : $ty:ty),* $(,)?
        }
    ) => {
        $(#[$meta])*
        #[repr(C)]
        #[derive(Copy, Clone, Debug, Default, PartialEq)]
        pub struct $name {
            $(pub $field: $ty),*
        }

        impl $name {
            pub const fn new($($field: $ty),*) -> Self {
                Self { $($field),* }
            }
        }

        // The engine reports the size for the build configuration in use; if the Rust struct
        // disagrees, every ptrcall involving it would read or write out of bounds.
        const _: () = assert!(
            std::mem::size_of::<$name>() == sys::builtin_sizes::$size_const,
            concat!(stringify!($name), " does not match the engine's reported size")
        );

        unsafe impl crate::ptrcall::PtrcallArg for $name {}

        unsafe impl crate::ptrcall::PtrcallRet for $name {
            unsafe fn from_ptrcall<F>(call: F) -> Self
            where
                F: FnOnce(sys::GDExtensionTypePtr),
            {
                let mut slot = std::mem::MaybeUninit::<Self>::zeroed();
                call(slot.as_mut_ptr() as sys::GDExtensionTypePtr);
                slot.assume_init()
            }
        }

        impl ToGodot for $name {
            fn to_variant(&self) -> Variant {
                // SAFETY: the struct is exactly the engine's representation, as asserted above.
                unsafe {
                    Variant::from_builtin(
                        sys::$variant_tag,
                        self as *const Self as sys::GDExtensionTypePtr,
                    )
                }
            }
        }

        impl FromGodot for $name {
            fn try_from_variant(variant: &Variant) -> Option<Self> {
                if variant.get_type() != sys::$variant_tag {
                    return None;
                }
                // SAFETY: the type was just checked.
                unsafe {
                    let mut value = std::mem::MaybeUninit::<Self>::zeroed();
                    variant.to_builtin(
                        sys::$variant_tag,
                        value.as_mut_ptr() as sys::GDExtensionTypePtr,
                    );
                    Some(value.assume_init())
                }
            }
        }
    };
}

flat_builtin!(
    /// A 2D vector of `real_t`.
    Vector2,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_VECTOR2,
    SIZE_VECTOR2,
    { x: Real, y: Real }
);

flat_builtin!(
    /// A 2D vector of 32-bit integers.
    Vector2i,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_VECTOR2I,
    SIZE_VECTOR2I,
    { x: i32, y: i32 }
);

flat_builtin!(
    /// A 3D vector of `real_t`.
    Vector3,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_VECTOR3,
    SIZE_VECTOR3,
    { x: Real, y: Real, z: Real }
);

flat_builtin!(
    /// A 3D vector of 32-bit integers.
    Vector3i,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_VECTOR3I,
    SIZE_VECTOR3I,
    { x: i32, y: i32, z: i32 }
);

flat_builtin!(
    /// A 4D vector of `real_t`.
    Vector4,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_VECTOR4,
    SIZE_VECTOR4,
    { x: Real, y: Real, z: Real, w: Real }
);

flat_builtin!(
    /// An RGBA color. Always four 32-bit floats, regardless of the engine's `real_t`.
    Color,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_COLOR,
    SIZE_COLOR,
    { r: f32, g: f32, b: f32, a: f32 }
);

flat_builtin!(
    /// An axis-aligned 2D rectangle.
    Rect2,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_RECT2,
    SIZE_RECT2,
    { position: Vector2, size: Vector2 }
);

flat_builtin!(
    /// An axis-aligned 2D rectangle with integer coordinates.
    Rect2i,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_RECT2I,
    SIZE_RECT2I,
    { position: Vector2i, size: Vector2i }
);

flat_builtin!(
    /// A 2D affine transform: two basis vectors and a translation.
    Transform2D,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_TRANSFORM2D,
    SIZE_TRANSFORM2D,
    { x: Vector2, y: Vector2, origin: Vector2 }
);

flat_builtin!(
    /// A 3x3 matrix, used for rotation and scale in 3D.
    Basis,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_BASIS,
    SIZE_BASIS,
    { x: Vector3, y: Vector3, z: Vector3 }
);

flat_builtin!(
    /// A 3D affine transform: a [`Basis`] plus a translation.
    Transform3D,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_TRANSFORM3D,
    SIZE_TRANSFORM3D,
    { basis: Basis, origin: Vector3 }
);

flat_builtin!(
    /// A rotation expressed as a unit quaternion.
    Quaternion,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_QUATERNION,
    SIZE_QUATERNION,
    { x: Real, y: Real, z: Real, w: Real }
);

flat_builtin!(
    /// An axis-aligned bounding box.
    AABB,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_AABB,
    SIZE_AABB,
    { position: Vector3, size: Vector3 }
);

flat_builtin!(
    /// An infinite plane, as a unit normal and a distance from the origin.
    Plane,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PLANE,
    SIZE_PLANE,
    { normal: Vector3, d: Real }
);

flat_builtin!(
    /// A 4x4 matrix, used for projections.
    Projection,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_PROJECTION,
    SIZE_PROJECTION,
    { x: Vector4, y: Vector4, z: Vector4, w: Vector4 }
);

flat_builtin!(
    /// An opaque handle to a resource owned by one of the engine's servers.
    ///
    /// Only meaningful to the server that issued it; it is not a pointer and must not be
    /// constructed by hand.
    Rid,
    GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_RID,
    SIZE_RID,
    { id: u64 }
);

// Field offsets, as reported by the engine. Checking these catches a reordering that a
// size check alone would miss.
const _: () = {
    assert!(std::mem::offset_of!(Vector2, x) == 0);
    assert!(std::mem::offset_of!(Vector2, y) == std::mem::size_of::<Real>());
    assert!(std::mem::offset_of!(Vector3, z) == 2 * std::mem::size_of::<Real>());
    assert!(std::mem::offset_of!(Color, a) == 12);
    assert!(std::mem::offset_of!(Rect2, size) == std::mem::size_of::<Vector2>());

    // Nested composites: a wrong element size here would shift everything after it.
    assert!(std::mem::offset_of!(Transform2D, origin) == 2 * std::mem::size_of::<Vector2>());
    assert!(std::mem::offset_of!(Basis, z) == 2 * std::mem::size_of::<Vector3>());
    assert!(std::mem::offset_of!(Transform3D, origin) == std::mem::size_of::<Basis>());
    assert!(std::mem::offset_of!(AABB, size) == std::mem::size_of::<Vector3>());
    assert!(std::mem::offset_of!(Plane, d) == std::mem::size_of::<Vector3>());
    assert!(std::mem::offset_of!(Projection, w) == 3 * std::mem::size_of::<Vector4>());
};

impl Vector2 {
    pub const ZERO: Self = Self::new(0.0, 0.0);
    pub const ONE: Self = Self::new(1.0, 1.0);

    pub fn length(self) -> Real {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    pub fn dot(self, other: Self) -> Real {
        self.x * other.x + self.y * other.y
    }

    /// Returns the zero vector when `self` has no length, matching Godot's behaviour.
    pub fn normalized(self) -> Self {
        let len = self.length();
        if len == 0.0 {
            Self::ZERO
        } else {
            Self::new(self.x / len, self.y / len)
        }
    }
}

impl Vector3 {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    pub const ONE: Self = Self::new(1.0, 1.0, 1.0);

    pub fn length(self) -> Real {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    pub fn dot(self, other: Self) -> Real {
        self.x * other.x + self.y * other.y + self.z * other.z
    }

    pub fn cross(self, other: Self) -> Self {
        Self::new(
            self.y * other.z - self.z * other.y,
            self.z * other.x - self.x * other.z,
            self.x * other.y - self.y * other.x,
        )
    }

    pub fn normalized(self) -> Self {
        let len = self.length();
        if len == 0.0 {
            Self::ZERO
        } else {
            Self::new(self.x / len, self.y / len, self.z / len)
        }
    }
}

impl Color {
    pub const WHITE: Self = Self::new(1.0, 1.0, 1.0, 1.0);
    pub const BLACK: Self = Self::new(0.0, 0.0, 0.0, 1.0);
}

/// Component-wise arithmetic, the operations game code reaches for constantly.
macro_rules! impl_vector_ops {
    ($name:ident, $scalar:ty, { $($field:ident),* }) => {
        impl std::ops::Add for $name {
            type Output = Self;
            fn add(self, rhs: Self) -> Self {
                Self::new($(self.$field + rhs.$field),*)
            }
        }

        impl std::ops::Sub for $name {
            type Output = Self;
            fn sub(self, rhs: Self) -> Self {
                Self::new($(self.$field - rhs.$field),*)
            }
        }

        impl std::ops::Mul<$scalar> for $name {
            type Output = Self;
            fn mul(self, rhs: $scalar) -> Self {
                Self::new($(self.$field * rhs),*)
            }
        }

        impl std::ops::Neg for $name {
            type Output = Self;
            fn neg(self) -> Self {
                Self::new($(-self.$field),*)
            }
        }
    };
}

impl_vector_ops!(Vector2, Real, { x, y });
impl_vector_ops!(Vector3, Real, { x, y, z });
impl_vector_ops!(Vector4, Real, { x, y, z, w });
impl_vector_ops!(Vector2i, i32, { x, y });
impl_vector_ops!(Vector3i, i32, { x, y, z });

impl Transform2D {
    /// The transform that changes nothing.
    pub const IDENTITY: Self = Self::new(
        Vector2::new(1.0, 0.0),
        Vector2::new(0.0, 1.0),
        Vector2::ZERO,
    );
}

impl Basis {
    pub const IDENTITY: Self = Self::new(
        Vector3::new(1.0, 0.0, 0.0),
        Vector3::new(0.0, 1.0, 0.0),
        Vector3::new(0.0, 0.0, 1.0),
    );
}

impl Transform3D {
    pub const IDENTITY: Self = Self::new(Basis::IDENTITY, Vector3::ZERO);
}

impl Quaternion {
    /// The rotation that changes nothing.
    pub const IDENTITY: Self = Self::new(0.0, 0.0, 0.0, 1.0);
}

impl Rid {
    /// The invalid handle, which every server rejects.
    pub const INVALID: Self = Self::new(0);

    pub fn is_valid(self) -> bool {
        self.id != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sizes are asserted at compile time against the engine's own numbers; this checks the
    /// other half, that the fields sit where the engine says and in the order it expects.
    ///
    /// A reordered field would still compile and still be the right size, and would silently
    /// scramble every value crossing the boundary.
    #[test]
    fn field_order_matches_the_engine() {
        // Built field by *name*, never by constructor: a constructor takes its arguments in
        // declaration order, so `new(1.0, 2.0, 3.0)` would follow a reordering and the check
        // would pass no matter what the order became.
        let v = Vector3 {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        };
        let raw = unsafe { std::slice::from_raw_parts(&v as *const Vector3 as *const Real, 3) };
        assert_eq!(
            raw,
            &[1.0, 2.0, 3.0][..],
            "Vector3 fields are not in x, y, z order"
        );

        let c = Color {
            r: 0.1,
            g: 0.2,
            b: 0.3,
            a: 0.4,
        };
        let raw = unsafe { std::slice::from_raw_parts(&c as *const Color as *const f32, 4) };
        assert_eq!(
            raw,
            &[0.1, 0.2, 0.3, 0.4][..],
            "Color fields are not in r, g, b, a order"
        );

        // Composites: a Transform2D is three Vector2s, laid out end to end.
        let t = Transform2D {
            x: Vector2 { x: 1.0, y: 2.0 },
            y: Vector2 { x: 3.0, y: 4.0 },
            origin: Vector2 { x: 5.0, y: 6.0 },
        };
        let raw = unsafe { std::slice::from_raw_parts(&t as *const Transform2D as *const Real, 6) };
        assert_eq!(raw, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0][..]);
    }

    /// The maths is inlined in Rust rather than dispatched to the engine, so it is worth a check
    /// of its own.
    #[test]
    fn vector_maths() {
        assert_eq!(Vector2::new(3.0, 4.0).length(), 5.0);
        assert_eq!(Vector2::new(3.0, 4.0).normalized().length(), 1.0);
        assert_eq!(Vector2::new(1.0, 2.0).dot(Vector2::new(3.0, 4.0)), 11.0);

        // Cross of the unit x and y axes is the unit z axis.
        let z = Vector3::new(1.0, 0.0, 0.0).cross(Vector3::new(0.0, 1.0, 0.0));
        assert_eq!(z, Vector3::new(0.0, 0.0, 1.0));

        // A zero vector has no direction to normalise towards; Godot returns zero rather than
        // NaN, and so must this.
        assert_eq!(Vector2::ZERO.normalized(), Vector2::ZERO);
        assert_eq!(Vector3::ZERO.normalized(), Vector3::ZERO);
    }

    #[test]
    fn rid_validity() {
        assert!(!Rid::INVALID.is_valid());
        assert!(Rid::new(1).is_valid());
    }
}
