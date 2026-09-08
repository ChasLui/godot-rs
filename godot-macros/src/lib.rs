//! Procedural macros for declaring Godot classes in Rust.

use proc_macro::TokenStream;

mod godot_api;

/// Turns an `impl` block into a Godot class registration.
///
/// ```ignore
/// struct Counter { value: i64 }
///
/// #[godot_api(base = Node)]
/// impl Counter {
///     fn init() -> Self {
///         Self { value: 0 }
///     }
///
///     #[func]
///     fn bump(&mut self, by: i64) -> i64 {
///         self.value += by;
///         self.value
///     }
///
///     #[godot_virtual]
///     fn ready(&mut self) {}
/// }
/// ```
///
/// The class is registered under the type's own name. `#[func]` methods become callable from
/// GDScript, with arguments and return values converted through `FromGodot`/`ToGodot`.
/// `#[godot_virtual]` marks an engine hook (`ready`, `process`, `physics_process`); the macro
/// also declares it in `OVERRIDDEN_VIRTUALS`, which is what makes Godot call it.
#[proc_macro_attribute]
pub fn godot_api(attr: TokenStream, item: TokenStream) -> TokenStream {
    godot_api::expand(attr.into(), item.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
