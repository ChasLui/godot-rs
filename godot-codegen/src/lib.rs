//! Generates Rust bindings from Godot's `extension_api.json`.
//!
//! Only a subset of the engine's 1000+ classes is generated: the transitive closure of a seed
//! set over everything those classes' signatures mention. Generating all of them would make
//! compile times unusable, and most projects touch a small fraction. Methods whose types are not
//! supported yet are skipped and *counted* -- the build prints what it dropped, so the coverage
//! gap is never silent.

pub mod api;
pub mod types;

use api::{Api, Class};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::collections::{HashMap, HashSet};
use types::{map_type, rust_safe_name, RustTy};

/// Classes the closure starts from: enough to write ordinary game logic.
///
/// Kept deliberately small. Each seed drags in everything its signatures mention, and the
/// generated code is the bulk of this crate's compile time; adding a seed is a decision to pay
/// for its whole dependency closure.
pub const SEED_CLASSES: &[&str] = &[
    // Core object model and the scene tree.
    "Object",
    "RefCounted",
    "Node",
    "CanvasItem",
    "Node2D",
    "Node3D",
    "Resource",
    "SceneTree",
    "Viewport",
    "Window",
    // Engine services.
    "Input",
    "OS",
    "Engine",
    "Time",
    "PackedScene",
    "ResourceLoader",
    "RandomNumberGenerator",
    // Common building blocks for actual games.
    "Timer",
    "Control",
    "Label",
    "Button",
    "Sprite2D",
    "AnimatedSprite2D",
    "Camera2D",
    "Camera3D",
    "MeshInstance3D",
    "Area2D",
    "RigidBody2D",
    "CharacterBody2D",
    "CollisionShape2D",
    "Marker2D",
    "Path2D",
    "PathFollow2D",
    "AudioStreamPlayer",
];

/// Additional seeds for editor tooling, pulled in only with the `editor` feature.
pub const EDITOR_SEED_CLASSES: &[&str] = &["EditorPlugin", "EditorInterface"];

pub struct Generated {
    pub code: String,
    pub class_count: usize,
    pub method_count: usize,
    pub skipped_methods: usize,
}

pub fn generate(api_json_path: &str) -> Generated {
    generate_with(api_json_path, false)
}

/// `include_editor` additionally generates editor-only classes (`EditorPlugin` and friends).
///
/// They are off by default: an extension that references them fails to load in an exported
/// project, where the editor classes do not exist.
pub fn generate_with(api_json_path: &str, include_editor: bool) -> Generated {
    let api = Api::load(api_json_path);

    let class_map: HashMap<&str, &Class> =
        api.classes.iter().map(|c| (c.name.as_str(), c)).collect();
    let is_class = |name: &str| class_map.contains_key(name);

    let selected = transitive_closure(&api, &class_map, include_editor);

    // Deterministic order: the generated file must not churn between builds.
    let mut ordered: Vec<&str> = selected.iter().copied().collect();
    ordered.sort_unstable();

    let mut class_defs = TokenStream::new();
    let mut method_count = 0usize;
    let mut skipped_methods = 0usize;

    for name in &ordered {
        let class = class_map[name];
        let (tokens, generated, skipped) = generate_class(class, &is_class, &selected);
        class_defs.extend(tokens);
        method_count += generated;
        skipped_methods += skipped;
    }

    let inherits = generate_inherits(&class_map, &selected, &ordered);
    let singletons = generate_singletons(&api, &selected);
    let global_enums = generate_global_enums(&api);

    let version = &api.header.version_full_name;
    let precision = &api.header.precision;

    let code = quote! {
        // Generated from Godot's `extension_api.json`. Do not edit.
        // Inner attributes live in the including crate's lib.rs, not here.

        /// The engine build these bindings were generated from.
        pub const GODOT_VERSION: &str = #version;
        /// The `real_t` precision of that build.
        pub const GODOT_PRECISION: &str = #precision;

        pub mod global {
            #global_enums
        }

        pub mod classes {
            #class_defs
            #inherits
            #singletons
        }
    };

    Generated {
        code: prettify(code),
        class_count: ordered.len(),
        method_count,
        skipped_methods,
    }
}

/// Every class reachable from [`SEED_CLASSES`] through inheritance or a signature mention.
fn transitive_closure<'a>(
    api: &'a Api,
    class_map: &HashMap<&'a str, &'a Class>,
    include_editor: bool,
) -> HashSet<&'a str> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut frontier: Vec<&str> = Vec::new();

    let seeds: Vec<&str> = if include_editor {
        SEED_CLASSES
            .iter()
            .chain(EDITOR_SEED_CLASSES)
            .copied()
            .collect()
    } else {
        SEED_CLASSES.to_vec()
    };

    for seed in &seeds {
        if let Some(class) = class_map.get(seed) {
            if seen.insert(class.name.as_str()) {
                frontier.push(class.name.as_str());
            }
        }
    }

    while let Some(current) = frontier.pop() {
        let class = class_map[current];
        for dep in class_dependencies(class, class_map) {
            if seen.insert(dep) {
                frontier.push(dep);
            }
        }
    }

    // Editor-only classes are dropped unless asked for: an extension that references them
    // fails to load in an exported project, where those classes do not exist.
    if !include_editor {
        seen.retain(|name| class_map[name].api_type != "editor");
    }

    let _ = api;
    seen
}

fn class_dependencies<'a>(
    class: &'a Class,
    class_map: &HashMap<&'a str, &'a Class>,
) -> Vec<&'a str> {
    let mut out = Vec::new();

    let mut push = |type_str: &'a str| {
        let bare = type_str
            .trim_start_matches("const ")
            .trim_end_matches('*')
            .trim();
        if let Some((_, name)) = class_map.get_key_value(bare) {
            out.push(name.name.as_str());
        }
    };

    if let Some(base) = class.inherits.as_deref() {
        push(base);
    }
    for m in &class.methods {
        if let Some(ret) = &m.return_value {
            push(&ret.type_);
        }
        for a in &m.arguments {
            push(&a.type_);
        }
    }
    for p in &class.properties {
        push(&p.type_);
    }
    for s in &class.signals {
        for a in &s.arguments {
            push(&a.type_);
        }
    }

    out
}

fn generate_class(
    class: &Class,
    is_class: &dyn Fn(&str) -> bool,
    selected: &HashSet<&str>,
) -> (TokenStream, usize, usize) {
    let class_ident = format_ident!("{}", class.name);
    let class_name = &class.name;
    let is_refcounted = class.is_refcounted;

    let mut methods = TokenStream::new();
    let mut generated = 0usize;
    let mut skipped = 0usize;

    for method in &class.methods {
        // Virtual methods have no bindable implementation; they are the extension's to override.
        if method.is_virtual {
            skipped += 1;
            continue;
        }

        let Some(hash) = method.hash else {
            skipped += 1;
            continue;
        };

        let generated_tokens = if method.is_vararg {
            generate_vararg_method(class, method, hash, is_class, selected)
        } else {
            generate_method(class, method, hash, is_class, selected)
        };

        match generated_tokens {
            Some(tokens) => {
                methods.extend(tokens);
                generated += 1;
            }
            None => skipped += 1,
        }
    }

    let doc = format!(
        "Godot's `{}` class.{}",
        class.name,
        if class.inherits.is_some() {
            format!(" Inherits `{}`.", class.inherits.as_deref().unwrap())
        } else {
            String::new()
        }
    );

    let tokens = quote! {
        #[doc = #doc]
        ///
        /// Only ever used behind [`Gd`](::godot_core::obj::Gd); it is a marker for the engine
        /// class, not a Rust-side value.
        #[repr(C)]
        pub struct #class_ident {
            _opaque: [u8; 0],
        }

        unsafe impl ::godot_core::obj::GodotObject for #class_ident {
            const CLASS_NAME: &'static str = #class_name;
            const IS_REFCOUNTED: bool = #is_refcounted;
        }

        impl #class_ident {
            #methods
        }
    };

    (tokens, generated, skipped)
}

fn generate_method(
    class: &Class,
    method: &api::Method,
    hash: i64,
    is_class: &dyn Fn(&str) -> bool,
    selected: &HashSet<&str>,
) -> Option<TokenStream> {
    // Return type.
    let ret_ty = match &method.return_value {
        None => RustTy::Void,
        Some(ret) => map_type(&ret.type_, ret.meta.as_deref(), is_class)?,
    };
    if let RustTy::Object(ref name) = ret_ty {
        if !selected.contains(name.as_str()) {
            return None;
        }
    }

    // Arguments.
    let mut arg_names = Vec::new();
    let mut arg_types = Vec::new();
    for arg in &method.arguments {
        let ty = map_type(&arg.type_, arg.meta.as_deref(), is_class)?;
        if let RustTy::Object(ref name) = ty {
            if !selected.contains(name.as_str()) {
                return None;
            }
        }
        if ty == RustTy::Void {
            return None;
        }
        arg_names.push(format_ident!("{}", rust_safe_name(&arg.name)));
        arg_types.push(ty);
    }

    let fn_ident = format_ident!("{}", rust_safe_name(&method.name));
    let class_name = &class.name;
    let method_name = &method.name;

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
        .map(|(name, ty)| match ty {
            // Object parameters arrive as `&Gd<T>` already; everything else is by value.
            RustTy::Object(_) => quote!(::godot_core::ptrcall::PtrcallArg::arg_ptr(#name)),
            _ => quote!(::godot_core::ptrcall::PtrcallArg::arg_ptr(&#name)),
        })
        .collect();

    // The method bind is resolved once, on first use, and reused for the process lifetime.
    let bind_setup = quote! {
        static BIND: ::std::sync::OnceLock<::godot_core::ptrcall::MethodBind> =
            ::std::sync::OnceLock::new();
        let bind = BIND.get_or_init(|| unsafe {
            ::godot_core::ptrcall::MethodBind::resolve(#class_name, #method_name, #hash)
        });
    };

    let doc = format!("Calls `{}::{}`.", class.name, method.name);

    // Static methods have no instance; Godot expects a null object pointer.
    let (self_param, instance_expr) = if method.is_static {
        (quote!(), quote!(::std::ptr::null_mut()))
    } else {
        (
            quote!(this: &::godot_core::obj::Gd<Self>,),
            quote!(::godot_core::obj::Gd::as_obj_ptr(this)),
        )
    };

    let body = if ret_ty == RustTy::Void {
        quote! {
            #bind_setup
            let args = [#(#arg_ptrs),*];
            unsafe { bind.ptrcall_void(#instance_expr, &args) }
        }
    } else {
        quote! {
            #bind_setup
            let args = [#(#arg_ptrs),*];
            unsafe { bind.ptrcall(#instance_expr, &args) }
        }
    };

    // A `-> ()` return type is noise the linter rejects; omit it for void methods.
    let ret_clause = if ret_ty == RustTy::Void {
        quote!()
    } else {
        let ret_tokens = ret_ty.ret_tokens();
        quote!(-> #ret_tokens)
    };

    Some(quote! {
        #[doc = #doc]
        pub fn #fn_ident(#self_param #(#params),*) #ret_clause {
            #body
        }
    })
}

/// Emits `Inherits<Ancestor>` for every class/ancestor pair, so `Gd::upcast_ref` can be checked
/// at compile time instead of taken on trust.
fn generate_inherits(
    class_map: &HashMap<&str, &Class>,
    selected: &HashSet<&str>,
    ordered: &[&str],
) -> TokenStream {
    let mut out = TokenStream::new();

    for name in ordered {
        let class_ident = format_ident!("{}", name);

        // Walk up the inheritance chain, emitting one impl per ancestor that was generated.
        let mut current = class_map[name].inherits.as_deref();
        while let Some(base) = current {
            if selected.contains(base) {
                let base_ident = format_ident!("{}", base);
                out.extend(quote! {
                    unsafe impl ::godot_core::obj::Inherits<#base_ident> for #class_ident {}
                });
            }
            current = class_map.get(base).and_then(|c| c.inherits.as_deref());
        }
    }

    out
}

/// Variadic methods (`emit_signal`, `call`, ...) have no fixed native signature, so they go
/// through the Variant call path. Fixed arguments are still typed; the variadic tail is a
/// `&[Variant]`.
fn generate_vararg_method(
    class: &Class,
    method: &api::Method,
    hash: i64,
    is_class: &dyn Fn(&str) -> bool,
    selected: &HashSet<&str>,
) -> Option<TokenStream> {
    let mut arg_names = Vec::new();
    let mut arg_types = Vec::new();

    for arg in &method.arguments {
        let ty = map_type(&arg.type_, arg.meta.as_deref(), is_class)?;
        if let RustTy::Object(ref name) = ty {
            if !selected.contains(name.as_str()) {
                return None;
            }
        }
        if ty == RustTy::Void || ty == RustTy::Enum {
            // Enums have no Variant conversion of their own yet.
            return None;
        }
        arg_names.push(format_ident!("{}", rust_safe_name(&arg.name)));
        arg_types.push(ty);
    }

    let fn_ident = format_ident!("{}", rust_safe_name(&method.name));
    let class_name = &class.name;
    let method_name = &method.name;

    let params: Vec<TokenStream> = arg_names
        .iter()
        .zip(&arg_types)
        .map(|(name, ty)| {
            let ty_tokens = ty.arg_tokens();
            quote!(#name: #ty_tokens)
        })
        .collect();

    let to_variants: Vec<TokenStream> = arg_names
        .iter()
        .zip(&arg_types)
        .map(|(name, ty)| match ty {
            // Object parameters arrive as `&Gd<T>` already; everything else is by value.
            RustTy::Object(_) => quote!(::godot_core::builtin::ToGodot::to_variant(#name)),
            _ => quote!(::godot_core::builtin::ToGodot::to_variant(&#name)),
        })
        .collect();

    let (self_param, instance_expr) = if method.is_static {
        (quote!(), quote!(::std::ptr::null_mut()))
    } else {
        (
            quote!(this: &::godot_core::obj::Gd<Self>,),
            quote!(::godot_core::obj::Gd::as_obj_ptr(this)),
        )
    };

    let doc = format!(
        "Calls the variadic `{}::{}`. Returns `Err` with Godot's call-error code on failure.",
        class.name, method.name
    );

    Some(quote! {
        #[doc = #doc]
        pub fn #fn_ident(
            #self_param
            #(#params,)*
            varargs: &[::godot_core::builtin::Variant],
        ) -> ::std::result::Result<
            ::godot_core::builtin::Variant,
            ::godot_sys::GDExtensionCallErrorType,
        > {
            static BIND: ::std::sync::OnceLock<::godot_core::ptrcall::MethodBind> =
                ::std::sync::OnceLock::new();
            let bind = BIND.get_or_init(|| unsafe {
                ::godot_core::ptrcall::MethodBind::resolve(#class_name, #method_name, #hash)
            });

            let mut args: ::std::vec::Vec<::godot_core::builtin::Variant> =
                ::std::vec![#(#to_variants),*];
            args.extend(varargs.iter().cloned());

            unsafe { bind.varcall(#instance_expr, &args) }
        }
    })
}

fn generate_singletons(api: &Api, selected: &HashSet<&str>) -> TokenStream {
    let mut out = TokenStream::new();

    let mut singletons: Vec<&api::Singleton> = api
        .singletons
        .iter()
        .filter(|s| selected.contains(s.type_.as_str()))
        .collect();
    singletons.sort_by(|a, b| a.name.cmp(&b.name));

    for singleton in singletons {
        let type_ident = format_ident!("{}", singleton.type_);
        let name = &singleton.name;
        let doc = format!("Returns the `{}` singleton.", singleton.name);

        out.extend(quote! {
            impl #type_ident {
                #[doc = #doc]
                ///
                /// # Panics
                /// If the engine has not created the singleton yet, which happens when this is
                /// called before the matching initialization level.
                pub fn singleton() -> ::godot_core::obj::Gd<Self> {
                    unsafe {
                        let name = ::godot_core::builtin::StringName::new(#name);
                        let ptr = ::godot_sys::interface_fn!(global_get_singleton)(name.as_ptr());
                        ::godot_core::obj::Gd::from_obj_ptr(ptr)
                            .expect(concat!("singleton ", #name, " is not available yet"))
                    }
                }
            }
        });
    }

    out
}

fn generate_global_enums(api: &Api) -> TokenStream {
    let mut out = TokenStream::new();

    for global_enum in &api.global_enums {
        // Skip the `Variant.Type`-style nested names; they are not valid Rust module paths and
        // are not needed by the current surface.
        if global_enum.name.contains('.') {
            continue;
        }

        let mod_ident = format_ident!("{}", global_enum.name);
        let mut consts = TokenStream::new();

        for value in &global_enum.values {
            let const_ident = format_ident!("{}", value.name);
            let val = value.value;
            consts.extend(quote! {
                pub const #const_ident: i64 = #val;
            });
        }

        let doc = format!("Godot's `{}` enum.", global_enum.name);

        out.extend(quote! {
            #[doc = #doc]
            #[allow(non_upper_case_globals)]
            pub mod #mod_ident {
                #consts
            }
        });
    }

    out
}

/// `quote!` emits everything on one line; keep the output readable for debugging.
fn prettify(tokens: TokenStream) -> String {
    let text = tokens.to_string();
    match syn_free_format(&text) {
        Some(pretty) => pretty,
        None => text,
    }
}

/// Minimal line-breaking, so the generated file can be read without pulling in a formatter.
fn syn_free_format(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len() * 2);
    let mut depth = 0usize;

    for ch in text.chars() {
        match ch {
            '{' => {
                depth += 1;
                out.push_str("{\n");
                out.push_str(&"    ".repeat(depth));
            }
            '}' => {
                depth = depth.saturating_sub(1);
                out.push('\n');
                out.push_str(&"    ".repeat(depth));
                out.push_str("}\n");
                out.push_str(&"    ".repeat(depth));
            }
            ';' => {
                out.push_str(";\n");
                out.push_str(&"    ".repeat(depth));
            }
            _ => out.push(ch),
        }
    }

    Some(out)
}
