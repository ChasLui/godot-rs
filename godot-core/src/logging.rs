//! Printing through Godot's own output.
//!
//! The GDExtension interface only exposes error/warning printing directly; ordinary `print` is a
//! *utility function*, looked up by name and hash like a bound method. Going through it is what
//! makes messages appear as plain output rather than as engine warnings.

use crate::builtin::{StringName, ToGodot, Variant};
use godot_sys as sys;
use std::sync::OnceLock;

/// A Godot utility function, resolved once.
struct UtilityFn {
    ptr: sys::GDExtensionPtrUtilityFunction,
}

// Engine-owned, immutable, valid for the process lifetime once resolved.
unsafe impl Send for UtilityFn {}
unsafe impl Sync for UtilityFn {}

impl UtilityFn {
    unsafe fn resolve(name: &str, hash: i64) -> Self {
        let name_sn = StringName::new(name);
        let ptr = sys::interface_fn!(variant_get_ptr_utility_function)(
            name_sn.as_ptr(),
            hash as sys::GDExtensionInt,
        );
        assert!(
            ptr.is_some(),
            "utility function `{name}` (hash {hash}) not found -- \
             the engine's API does not match the one these bindings were generated from"
        );
        Self { ptr }
    }

    /// Calls the function with a single Variant argument.
    unsafe fn call1(&self, arg: &Variant) {
        let f = self.ptr.expect("utility function pointer was null");
        let args: [sys::GDExtensionConstTypePtr; 1] =
            [arg as *const Variant as sys::GDExtensionConstTypePtr];
        // These functions return nothing, so the return slot is null.
        f(std::ptr::null_mut(), args.as_ptr(), 1);
    }
}

macro_rules! define_printer {
    ($fn_name:ident, $godot_name:literal, $hash_const:ident, $doc:literal) => {
        #[doc = $doc]
        ///
        /// # Panics
        /// If called before the extension has been initialized, which cannot happen from within
        /// a registered class.
        pub fn $fn_name(msg: &str) {
            static FUNC: OnceLock<UtilityFn> = OnceLock::new();
            // SAFETY: resolution and the call both require only that the extension is
            // initialized, which `interface_fn!` checks.
            unsafe {
                let func = FUNC.get_or_init(|| {
                    UtilityFn::resolve($godot_name, sys::method_hashes::$hash_const)
                });
                func.call1(&msg.to_variant());
            }
        }
    };
}

define_printer!(
    godot_print,
    "print",
    UTILITY_PRINT,
    "Prints to Godot's output, like GDScript's `print`."
);
define_printer!(
    godot_print_err,
    "printerr",
    UTILITY_PRINTERR,
    "Prints to Godot's error output, like GDScript's `printerr`."
);
define_printer!(
    godot_error,
    "push_error",
    UTILITY_PUSH_ERROR,
    "Reports an error, showing it in the editor's Errors panel."
);
define_printer!(
    godot_warn,
    "push_warning",
    UTILITY_PUSH_WARNING,
    "Reports a warning, showing it in the editor's Errors panel."
);
