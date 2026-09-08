mod math;
mod string;
mod string_name;
mod variant;

pub use math::{Color, Real, Rect2, Rect2i, Vector2, Vector2i, Vector3, Vector3i, Vector4};
pub use string::GString;
pub use string_name::StringName;
pub use variant::{FromGodot, ToGodot, Variant};
