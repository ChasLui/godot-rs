//! Generates methods for Godot's builtin types.
//!
//! These attach to the hand-written structs in `godot-core::builtin`, so the generated code
//! lives in that crate rather than in `godot-bindings`. Only types the crate actually defines
//! are generated, and methods already written by hand are skipped -- notably the vector maths,
//! which is inlined in Rust rather than dispatched through the engine on every call.

use crate::api::Api;
use crate::types::{map_type, rust_safe_name, RustTy};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Godot's name for a builtin, the Rust struct it maps to, and its Variant type tag.
struct BuiltinType {
    godot: &'static str,
    rust: &'static str,
    tag: &'static str,
}

const BUILTINS: &[BuiltinType] = &[
    BuiltinType {
        godot: "String",
        rust: "GString",
        tag: "STRING",
    },
    BuiltinType {
        godot: "StringName",
        rust: "StringName",
        tag: "STRING_NAME",
    },
    BuiltinType {
        godot: "NodePath",
        rust: "NodePath",
        tag: "NODE_PATH",
    },
    BuiltinType {
        godot: "Array",
        rust: "VariantArray",
        tag: "ARRAY",
    },
    BuiltinType {
        godot: "Dictionary",
        rust: "Dictionary",
        tag: "DICTIONARY",
    },
    BuiltinType {
        godot: "Vector2",
        rust: "Vector2",
        tag: "VECTOR2",
    },
    BuiltinType {
        godot: "Vector2i",
        rust: "Vector2i",
        tag: "VECTOR2I",
    },
    BuiltinType {
        godot: "Vector3",
        rust: "Vector3",
        tag: "VECTOR3",
    },
    BuiltinType {
        godot: "Vector3i",
        rust: "Vector3i",
        tag: "VECTOR3I",
    },
    BuiltinType {
        godot: "Vector4",
        rust: "Vector4",
        tag: "VECTOR4",
    },
    BuiltinType {
        godot: "Color",
        rust: "Color",
        tag: "COLOR",
    },
    BuiltinType {
        godot: "Rect2",
        rust: "Rect2",
        tag: "RECT2",
    },
    BuiltinType {
        godot: "Rect2i",
        rust: "Rect2i",
        tag: "RECT2I",
    },
    BuiltinType {
        godot: "Transform2D",
        rust: "Transform2D",
        tag: "TRANSFORM2D",
    },
    BuiltinType {
        godot: "Transform3D",
        rust: "Transform3D",
        tag: "TRANSFORM3D",
    },
    BuiltinType {
        godot: "Basis",
        rust: "Basis",
        tag: "BASIS",
    },
    BuiltinType {
        godot: "Quaternion",
        rust: "Quaternion",
        tag: "QUATERNION",
    },
    BuiltinType {
        godot: "AABB",
        rust: "AABB",
        tag: "AABB",
    },
    BuiltinType {
        godot: "Plane",
        rust: "Plane",
        tag: "PLANE",
    },
    BuiltinType {
        godot: "Projection",
        rust: "Projection",
        tag: "PROJECTION",
    },
    BuiltinType {
        godot: "RID",
        rust: "Rid",
        tag: "RID",
    },
    BuiltinType {
        godot: "PackedByteArray",
        rust: "PackedByteArray",
        tag: "PACKED_BYTE_ARRAY",
    },
    BuiltinType {
        godot: "PackedInt32Array",
        rust: "PackedInt32Array",
        tag: "PACKED_INT32_ARRAY",
    },
    BuiltinType {
        godot: "PackedInt64Array",
        rust: "PackedInt64Array",
        tag: "PACKED_INT64_ARRAY",
    },
    BuiltinType {
        godot: "PackedFloat32Array",
        rust: "PackedFloat32Array",
        tag: "PACKED_FLOAT32_ARRAY",
    },
    BuiltinType {
        godot: "PackedFloat64Array",
        rust: "PackedFloat64Array",
        tag: "PACKED_FLOAT64_ARRAY",
    },
    BuiltinType {
        godot: "PackedStringArray",
        rust: "PackedStringArray",
        tag: "PACKED_STRING_ARRAY",
    },
    BuiltinType {
        godot: "PackedVector2Array",
        rust: "PackedVector2Array",
        tag: "PACKED_VECTOR2_ARRAY",
    },
    BuiltinType {
        godot: "PackedVector3Array",
        rust: "PackedVector3Array",
        tag: "PACKED_VECTOR3_ARRAY",
    },
    BuiltinType {
        godot: "PackedColorArray",
        rust: "PackedColorArray",
        tag: "PACKED_COLOR_ARRAY",
    },
    BuiltinType {
        godot: "PackedVector4Array",
        rust: "PackedVector4Array",
        tag: "PACKED_VECTOR4_ARRAY",
    },
    BuiltinType {
        godot: "Callable",
        rust: "Callable",
        tag: "CALLABLE",
    },
    BuiltinType {
        godot: "Signal",
        rust: "Signal",
        tag: "SIGNAL",
    },
];

/// Methods already written by hand in `godot-core`, which must not be generated a second time.
///
/// The vector maths is here on purpose: computing a dot product in Rust is a few instructions,
/// while dispatching it through the engine is an indirect call plus argument marshalling, and
/// game code calls these constantly.
fn is_hand_written(godot_type: &str, method: &str) -> bool {
    const MATH_TYPES: &[&str] = &["Vector2", "Vector3", "Vector4", "Color"];

    match method {
        "length" | "dot" | "normalized" | "cross" if MATH_TYPES.contains(&godot_type) => true,
        // `is_valid` on RID, and the container conveniences.
        "is_valid" if godot_type == "RID" => true,
        _ => false,
    }
}

pub struct GeneratedBuiltins {
    pub code: String,
    pub type_count: usize,
    pub method_count: usize,
    pub skipped: usize,
}

pub fn generate_builtin_methods(api_json_path: &str) -> GeneratedBuiltins {
    let api = Api::load(api_json_path);
    let by_name: std::collections::HashMap<&str, &crate::api::BuiltinClass> = api
        .builtin_classes
        .iter()
        .map(|b| (b.name.as_str(), b))
        .collect();

    // Builtin methods never take engine classes, so nothing here resolves to an object type.
    let is_class = |_: &str| false;

    let mut out = TokenStream::new();
    let mut method_count = 0usize;
    let mut skipped = 0usize;

    for builtin in BUILTINS {
        let Some(class) = by_name.get(builtin.godot) else {
            continue;
        };

        let rust_ident = format_ident!("{}", builtin.rust);
        let tag_ident = format_ident!(
            "GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_{}",
            builtin.tag
        );

        let mut methods = TokenStream::new();

        for method in &class.methods {
            if method.is_vararg || is_hand_written(builtin.godot, &method.name) {
                skipped += 1;
                continue;
            }

            match generate_method(builtin, &tag_ident, method, &is_class) {
                Some(tokens) => {
                    methods.extend(tokens);
                    method_count += 1;
                }
                None => skipped += 1,
            }
        }

        out.extend(quote! {
            // Argument counts and names come from the engine's signatures, so neither the
            // arity nor the casing is this generator's to fix.
            #[allow(clippy::too_many_arguments)]
            #[allow(non_snake_case)]
            impl #rust_ident {
                #methods
            }
        });
    }

    let code = quote! {
        // Generated from `extension_api.json`. Do not edit.
        use crate::builtin::macros::builtin_method;

        #out
    };

    GeneratedBuiltins {
        code: code.to_string(),
        type_count: BUILTINS.len(),
        method_count,
        skipped,
    }
}

fn generate_method(
    builtin: &BuiltinType,
    tag_ident: &proc_macro2::Ident,
    method: &crate::api::BuiltinMethod,
    is_class: &dyn Fn(&str) -> bool,
) -> Option<TokenStream> {
    let ret_ty = match &method.return_type {
        None => RustTy::Void,
        Some(t) => map_type(t, None, is_class)?,
    };

    let mut arg_names = Vec::new();
    let mut arg_types = Vec::new();
    for arg in &method.arguments {
        let ty = map_type(&arg.type_, arg.meta.as_deref(), is_class)?;
        if ty == RustTy::Void {
            return None;
        }
        arg_names.push(format_ident!("{}", rust_safe_name(&arg.name)));
        arg_types.push(ty);
    }

    let fn_ident = format_ident!("{}", rust_safe_name(&method.name));
    let godot_name = &method.name;
    let hash = method.hash;

    let params: Vec<TokenStream> = arg_names
        .iter()
        .zip(&arg_types)
        .map(|(name, ty)| {
            let ty_tokens = ty.arg_tokens();
            quote!(#name: #ty_tokens)
        })
        .collect();

    let arg_ptrs: Vec<TokenStream> = arg_names
        .iter()
        .zip(&arg_types)
        .map(|(name, ty)| {
            // The pointee type is spelled out: `&T as *const _` gives the compiler nothing to
            // infer from, since the reference itself is also a pointer-like value.
            let owned = ty.owned_tokens();
            if ty.is_by_ref() {
                quote!(#name as *const #owned as ::godot_sys::GDExtensionConstTypePtr)
            } else {
                quote!(&#name as *const #owned as ::godot_sys::GDExtensionConstTypePtr)
            }
        })
        .collect();

    // A const method borrows immutably; anything else may mutate the value in place.
    let (self_param, base_expr) = if method.is_static {
        (quote!(), quote!(::std::ptr::null_mut()))
    } else if method.is_const {
        (
            quote!(&self,),
            quote!(self as *const Self as ::godot_sys::GDExtensionTypePtr),
        )
    } else {
        (
            quote!(&mut self,),
            quote!(self as *mut Self as ::godot_sys::GDExtensionTypePtr),
        )
    };

    let ret_clause = if ret_ty == RustTy::Void {
        quote!()
    } else {
        let t = ret_ty.ret_tokens();
        quote!(-> #t)
    };

    let doc = format!("Calls Godot's `{}::{}`.", builtin.godot, method.name);
    let argc = arg_names.len() as i32;

    // Decided at generation time rather than emitted as a runtime `if`: an empty array's
    // `as_ptr` is dangling, and a zero-argument method needs no array at all.
    let args_setup = if arg_names.is_empty() {
        quote!(let args_ptr = ::std::ptr::null();)
    } else {
        quote! {
            let args: [::godot_sys::GDExtensionConstTypePtr; #argc as usize] = [#(#arg_ptrs),*];
            let args_ptr = args.as_ptr();
        }
    };

    Some(quote! {
        #[doc = #doc]
        pub fn #fn_ident(#self_param #(#params),*) #ret_clause {
            static METHOD: ::std::sync::OnceLock<::godot_sys::GDExtensionPtrBuiltInMethod> =
                ::std::sync::OnceLock::new();
            let method = METHOD.get_or_init(|| unsafe {
                builtin_method(::godot_sys::#tag_ident, #godot_name, #hash)
            });
            let method = method.expect("builtin method pointer was null");

            #args_setup

            // SAFETY: the signature matches the method identified by name and hash.
            unsafe {
                crate::ptrcall::PtrcallRet::from_ptrcall(|ret| {
                    method(#base_expr, args_ptr, ret, #argc);
                })
            }
        }
    })
}
