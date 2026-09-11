//! Signal and property registration for extension classes.

use crate::builtin::{GString, StringName};
use crate::registry::GodotClass;
use godot_sys as sys;

/// Owns the strings a `GDExtensionPropertyInfo` points at.
///
/// Godot dereferences `name`, `class_name` and `hint_string` unconditionally, so none of them
/// may be null for the duration of a registration call.
struct PropertyStrings {
    name: StringName,
    class_name: StringName,
    hint_string: GString,
}

impl PropertyStrings {
    fn new(name: &str) -> Self {
        Self {
            name: StringName::new(name),
            class_name: StringName::new(""),
            hint_string: GString::new(""),
        }
    }

    fn info(&mut self, variant_type: sys::GDExtensionVariantType) -> sys::GDExtensionPropertyInfo {
        sys::GDExtensionPropertyInfo {
            type_: variant_type,
            name: self.name.as_mut_ptr(),
            class_name: self.class_name.as_mut_ptr(),
            hint: PROPERTY_HINT_NONE,
            hint_string: self.hint_string.as_mut_ptr(),
            usage: PROPERTY_USAGE_DEFAULT,
        }
    }
}

/// Declares a signal on `T`.
///
/// Argument types are left untyped (any Variant); naming them is what shows up in the editor's
/// signal list and in `connect` autocompletion.
///
/// # Safety
/// `T` must already be registered.
pub unsafe fn register_signal<T: GodotClass>(name: &str, args: &[SignalArg]) {
    let class_name = StringName::new(T::CLASS_NAME);
    let signal_name = StringName::new(name);

    let mut arg_strings: Vec<PropertyStrings> = args
        .iter()
        .map(|a| {
            let mut strings = PropertyStrings::new(a.name);
            strings.class_name = StringName::new(a.class_name);
            strings
        })
        .collect();

    let mut arg_infos: Vec<sys::GDExtensionPropertyInfo> = arg_strings
        .iter_mut()
        .zip(args)
        .map(|(s, arg)| {
            let mut info = s.info(arg.variant_type);
            // NIL means two different things in a property info: "no value" and "any value".
            // The flag picks the second, and only an argument declared `Variant` wants it.
            info.usage =
                if arg.variant_type == sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_NIL {
                    PROPERTY_USAGE_NIL_IS_VARIANT | PROPERTY_USAGE_DEFAULT
                } else {
                    PROPERTY_USAGE_DEFAULT
                };
            info
        })
        .collect();

    // An empty Vec yields a dangling pointer, which the engine would still read.
    let args_ptr = if arg_infos.is_empty() {
        std::ptr::null()
    } else {
        arg_infos.as_mut_ptr() as *const _
    };

    sys::interface_fn!(classdb_register_extension_class_signal)(
        sys::library(),
        class_name.as_ptr(),
        signal_name.as_ptr(),
        args_ptr,
        arg_infos.len() as sys::GDExtensionInt,
    );
}

/// Declares a property backed by two already-registered methods.
///
/// Godot refers to the accessors by name, so `setter` and `getter` must be `#[func]` methods on
/// the same class, registered before this call.
///
/// # Safety
/// `T` must already be registered, along with both accessor methods.
#[allow(clippy::too_many_arguments)]
pub unsafe fn register_property<T: GodotClass>(
    name: &str,
    variant_type: sys::GDExtensionVariantType,
    property_class: &str,
    hint: u32,
    hint_string: &str,
    usage: u32,
    setter: &str,
    getter: &str,
) {
    let class_name = StringName::new(T::CLASS_NAME);
    let setter_name = StringName::new(setter);
    let getter_name = StringName::new(getter);

    let mut strings = PropertyStrings::new(name);
    // For an object property this names the class the value must be. Empty for everything else,
    // where the Variant type says all there is to say.
    strings.class_name = StringName::new(property_class);
    // The hint is what the inspector draws with: a number with PROPERTY_HINT_RANGE and "0,100"
    // is a slider rather than a spin box, a string with PROPERTY_HINT_FILE is a file picker.
    strings.hint_string = GString::new(hint_string);
    let mut info = strings.info(variant_type);
    info.hint = hint;
    // Usage decides whether the property is saved, shown, or neither -- a runtime-only value
    // wants NONE, a hidden-but-saved one STORAGE. The default is saved and shown.
    info.usage = usage;

    sys::interface_fn!(classdb_register_extension_class_property)(
        sys::library(),
        class_name.as_ptr(),
        &info as *const _,
        setter_name.as_ptr(),
        getter_name.as_ptr(),
    );
}

pub(crate) use crate::property_flags::{
    PROPERTY_HINT_NONE, PROPERTY_USAGE_DEFAULT, PROPERTY_USAGE_NIL_IS_VARIANT,
};

/// One argument of a `#[signal]` declaration.
pub struct SignalArg {
    pub name: &'static str,
    pub variant_type: sys::GDExtensionVariantType,
    /// For an object argument, the class it holds; empty otherwise.
    pub class_name: &'static str,
}
