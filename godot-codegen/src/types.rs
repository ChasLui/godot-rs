//! Mapping from Godot's type strings to Rust types.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// How a Godot type surfaces in Rust, for the subset the generator currently supports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RustTy {
    Void,
    /// A scalar whose Rust value is bit-identical to Godot's.
    Primitive(&'static str),
    /// An opaque builtin wrapper from `godot_core::builtin`.
    Builtin(&'static str),
    /// `Gd<ClassName>` -- always optional on return, since the engine may hand back null.
    Object(String),
    Variant,
    /// An engine enum; Godot passes these as 64-bit integers through ptrcall.
    Enum,
    /// `TypedArray<T>` -- an `Array` whose element type the engine enforces.
    TypedArray(Box<RustTy>),
}

/// Builtins whose Rust struct is `Copy`: flat data the engine passes by value.
///
/// Everything else owns engine memory, so taking it by value in an argument would force the
/// caller to clone at every call site.
fn is_copy_builtin(name: &str) -> bool {
    matches!(
        name,
        "Vector2"
            | "Vector2i"
            | "Vector3"
            | "Vector3i"
            | "Vector4"
            | "Color"
            | "Rect2"
            | "Rect2i"
            | "Transform2D"
            | "Transform3D"
            | "Basis"
            | "Quaternion"
            | "AABB"
            | "Plane"
            | "Projection"
            | "Rid"
    )
}

impl RustTy {
    /// Whether an argument of this type is passed by reference.
    pub fn is_by_ref(&self) -> bool {
        match self {
            RustTy::Object(_) | RustTy::Variant | RustTy::TypedArray(_) => true,
            RustTy::Builtin(name) => !is_copy_builtin(name),
            _ => false,
        }
    }

    /// The Rust type as it appears in an argument position.
    pub fn arg_tokens(&self) -> TokenStream {
        match self {
            RustTy::Void => quote!(()),
            RustTy::Primitive(name) => {
                let ident = format_ident!("{}", *name);
                quote!(#ident)
            }
            RustTy::Builtin(name) => {
                let ident = format_ident!("{}", *name);
                if is_copy_builtin(name) {
                    quote!(::godot_core::builtin::#ident)
                } else {
                    quote!(&::godot_core::builtin::#ident)
                }
            }
            RustTy::Object(class) => {
                let ident = format_ident!("{}", class);
                quote!(&::godot_core::obj::Gd<crate::classes::#ident>)
            }
            RustTy::Variant => quote!(&::godot_core::builtin::Variant),
            RustTy::Enum => quote!(i64),
            RustTy::TypedArray(elem) => {
                // The element appears by value inside the generic, even when it is an object:
                // `TypedArray<Gd<Node>>`, not `TypedArray<&Gd<Node>>`.
                let elem_tokens = elem.owned_tokens();
                quote!(&::godot_core::builtin::TypedArray<#elem_tokens>)
            }
        }
    }

    /// The Rust type when it appears by value: return position, or a generic parameter.
    pub fn owned_tokens(&self) -> TokenStream {
        match self {
            RustTy::Object(class) => {
                let ident = format_ident!("{}", class);
                quote!(::godot_core::obj::Gd<crate::classes::#ident>)
            }
            RustTy::Builtin(name) => {
                let ident = format_ident!("{}", *name);
                quote!(::godot_core::builtin::#ident)
            }
            RustTy::Variant => quote!(::godot_core::builtin::Variant),
            RustTy::TypedArray(elem) => {
                let elem_tokens = elem.owned_tokens();
                quote!(::godot_core::builtin::TypedArray<#elem_tokens>)
            }
            other => other.arg_tokens(),
        }
    }

    /// The Rust type as it appears in return position.
    pub fn ret_tokens(&self) -> TokenStream {
        match self {
            RustTy::Object(class) => {
                let ident = format_ident!("{}", class);
                quote!(Option<::godot_core::obj::Gd<crate::classes::#ident>>)
            }
            other => other.owned_tokens(),
        }
    }
}

/// Godot classes are not builtins, so anything not recognised here is looked up as a class.
///
/// `meta` pins the exact width for numbers: ptrcall passes the native representation, so an
/// `int` with `meta: "int32"` must be marshalled as `i32`, not `i64`.
pub fn map_type(
    godot_type: &str,
    meta: Option<&str>,
    is_class: &dyn Fn(&str) -> bool,
) -> Option<RustTy> {
    // Enums and bitfields both arrive as integers.
    if godot_type.starts_with("enum::") || godot_type.starts_with("bitfield::") {
        return Some(RustTy::Enum);
    }

    // A typed array's element type is resolved recursively; a nested typed array is not
    // something the engine produces, so one level is enough.
    if let Some(elem) = godot_type.strip_prefix("typedarray::") {
        let elem_ty = map_type(elem, None, is_class)?;
        if matches!(elem_ty, RustTy::Void | RustTy::Enum) {
            return None;
        }
        return Some(RustTy::TypedArray(Box::new(elem_ty)));
    }

    // Pointer-typed arguments (native structures) are out of scope.
    if godot_type.contains('*') {
        return None;
    }

    match godot_type {
        "void" => return Some(RustTy::Void),
        "bool" => return Some(RustTy::Primitive("bool")),
        "String" => return Some(RustTy::Builtin("GString")),
        "StringName" => return Some(RustTy::Builtin("StringName")),
        "Variant" => return Some(RustTy::Variant),
        // Flat math types: plain data, passed by value.
        "Vector2" => return Some(RustTy::Builtin("Vector2")),
        "Vector2i" => return Some(RustTy::Builtin("Vector2i")),
        "Vector3" => return Some(RustTy::Builtin("Vector3")),
        "Vector3i" => return Some(RustTy::Builtin("Vector3i")),
        "Vector4" => return Some(RustTy::Builtin("Vector4")),
        "Color" => return Some(RustTy::Builtin("Color")),
        "Rect2" => return Some(RustTy::Builtin("Rect2")),
        "Rect2i" => return Some(RustTy::Builtin("Rect2i")),
        "Transform2D" => return Some(RustTy::Builtin("Transform2D")),
        "Transform3D" => return Some(RustTy::Builtin("Transform3D")),
        "Basis" => return Some(RustTy::Builtin("Basis")),
        "Quaternion" => return Some(RustTy::Builtin("Quaternion")),
        "AABB" => return Some(RustTy::Builtin("AABB")),
        "Plane" => return Some(RustTy::Builtin("Plane")),
        "Projection" => return Some(RustTy::Builtin("Projection")),
        "RID" => return Some(RustTy::Builtin("Rid")),
        "NodePath" => return Some(RustTy::Builtin("NodePath")),
        "Array" => return Some(RustTy::Builtin("VariantArray")),
        "Dictionary" => return Some(RustTy::Builtin("Dictionary")),
        "PackedByteArray" => return Some(RustTy::Builtin("PackedByteArray")),
        "PackedInt32Array" => return Some(RustTy::Builtin("PackedInt32Array")),
        "PackedInt64Array" => return Some(RustTy::Builtin("PackedInt64Array")),
        "PackedFloat32Array" => return Some(RustTy::Builtin("PackedFloat32Array")),
        "PackedFloat64Array" => return Some(RustTy::Builtin("PackedFloat64Array")),
        "PackedStringArray" => return Some(RustTy::Builtin("PackedStringArray")),
        "PackedVector2Array" => return Some(RustTy::Builtin("PackedVector2Array")),
        "PackedVector3Array" => return Some(RustTy::Builtin("PackedVector3Array")),
        "PackedColorArray" => return Some(RustTy::Builtin("PackedColorArray")),
        "PackedVector4Array" => return Some(RustTy::Builtin("PackedVector4Array")),
        _ => {}
    }

    if godot_type == "int" {
        return Some(RustTy::Primitive(match meta {
            Some("int8") => "i8",
            Some("int16") => "i16",
            Some("int32") => "i32",
            Some("int64") | None => "i64",
            Some("uint8") => "u8",
            Some("uint16") => "u16",
            Some("uint32") => "u32",
            Some("uint64") => "u64",
            Some(_) => return None,
        }));
    }

    if godot_type == "float" {
        return Some(RustTy::Primitive(match meta {
            Some("float") => "f32",
            Some("double") | None => "f64",
            Some(_) => return None,
        }));
    }

    if is_class(godot_type) {
        return Some(RustTy::Object(godot_type.to_string()));
    }

    // Remaining builtins (Vector2, Array, Dictionary, Packed*Array, ...) have no wrapper yet.
    None
}

/// Rust keywords that cannot be used as identifiers.
///
/// Carried over from the Godot 3 generator (`bindings-generator/src/lib.rs::rust_safe_name`),
/// which solved exactly this problem for the same source of names.
pub fn rust_safe_name(name: &str) -> String {
    match name {
        "use" | "type" | "loop" | "in" | "override" | "typeof" | "match" | "move" | "box"
        | "ref" | "self" | "Self" | "as" | "break" | "const" | "continue" | "crate" | "else"
        | "enum" | "extern" | "false" | "fn" | "for" | "if" | "impl" | "let" | "mod" | "mut"
        | "pub" | "return" | "static" | "struct" | "super" | "trait" | "true" | "unsafe"
        | "where" | "while" | "abstract" | "become" | "do" | "final" | "macro" | "priv"
        | "unsized" | "virtual" | "yield" | "async" | "await" | "dyn" | "try" => {
            format!("{name}_")
        }
        _ => name.to_string(),
    }
}
