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
pub unsafe fn register_signal<T: GodotClass>(name: &str, arg_names: &[&str]) {
    let class_name = StringName::new(T::CLASS_NAME);
    let signal_name = StringName::new(name);

    let mut arg_strings: Vec<PropertyStrings> =
        arg_names.iter().map(|n| PropertyStrings::new(n)).collect();

    let mut arg_infos: Vec<sys::GDExtensionPropertyInfo> = arg_strings
        .iter_mut()
        .map(|s| {
            let mut info = s.info(sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_NIL);
            info.usage = PROPERTY_USAGE_NIL_IS_VARIANT | PROPERTY_USAGE_DEFAULT;
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
pub unsafe fn register_property<T: GodotClass>(
    name: &str,
    variant_type: sys::GDExtensionVariantType,
    property_class: &str,
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
    let info = strings.info(variant_type);

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
