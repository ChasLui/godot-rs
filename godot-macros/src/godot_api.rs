use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::Parser as _;
use syn::spanned::Spanned;
use syn::{FnArg, ImplItem, ImplItemFn, ItemImpl, ReturnType};

pub fn expand(attr: TokenStream, item: TokenStream) -> syn::Result<TokenStream> {
    let (base, is_runtime) = parse_attr(attr)?;
    let mut impl_block: ItemImpl = syn::parse2(item)?;
    let mut signals: Vec<(String, Vec<(String, TokenStream)>)> = Vec::new();

    let self_ty = &impl_block.self_ty;
    let class_name = type_name(self_ty)?;

    let mut exported = Vec::new();
    let mut virtuals = Vec::new();
    let mut properties = Vec::new();
    let mut has_init = false;
    let mut has_on_base_ready = false;
    let mut has_on_recreated = false;
    let mut has_to_string = false;
    let mut has_notification = false;
    let mut has_get = false;
    let mut has_set = false;
    let mut has_property_list = false;

    // Collect the marked methods and strip the marker attributes, so the original `impl` block
    // still compiles as ordinary Rust.
    for item in &mut impl_block.items {
        let ImplItem::Fn(method) = item else { continue };

        if method.sig.ident == "init" {
            has_init = true;
        }
        if method.sig.ident == "on_base_ready" {
            has_on_base_ready = true;
        }
        if method.sig.ident == "on_recreated" {
            has_on_recreated = true;
        }

        let is_func = take_attr(method, "func");
        let is_virtual = take_attr(method, "godot_virtual");

        if take_attr(method, "signal") {
            signals.push(parse_signal(method)?);
            continue;
        }

        if is_func && is_virtual {
            return Err(syn::Error::new(
                method.sig.ident.span(),
                "a method cannot be both #[func] and #[godot_virtual]",
            ));
        }

        // A `#[prop]` getter is also exported, so GDScript and the editor can reach it.
        let prop_setter = take_prop_attr(method)?;

        if is_func || prop_setter.is_some() {
            exported.push(parse_exported(method)?);
        }
        if is_virtual {
            // A few hooks have dedicated slots in the creation info and are never requested by
            // name, so routing them through the trampoline table would never fire.
            match method.sig.ident.to_string().as_str() {
                "to_string" => has_to_string = true,
                "notification" => has_notification = true,
                "get" => has_get = true,
                "set" => has_set = true,
                "get_property_list" => has_property_list = true,
                _ => virtuals.push(parse_virtual(method)?),
            }
        }
        if let Some(setter) = prop_setter {
            properties.push(parse_property(method, setter)?);
        }
    }

    // `#[signal]` methods are declarations, not code; drop their bodies from the impl block.
    impl_block.items.retain(|item| match item {
        ImplItem::Fn(f) => !signals.iter().any(|(name, _)| f.sig.ident == name.as_str()),
        _ => true,
    });

    if !has_init {
        return Err(syn::Error::new(
            impl_block.span(),
            "#[godot_api] requires an `fn init() -> Self` in the impl block",
        ));
    }

    let shims = exported.iter().map(shim_for);
    let registrations = exported.iter().map(|e| {
        let name = &e.godot_name;
        let shim = &e.shim_ident;
        let argc = e.arg_types.len() as u32;
        quote! {
            ::godot::godot_core::method::register_method(::godot::godot_core::method::MethodDecl::<Self> {
                name: #name,
                func: Self::#shim,
                arg_count: #argc,
            });
        }
    });

    let property_registrations = properties.iter().map(|p| {
        let name = &p.name;
        let setter = &p.setter;
        let getter = &p.getter;
        let variant_type = &p.variant_type;
        quote! {
            {
                let (__godot_type, __godot_class) = #variant_type;
                ::godot::godot_core::signal::register_property::<Self>(
                    #name,
                    __godot_type,
                    __godot_class,
                    #setter,
                    #getter,
                );
            }
        }
    });

    let signal_registrations = signals.iter().map(|s| {
        let name = &s.0;
        let args = s.1.iter().map(|(arg_name, ty)| {
            quote! {
                {
                    let (__godot_type, __godot_class) = #ty;
                    ::godot::godot_core::signal::SignalArg {
                        name: #arg_name,
                        variant_type: __godot_type,
                        class_name: __godot_class,
                    }
                }
            }
        });
        quote! {
            ::godot::godot_core::signal::register_signal::<Self>(#name, &[#(#args),*]);
        }
    });

    let virtual_trampolines = virtuals.iter().map(trampoline_for);
    // Names only, for the registration-time check that the base class actually has them.
    let virtual_names = virtuals.iter().map(|v| &v.godot_name);

    let virtual_arms = virtuals.iter().map(|v| {
        let godot_name = &v.godot_name;
        let tramp = &v.trampoline_ident;
        quote! {
            #godot_name => Some(Self::#tramp as ::godot::godot_core::virtuals::VirtualTrampoline),
        }
    });

    // Forwarded only when the user wrote one; the trait's default is a no-op.
    let base_ready_forward = if has_on_base_ready {
        quote! {
            fn on_base_ready(&mut self, base: ::godot::sys::GDExtensionObjectPtr) {
                <Self>::on_base_ready(self, base)
            }
        }
    } else {
        quote!()
    };

    // `_to_string` has its own slot in the creation info; forwarding it through the by-name
    // dispatch would never fire, since Godot does not ask for it that way.
    let to_string_forward = if has_to_string {
        quote! {
            fn godot_to_string(&mut self) -> Option<::godot::godot_core::builtin::GString> {
                Some(<Self>::to_string(self))
            }
        }
    } else {
        quote!()
    };

    let notification_forward = if has_notification {
        quote! {
            fn godot_notification(&mut self, what: i32, reversed: bool) {
                <Self>::notification(self, what, reversed)
            }
        }
    } else {
        quote!()
    };

    let get_forward = if has_get {
        quote! {
            fn godot_get(
                &mut self,
                property: &str,
            ) -> Option<::godot::godot_core::builtin::Variant> {
                <Self>::get(self, property)
            }
        }
    } else {
        quote!()
    };

    let set_forward = if has_set {
        quote! {
            fn godot_set(
                &mut self,
                property: &str,
                value: &::godot::godot_core::builtin::Variant,
            ) -> bool {
                <Self>::set(self, property, value)
            }
        }
    } else {
        quote!()
    };

    let property_list_forward = if has_property_list {
        quote! {
            fn godot_get_property_list(
                &mut self,
            ) -> ::std::vec::Vec<::godot::godot_core::registry::PropertyDesc> {
                <Self>::get_property_list(self)
            }
        }
    } else {
        quote!()
    };

    let recreated_forward = if has_on_recreated {
        quote! {
            fn on_recreated(&mut self) {
                <Self>::on_recreated(self)
            }
        }
    } else {
        quote!()
    };

    let base_name = base.to_string();

    Ok(quote! {
        // Godot's virtual names drive these signatures: `to_string` takes `&mut self` because
        // the engine hook does, not because it ignores Rust's ToString convention.
        #[allow(clippy::wrong_self_convention)]
        #impl_block

        // Generated shims take their names from the user's methods, so their spelling is not
        // the macro's to fix; the lint would fire on code the user never wrote.
        #[doc(hidden)]
        #[allow(non_snake_case)]
        impl #self_ty {
            #(#shims)*
        }

        impl ::godot::godot_core::registry::GodotClass for #self_ty {
            const CLASS_NAME: &'static str = #class_name;
            const BASE_NAME: &'static str = #base_name;
            const IS_RUNTIME: bool = #is_runtime;

            fn init() -> Self {
                <Self>::init()
            }

            fn register_methods() {
                // SAFETY: called by `register_class` right after the class enters ClassDB.
                unsafe {
                    #(#registrations)*
                }
            }

            fn register_properties() {
                // SAFETY: called after `register_methods`, so the accessors already exist.
                unsafe {
                    #(#property_registrations)*
                }
            }

            fn register_signals() {
                // SAFETY: called by `register_class` once the class is in ClassDB.
                unsafe {
                    #(#signal_registrations)*
                }
            }

            #base_ready_forward
            #recreated_forward
            #to_string_forward
            #notification_forward
            #get_forward
            #set_forward
            #property_list_forward

            const VIRTUAL_NAMES: &'static [&'static str] = &[#(#virtual_names),*];

            fn virtual_trampoline(
                name: &str,
            ) -> Option<::godot::godot_core::virtuals::VirtualTrampoline> {
                match name {
                    #(#virtual_arms)*
                    _ => None,
                }
            }
        }

        // Trampolines unpack the engine's ptrcall arguments into the types each method declares.
        #[doc(hidden)]
        #[allow(non_snake_case)]
        impl #self_ty {
            #(#virtual_trampolines)*
        }
    })
}

/// `#[godot_api(base = Node)]`, optionally `#[godot_api(base = Node, runtime)]`.
fn parse_attr(attr: TokenStream) -> syn::Result<(syn::Ident, bool)> {
    if attr.is_empty() {
        return Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[godot_api] needs a base class, e.g. #[godot_api(base = Node)]",
        ));
    }

    let args =
        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated.parse2(attr)?;

    let mut base = None;
    let mut runtime = false;

    for arg in args {
        match arg {
            syn::Meta::NameValue(meta) if meta.path.is_ident("base") => match meta.value {
                syn::Expr::Path(path) => {
                    base = path.path.get_ident().cloned();
                    if base.is_none() {
                        return Err(syn::Error::new(
                            path.span(),
                            "base must be a plain class name",
                        ));
                    }
                }
                other => {
                    return Err(syn::Error::new(
                        other.span(),
                        "base must be a plain class name",
                    ))
                }
            },
            syn::Meta::Path(path) if path.is_ident("runtime") => runtime = true,
            other => {
                return Err(syn::Error::new(
                    other.span(),
                    "expected `base = ClassName` or `runtime`",
                ))
            }
        }
    }

    let base = base.ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[godot_api] needs a base class, e.g. #[godot_api(base = Node)]",
        )
    })?;

    Ok((base, runtime))
}

fn type_name(ty: &syn::Type) -> syn::Result<String> {
    match ty {
        syn::Type::Path(p) => p
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .ok_or_else(|| syn::Error::new(ty.span(), "cannot determine class name")),
        _ => Err(syn::Error::new(
            ty.span(),
            "#[godot_api] must be applied to an inherent impl of a named type",
        )),
    }
}

/// Removes `#[name]` from the method if present, reporting whether it was there.
fn take_attr(method: &mut ImplItemFn, name: &str) -> bool {
    let before = method.attrs.len();
    method.attrs.retain(|a| !a.path().is_ident(name));
    method.attrs.len() != before
}

struct Exported {
    ident: syn::Ident,
    shim_ident: syn::Ident,
    godot_name: String,
    arg_types: Vec<syn::Type>,
    has_return: bool,
}

fn parse_exported(method: &ImplItemFn) -> syn::Result<Exported> {
    let ident = method.sig.ident.clone();

    let mut arg_types = Vec::new();
    let mut saw_receiver = false;

    for arg in &method.sig.inputs {
        match arg {
            FnArg::Receiver(recv) => {
                if recv.reference.is_none() || recv.mutability.is_none() {
                    return Err(syn::Error::new(
                        recv.span(),
                        "#[func] methods must take `&mut self`",
                    ));
                }
                saw_receiver = true;
            }
            FnArg::Typed(pat) => arg_types.push((*pat.ty).clone()),
        }
    }

    if !saw_receiver {
        return Err(syn::Error::new(
            method.sig.span(),
            "#[func] methods must take `&mut self`",
        ));
    }

    Ok(Exported {
        shim_ident: format_ident!("__godot_shim_{}", ident),
        godot_name: ident.to_string(),
        ident,
        arg_types,
        has_return: !matches!(method.sig.output, ReturnType::Default),
    })
}

/// Builds the `fn(&mut Self, &[Variant]) -> Variant` the registry expects.
///
/// A wrongly typed argument yields nil rather than a panic: a panic here would cross the FFI
/// boundary back into the engine, which is undefined behaviour.
fn shim_for(exported: &Exported) -> TokenStream {
    let shim_ident = &exported.shim_ident;
    let ident = &exported.ident;

    let conversions = exported.arg_types.iter().enumerate().map(|(i, ty)| {
        let var = format_ident!("arg{}", i);
        quote! {
            let Some(#var) = <#ty as ::godot::godot_core::builtin::FromGodot>::try_from_variant(&args[#i])
            else {
                return ::godot::godot_core::builtin::Variant::nil();
            };
        }
    });

    let arg_idents: Vec<_> = (0..exported.arg_types.len())
        .map(|i| format_ident!("arg{}", i))
        .collect();

    let call = if exported.has_return {
        quote! {
            let result = this.#ident(#(#arg_idents),*);
            ::godot::godot_core::builtin::ToGodot::to_variant(&result)
        }
    } else {
        quote! {
            this.#ident(#(#arg_idents),*);
            ::godot::godot_core::builtin::Variant::nil()
        }
    };

    let expected = exported.arg_types.len();

    quote! {
        #[doc(hidden)]
        fn #shim_ident(
            this: &mut Self,
            args: &[::godot::godot_core::builtin::Variant],
        ) -> ::godot::godot_core::builtin::Variant {
            if args.len() != #expected {
                return ::godot::godot_core::builtin::Variant::nil();
            }
            #(#conversions)*
            #call
        }
    }
}

struct Virtual {
    ident: syn::Ident,
    trampoline_ident: syn::Ident,
    godot_name: String,
    arg_types: Vec<syn::Type>,
    has_return: bool,
}

/// Godot spells its virtuals with a leading underscore (`_ready`), so a Rust `fn ready`
/// overrides `_ready`. Any engine virtual can be named this way; a name the engine does not
/// have is simply never asked for.
fn parse_virtual(method: &ImplItemFn) -> syn::Result<Virtual> {
    let ident = method.sig.ident.clone();

    let mut arg_types = Vec::new();
    let mut saw_receiver = false;

    for arg in &method.sig.inputs {
        match arg {
            FnArg::Receiver(recv) => {
                if recv.reference.is_none() || recv.mutability.is_none() {
                    return Err(syn::Error::new(
                        recv.span(),
                        "#[godot_virtual] methods must take `&mut self`",
                    ));
                }
                saw_receiver = true;
            }
            FnArg::Typed(pat) => arg_types.push((*pat.ty).clone()),
        }
    }

    if !saw_receiver {
        return Err(syn::Error::new(
            method.sig.span(),
            "#[godot_virtual] methods must take `&mut self`",
        ));
    }

    Ok(Virtual {
        trampoline_ident: format_ident!("__godot_virtual_{}", ident),
        godot_name: format!("_{ident}"),
        ident,
        arg_types,
        has_return: !matches!(method.sig.output, ReturnType::Default),
    })
}

/// Builds the function Godot calls: read each argument out of the ptrcall array, invoke the
/// user's method, and write back any return value.
fn trampoline_for(v: &Virtual) -> TokenStream {
    let tramp_ident = &v.trampoline_ident;
    let ident = &v.ident;

    let reads: Vec<TokenStream> = v
        .arg_types
        .iter()
        .enumerate()
        .map(|(i, ty)| {
            let var = format_ident!("arg{}", i);
            quote! {
                let #var = <#ty as ::godot::godot_core::virtuals::FromPtrcallArg>::from_arg(
                    *args.add(#i)
                );
            }
        })
        .collect();

    let arg_idents: Vec<_> = (0..v.arg_types.len())
        .map(|i| format_ident!("arg{}", i))
        .collect();

    let call = if v.has_return {
        quote! {
            let result = this.#ident(#(#arg_idents),*);
            ::godot::godot_core::virtuals::IntoPtrcallRet::into_ret(result, ret);
        }
    } else {
        quote! {
            this.#ident(#(#arg_idents),*);
            let _ = ret;
        }
    };

    quote! {
        #[doc(hidden)]
        unsafe fn #tramp_ident(
            instance: ::godot::sys::GDExtensionClassInstancePtr,
            args: *const ::godot::sys::GDExtensionConstTypePtr,
            ret: ::godot::sys::GDExtensionTypePtr,
        ) {
            let this = &mut *(instance as *mut Self);
            let _ = args;
            #(#reads)*
            #call
        }
    }
}

/// `#[signal] fn damaged(amount: i64, source: GString) {}`
///
/// The body is ignored; the name, argument names and argument types are registered. Declaring
/// the types is what lets the editor's connection dialog and `get_signal_list` show a signal's
/// shape -- an argument left untyped is a Variant, which says nothing.
fn parse_signal(method: &ImplItemFn) -> syn::Result<(String, Vec<(String, TokenStream)>)> {
    let name = method.sig.ident.to_string();

    let mut arg_names = Vec::new();
    for arg in &method.sig.inputs {
        match arg {
            FnArg::Receiver(recv) => {
                return Err(syn::Error::new(
                    recv.span(),
                    "#[signal] declarations must not take self",
                ))
            }
            FnArg::Typed(pat) => {
                let syn::Pat::Ident(ident) = &*pat.pat else {
                    return Err(syn::Error::new(
                        pat.span(),
                        "#[signal] arguments must be plain names",
                    ));
                };
                arg_names.push((ident.ident.to_string(), variant_type_of(&pat.ty)?));
            }
        }
    }

    Ok((name, arg_names))
}

struct Property {
    name: String,
    getter: String,
    setter: String,
    variant_type: TokenStream,
}

/// Reads `#[prop(set = set_speed)]` off a getter, returning the setter name.
fn take_prop_attr(method: &mut ImplItemFn) -> syn::Result<Option<String>> {
    let Some(pos) = method.attrs.iter().position(|a| a.path().is_ident("prop")) else {
        return Ok(None);
    };

    let attr = method.attrs.remove(pos);
    let meta: syn::MetaNameValue = attr.parse_args()?;

    if !meta.path.is_ident("set") {
        return Err(syn::Error::new(
            meta.path.span(),
            "expected #[prop(set = setter_method_name)]",
        ));
    }

    match meta.value {
        syn::Expr::Path(p) => p
            .path
            .get_ident()
            .map(|i| Some(i.to_string()))
            .ok_or_else(|| syn::Error::new(p.span(), "setter must be a method name")),
        other => Err(syn::Error::new(
            other.span(),
            "setter must be a method name",
        )),
    }
}

/// Derives the property from its getter: `get_speed` returning `f64` becomes the `speed`
/// property of Godot type FLOAT.
fn parse_property(method: &ImplItemFn, setter: String) -> syn::Result<Property> {
    let getter = method.sig.ident.to_string();
    let name = getter
        .strip_prefix("get_")
        .ok_or_else(|| {
            syn::Error::new(
                method.sig.ident.span(),
                "a #[prop] getter must be named `get_<property>`",
            )
        })?
        .to_string();

    let ReturnType::Type(_, ty) = &method.sig.output else {
        return Err(syn::Error::new(
            method.sig.span(),
            "a #[prop] getter must return the property's type",
        ));
    };

    Ok(Property {
        name,
        getter,
        setter,
        variant_type: variant_type_of(ty)?,
    })
}

/// Maps the getter's Rust type to Godot's Variant type tag.
fn variant_type_of(ty: &syn::Type) -> syn::Result<TokenStream> {
    let syn::Type::Path(path) = ty else {
        return Err(syn::Error::new(ty.span(), "unsupported property type"));
    };

    let name = path
        .path
        .segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_default();

    let tag = match name.as_str() {
        "bool" => "BOOL",
        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => "INT",
        "f32" | "f64" => "FLOAT",
        "GString" => "STRING",
        "StringName" => "STRING_NAME",
        "NodePath" => "NODE_PATH",
        "Vector2" => "VECTOR2",
        "Vector2i" => "VECTOR2I",
        "Vector3" => "VECTOR3",
        "Vector3i" => "VECTOR3I",
        "Vector4" => "VECTOR4",
        "Rect2" => "RECT2",
        "Rect2i" => "RECT2I",
        "Color" => "COLOR",
        "Transform2D" => "TRANSFORM2D",
        "Transform3D" => "TRANSFORM3D",
        "Basis" => "BASIS",
        "Quaternion" => "QUATERNION",
        "Plane" => "PLANE",
        "AABB" => "AABB",
        "Projection" => "PROJECTION",
        "Rid" => "RID",
        "Callable" => "CALLABLE",
        "Signal" => "SIGNAL",
        "Variant" => "NIL",
        "VariantArray" => "ARRAY",
        "Dictionary" => "DICTIONARY",
        "PackedByteArray" => "PACKED_BYTE_ARRAY",
        "PackedInt32Array" => "PACKED_INT32_ARRAY",
        "PackedInt64Array" => "PACKED_INT64_ARRAY",
        "PackedFloat32Array" => "PACKED_FLOAT32_ARRAY",
        "PackedFloat64Array" => "PACKED_FLOAT64_ARRAY",
        "PackedStringArray" => "PACKED_STRING_ARRAY",
        "PackedVector2Array" => "PACKED_VECTOR2_ARRAY",
        "PackedVector3Array" => "PACKED_VECTOR3_ARRAY",
        "PackedVector4Array" => "PACKED_VECTOR4_ARRAY",
        "PackedColorArray" => "PACKED_COLOR_ARRAY",
        // `Gd<T>` is an object, and the engine wants to know *which* class: without it the
        // inspector shows an untyped object slot and accepts anything dropped on it.
        "Gd" => {
            let class = object_class_of(path).ok_or_else(|| {
                syn::Error::new(
                    ty.span(),
                    "a `Gd` property needs a concrete class, e.g. `Gd<Node>`",
                )
            })?;
            return Ok(quote! {
                (
                    ::godot::sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_OBJECT,
                    <#class as ::godot::obj::GodotObject>::CLASS_NAME,
                )
            });
        }
        other => {
            return Err(syn::Error::new(
                ty.span(),
                format!("property type `{other}` is not supported yet"),
            ))
        }
    };

    let ident = format_ident!("GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_{}", tag);
    Ok(quote!((::godot::sys::#ident, "")))
}

/// The `T` of a `Gd<T>` path, if that is what this is.
fn object_class_of(path: &syn::TypePath) -> Option<TokenStream> {
    let segment = path.path.segments.last()?;
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    match args.args.first()? {
        syn::GenericArgument::Type(inner) => Some(quote!(#inner)),
        _ => None,
    }
}
