//! Printing through Godot's own output.
//!
//! The GDExtension interface only exposes error/warning printing directly; ordinary `print` is a
//! *utility function*, looked up by name and hash like a bound method. Going through it is what
//! makes messages appear as plain output rather than as engine warnings.
//!
//! The generated bindings expose all of Godot's utility functions, `global::print` among them,
//! so these four look like duplicates. They are not reachable from here, though: `godot-bindings`
//! depends on `godot-core`, so `godot-core` cannot depend back on it without a cycle. These stay
//! as the facade that `godot-core` itself -- and anything that only needs printing -- can use.

use crate::builtin::ToGodot;
use crate::ptrcall::{PtrcallArg, UtilityBind};
use godot_sys as sys;
use std::sync::OnceLock;

macro_rules! define_printer {
    ($fn_name:ident, $godot_name:literal, $hash_const:ident, $doc:literal) => {
        #[doc = $doc]
        ///
        /// # Panics
        /// If called before the extension has been initialized, which cannot happen from within
        /// a registered class.
        pub fn $fn_name(msg: &str) {
            static FUNC: OnceLock<UtilityBind> = OnceLock::new();
            // SAFETY: resolution and the call both require only that the extension is
            // initialized, which `interface_fn!` checks. These are variadic printers, which take
            // Variant arguments and return nothing -- hence `()`, whose return slot is null.
            unsafe {
                let func = FUNC.get_or_init(|| {
                    UtilityBind::resolve($godot_name, sys::method_hashes::$hash_const)
                });
                let arg = msg.to_variant();
                func.call::<()>(&[PtrcallArg::arg_ptr(&arg)]);
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
