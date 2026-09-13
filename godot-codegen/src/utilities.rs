//! Generation of Godot's global utility functions -- `sin`, `randi`, `print`, ...
//!
//! These are not ClassDB methods, so they cannot go through a method bind: they live in their own
//! table, keyed by `(name, hash)`, and are emitted as free functions in `global` rather than as
//! inherent methods on a class.
//!
//! The call convention is the ordinary ptrcall one: every argument is a pointer to its *declared*
//! type's native representation. That is why nothing here special-cases variadics -- an argument
//! the dump types as `Variant` is passed as a Variant pointer whether the function is variadic or
//! not, which is exactly what the engine's `FUNCBINDVR*` binds expect.

use crate::all_classes_available;
use crate::api::{Api, UtilityFunction};
use crate::types::{map_type_with, rust_safe_name, RustTy};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::collections::HashSet;

/// Utility functions the Rust standard library already has, as a handful of CPU instructions.
///
/// All of them are still generated -- a binding with holes in it sends people looking for the
/// hole rather than for the faster call -- but their documentation says so, the way
/// `builtins.rs` refuses to generate `Vector2::length` at all for the same reason.
const HAS_RUST_EQUIVALENT: &[&str] = &[
    "abs",
    "absf",
    "absi",
    "acos",
    "acosh",
    "asin",
    "asinh",
    "atan",
    "atan2",
    "atanh",
    "ceil",
    "ceilf",
    "ceili",
    "clamp",
    "clampf",
    "clampi",
    "cos",
    "cosh",
    "exp",
    "floor",
    "floorf",
    "floori",
    "fmod",
    "is_finite",
    "is_inf",
    "is_nan",
    "log",
    "max",
    "maxf",
    "maxi",
    "min",
    "minf",
    "mini",
    "pow",
    "round",
    "roundf",
    "roundi",
    "sign",
    "signf",
    "signi",
    "sin",
    "sinh",
    "sqrt",
    "tan",
    "tanh",
];

/// Emits every utility function, returning `(tokens, generated, skipped)`.
pub fn generate_utility_functions(
    api: &Api,
    is_class: &dyn Fn(&str) -> bool,
    is_global_enum: &dyn Fn(&str) -> bool,
    selected: &HashSet<&str>,
) -> (TokenStream, usize, usize) {
    // Deterministic order: the generated file must not churn between builds, whatever order the
    // dump happens to list these in.
    let mut ordered: Vec<&UtilityFunction> = api.utility_functions.iter().collect();
    ordered.sort_unstable_by(|a, b| a.name.cmp(&b.name));

    let mut out = TokenStream::new();
    let mut generated = 0usize;
    let mut skipped = 0usize;

    for func in ordered {
        match generate_utility_function(func, is_class, is_global_enum, selected) {
            Some(tokens) => {
                out.extend(tokens);
                generated += 1;
            }
            None => skipped += 1,
        }
    }

    (out, generated, skipped)
}

fn generate_utility_function(
    func: &UtilityFunction,
    is_class: &dyn Fn(&str) -> bool,
    is_global_enum: &dyn Fn(&str) -> bool,
    selected: &HashSet<&str>,
) -> Option<TokenStream> {
    // The return slot follows the declared return type, not the variadic flag: `print` writes
    // nothing and is handed a null slot, while the equally variadic `str` and `max` write
    // unconditionally and would trample a null one.
    let ret_ty = match &func.return_type {
        None => RustTy::Void,
        Some(ty) => map_type_with(ty, None, is_class, is_global_enum)?,
    };
    if !usable(&ret_ty, selected) {
        return None;
    }

    let fn_ident = format_ident!("{}", rust_safe_name(&func.name));
    // The lookup key is the engine's spelling; only the Rust identifier is escaped. `typeof` is
    // resolved as "typeof" but declared as `typeof_`, and swapping the two fails at run time.
    let godot_name = &func.name;
    let hash = func.hash;

    let bind_setup = quote! {
        static BIND: ::std::sync::OnceLock<::godot_core::ptrcall::UtilityBind> =
            ::std::sync::OnceLock::new();
        let bind = BIND.get_or_init(|| unsafe {
            ::godot_core::ptrcall::UtilityBind::resolve(#godot_name, #hash)
        });
    };

    // A `-> ()` return type is noise the linter rejects; omit it for the void functions.
    let ret_clause = if ret_ty == RustTy::Void {
        quote!()
    } else {
        let ret_tokens = ret_ty.ret_tokens();
        quote!(-> #ret_tokens)
    };

    let doc = utility_doc(func);

    if func.is_vararg {
        // Godot gives the variadic functions one or two named arguments, but they are as optional
        // as the rest: `print()` with nothing at all is valid GDScript. Making them required Rust
        // parameters would forbid that, so they are folded into `varargs`.
        return Some(quote! {
            #[doc = #doc]
            pub fn #fn_ident(varargs: &[::godot_core::builtin::Variant]) #ret_clause {
                #bind_setup

                let args: ::std::vec::Vec<::godot_sys::GDExtensionConstTypePtr> = varargs
                    .iter()
                    .map(::godot_core::ptrcall::PtrcallArg::arg_ptr)
                    .collect();

                unsafe { bind.call(&args) }
            }
        });
    }

    let mut arg_names = Vec::new();
    let mut arg_types = Vec::new();
    for arg in &func.arguments {
        let ty = map_type_with(&arg.type_, arg.meta.as_deref(), is_class, is_global_enum)?;
        if ty == RustTy::Void || !usable(&ty, selected) {
            return None;
        }
        arg_names.push(format_ident!("{}", rust_safe_name(&arg.name)));
        arg_types.push(ty);
    }

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
            // By-ref parameters are already references; by-value ones need addressing.
            if ty.is_by_ref() {
                quote!(::godot_core::ptrcall::PtrcallArg::arg_ptr(#name))
            } else {
                quote!(::godot_core::ptrcall::PtrcallArg::arg_ptr(&#name))
            }
        })
        .collect();

    Some(quote! {
        #[doc = #doc]
        pub fn #fn_ident(#(#params),*) #ret_clause {
            #bind_setup

            let args = [#(#arg_ptrs),*];
            unsafe { bind.call(&args) }
        }
    })
}

/// Whether a type can appear in a generated utility function at all.
///
/// A raw pointer would force the function to be `unsafe`, and the engine's description says
/// nothing about what it points at. None of Godot 4's utility functions take one today; if one
/// ever does, dropping it is honest, whereas handing out a safe wrapper around it would not be.
fn usable(ty: &RustTy, selected: &HashSet<&str>) -> bool {
    !ty.requires_unsafe() && all_classes_available(ty, selected)
}

fn utility_doc(func: &UtilityFunction) -> String {
    let mut doc = format!("Calls Godot's `{}` utility function.\n\n", func.name);
    doc.push_str(&crate::markdown_safe(&func.description));

    if func.is_vararg {
        let named: Vec<String> = func
            .arguments
            .iter()
            .map(|arg| format!("`{}`", arg.name))
            .collect();
        doc.push_str(&format!(
            "\n\n# Arguments\nGodot names the first {} argument(s) ({}), but they are as optional \
             as the rest -- `{}()` with nothing at all is valid GDScript. They are therefore part \
             of `varargs` here rather than separate parameters.",
            named.len(),
            named.join(", "),
            func.name,
        ));
    }

    if HAS_RUST_EQUIVALENT.contains(&func.name.as_str()) {
        doc.push_str(
            "\n\n# Cost\nThis is a call into the engine: a table lookup on first use, then an \
             indirect call with every argument marshalled through a pointer. The Rust standard \
             library's equivalent is a CPU instruction or two. Reach for this one only when \
             Godot's exact behaviour is what is wanted.",
        );
    }

    // Only the functions the engine binds through Variant build a `Callable::CallError`, and all
    // of them take Variant arguments.
    if func.arguments.iter().any(|arg| arg.type_ == "Variant") {
        doc.push_str(
            "\n\n# Errors\nArguments are checked by the engine, not here. A mistyped or missing \
             one is reported on Godot's own console and yields a nil `Variant`: the ptrcall path \
             discards the call-error code, so unlike the variadic *class* methods this returns no \
             `Result` to test.",
        );
    }

    doc
}
