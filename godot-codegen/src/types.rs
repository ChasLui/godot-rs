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
    /// An engine enum or bitfield, as its generated newtype. Passed through ptrcall as a
    /// 64-bit integer, which is what the newtype wraps.
    ///
    /// `owner` is the class that declares it, or `None` for a global enum; the two live in
    /// different modules and a class-scoped one is only usable if its class was generated.
    Enum {
        name: String,
        owner: Option<String>,
    },
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
            RustTy::Enum { name, owner } => match owner {
                Some(class) => {
                    let ident = format_ident!("{}{}", class, name);
                    quote!(crate::classes::#ident)
                }
                None => {
                    let ident = format_ident!("{}", name);
                    quote!(crate::global::#ident)
                }
            },
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
    // `enum::Error`, `enum::Node.ProcessMode`, `bitfield::PropertyUsageFlags`. Class-scoped
    // names become `<Class><Enum>` to match how the generator emits them.
    for prefix in ["enum::", "bitfield::"] {
        if let Some(rest) = godot_type.strip_prefix(prefix) {
            return Some(match rest.split_once('.') {
                Some((class, name)) => RustTy::Enum {
                    name: name.to_string(),
                    owner: Some(class.to_string()),
                },
                None => RustTy::Enum {
                    name: rest.to_string(),
                    owner: None,
                },
            });
        }
    }

    // A typed array's element type is resolved recursively; a nested typed array is not
    // something the engine produces, so one level is enough.
    if let Some(elem) = godot_type.strip_prefix("typedarray::") {
        let elem_ty = map_type(elem, None, is_class)?;
        if matches!(elem_ty, RustTy::Void | RustTy::Enum { .. }) {
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
        "Callable" => return Some(RustTy::Builtin("Callable")),
        "Signal" => return Some(RustTy::Builtin("Signal")),
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

/// Turns Godot's textual default value into a Rust expression of type `ty`.
///
/// Returns `None` for forms not worth special-casing (object nulls, container literals); a
/// method with one of those simply keeps its full-argument signature and gains no short form,
/// rather than being dropped.
pub fn default_value_expr(ty: &RustTy, raw: &str) -> Option<TokenStream> {
    let raw = raw.trim();

    match ty {
        RustTy::Primitive(name) => {
            match *name {
                "bool" => match raw {
                    "true" => Some(quote!(true)),
                    "false" => Some(quote!(false)),
                    _ => None,
                },
                // Emitted as a literal of the target type rather than a cast: Godot writes
                // some float defaults without a decimal point ("-1"), so the text cannot be
                // passed through, but `0f64 as f64` would be a redundant cast.
                "f32" => {
                    let value = raw.parse::<f64>().ok()? as f32;
                    Some(quote!(#value))
                }
                "f64" => {
                    let value: f64 = raw.parse().ok()?;
                    Some(quote!(#value))
                }
                // Same reasoning as the floats: emit a literal of the exact width so no cast
                // is needed. `quote!` suffixes integers by their Rust type, hence the match.
                _ => {
                    let value: i64 = raw.parse().ok()?;
                    Some(match *name {
                        "i8" => {
                            let v = value as i8;
                            quote!(#v)
                        }
                        "i16" => {
                            let v = value as i16;
                            quote!(#v)
                        }
                        "i32" => {
                            let v = value as i32;
                            quote!(#v)
                        }
                        "i64" => quote!(#value),
                        "u8" => {
                            let v = value as u8;
                            quote!(#v)
                        }
                        "u16" => {
                            let v = value as u16;
                            quote!(#v)
                        }
                        "u32" => {
                            let v = value as u32;
                            quote!(#v)
                        }
                        "u64" => {
                            let v = value as u64;
                            quote!(#v)
                        }
                        _ => return None,
                    })
                }
            }
        }

        RustTy::Enum { .. } => {
            let value: i64 = raw.parse().ok()?;
            let ty_tokens = ty.owned_tokens();
            Some(quote!(#ty_tokens(#value)))
        }

        RustTy::Builtin(name) => match *name {
            // `&""` is how the dump spells an empty StringName.
            "StringName" => {
                let text = strip_quotes(raw.trim_start_matches('&'))?;
                Some(quote!(&::godot_core::builtin::StringName::new(#text)))
            }
            "GString" => {
                let text = strip_quotes(raw)?;
                Some(quote!(&::godot_core::builtin::GString::new(#text)))
            }
            "Color" => {
                let n = parse_call_args("Color", raw)?;
                let [r, g, b, a] = <[f64; 4]>::try_from(n).ok()?;
                let (r, g, b, a) = (r as f32, g as f32, b as f32, a as f32);
                Some(quote!(::godot_core::builtin::Color::new(#r, #g, #b, #a)))
            }
            "Vector2" => {
                let n = parse_call_args("Vector2", raw)?;
                let [x, y] = <[f64; 2]>::try_from(n).ok()?;
                Some(quote!(::godot_core::builtin::Vector2::new(
                    #x as ::godot_core::builtin::Real,
                    #y as ::godot_core::builtin::Real
                )))
            }
            "Vector2i" => {
                let n = parse_call_args("Vector2i", raw)?;
                let [x, y] = <[f64; 2]>::try_from(n).ok()?;
                let (x, y) = (x as i32, y as i32);
                Some(quote!(::godot_core::builtin::Vector2i::new(#x, #y)))
            }
            "Vector3" => {
                let n = parse_call_args("Vector3", raw)?;
                let [x, y, z] = <[f64; 3]>::try_from(n).ok()?;
                Some(quote!(::godot_core::builtin::Vector3::new(
                    #x as ::godot_core::builtin::Real,
                    #y as ::godot_core::builtin::Real,
                    #z as ::godot_core::builtin::Real
                )))
            }
            _ => None,
        },

        _ => None,
    }
}

/// `"abc"` -> `abc`
fn strip_quotes(raw: &str) -> Option<String> {
    let inner = raw.strip_prefix('"')?.strip_suffix('"')?;
    Some(inner.to_string())
}

/// `Color(1, 1, 1, 1)` -> `[1.0, 1.0, 1.0, 1.0]`
fn parse_call_args(name: &str, raw: &str) -> Option<Vec<f64>> {
    let inner = raw
        .strip_prefix(name)?
        .trim()
        .strip_prefix('(')?
        .strip_suffix(')')?;
    inner
        .split(',')
        .map(|part| part.trim().parse::<f64>().ok())
        .collect()
}
