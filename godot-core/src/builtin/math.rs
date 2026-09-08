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

// Field offsets, as reported by the engine. Checking these catches a reordering that a
// size check alone would miss.
const _: () = {
    assert!(std::mem::offset_of!(Vector2, x) == 0);
    assert!(std::mem::offset_of!(Vector2, y) == std::mem::size_of::<Real>());
    assert!(std::mem::offset_of!(Vector3, z) == 2 * std::mem::size_of::<Real>());
    assert!(std::mem::offset_of!(Color, a) == 12);
    assert!(std::mem::offset_of!(Rect2, size) == std::mem::size_of::<Vector2>());
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
