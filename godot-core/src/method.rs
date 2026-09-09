use crate::builtin::{GString, StringName, Variant};
use crate::registry::GodotClass;
use godot_sys as sys;

/// A method exported to Godot.
///
/// Arguments and return values pass as [`Variant`]; typed signatures are a later layer on top
/// of this, not a different mechanism.
pub struct MethodDecl<T> {
    pub name: &'static str,
    pub func: fn(&mut T, &[Variant]) -> Variant,
    pub arg_count: u32,
}

/// Leaked per-method state handed to Godot as `method_userdata`.
struct MethodUserdata<T> {
    func: fn(&mut T, &[Variant]) -> Variant,
    arg_count: u32,
    /// Kept for the panic message, which the engine's backtrace cannot supply.
    name: &'static str,
}

/// Godot's dynamic ("varcall") entry point for an exported method.
unsafe extern "C" fn method_call<T: GodotClass>(
    method_userdata: *mut std::ffi::c_void,
    instance: sys::GDExtensionClassInstancePtr,
    args: *const sys::GDExtensionConstVariantPtr,
    arg_count: sys::GDExtensionInt,
    r_return: sys::GDExtensionVariantPtr,
    r_error: *mut sys::GDExtensionCallError,
) {
    let userdata = &*(method_userdata as *const MethodUserdata<T>);

    if instance.is_null() {
        (*r_error).error = sys::GDExtensionCallErrorType_GDEXTENSION_CALL_ERROR_INSTANCE_IS_NULL;
        return;
    }

    if arg_count != userdata.arg_count as sys::GDExtensionInt {
        (*r_error).error = if arg_count < userdata.arg_count as sys::GDExtensionInt {
            sys::GDExtensionCallErrorType_GDEXTENSION_CALL_ERROR_TOO_FEW_ARGUMENTS
        } else {
            sys::GDExtensionCallErrorType_GDEXTENSION_CALL_ERROR_TOO_MANY_ARGUMENTS
        };
        (*r_error).expected = userdata.arg_count as i32;
        return;
    }

    // The engine owns the argument Variants for the duration of the call; copy them so the
    // Rust side works with ordinary owned values.
    let mut owned_args = Vec::with_capacity(arg_count as usize);
    for i in 0..arg_count as usize {
        owned_args.push(Variant::from_sys_copy(*args.add(i)));
    }

    let this = &mut *(instance as *mut T);
    let result = crate::panics::catch(
        || format!("{}::{}", T::CLASS_NAME, userdata.name),
        Variant::nil(),
        || (userdata.func)(this, &owned_args),
    );

    result.move_into(r_return);
    (*r_error).error = sys::GDExtensionCallErrorType_GDEXTENSION_CALL_OK;
}

/// Owns the strings a `GDExtensionPropertyInfo` points at.
///
/// Godot dereferences `name`, `class_name` and `hint_string` unconditionally -- a zeroed
/// struct segfaults the engine -- so every one of them must be a live StringName/String for as
/// long as the registration call runs.
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

    /// Builds an "any Variant" property descriptor pointing at this struct's strings.
    fn as_variant_info(&mut self) -> sys::GDExtensionPropertyInfo {
        sys::GDExtensionPropertyInfo {
            // NIL together with NIL_IS_VARIANT means "any type", not "nothing".
            type_: sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_NIL,
            name: self.name.as_mut_ptr(),
            class_name: self.class_name.as_mut_ptr(),
            hint: PROPERTY_HINT_NONE,
            hint_string: self.hint_string.as_mut_ptr(),
            usage: PROPERTY_USAGE_NIL_IS_VARIANT | PROPERTY_USAGE_DEFAULT,
        }
    }
}

/// Registers a method on an already-registered class.
///
/// # Safety
/// `T` must have been registered with `register_class` beforehand, at the same init level.
pub unsafe fn register_method<T: GodotClass>(decl: MethodDecl<T>) {
    let class_name = StringName::new(T::CLASS_NAME);
    let mut method_name = StringName::new(decl.name);

    // Leaked deliberately: Godot may dispatch through this after Rust statics are gone.
    let userdata = Box::into_raw(Box::new(MethodUserdata::<T> {
        func: decl.func,
        arg_count: decl.arg_count,
        name: decl.name,
    }));

    let mut return_strings = PropertyStrings::new("ret");
    let mut return_info = return_strings.as_variant_info();

    // Kept in scope so the pointers inside `arg_infos` stay valid across the call below.
    let mut arg_strings: Vec<PropertyStrings> = (0..decl.arg_count)
        .map(|i| PropertyStrings::new(&format!("arg{i}")))
        .collect();

    let mut arg_infos: Vec<sys::GDExtensionPropertyInfo> = arg_strings
        .iter_mut()
        .map(|s| s.as_variant_info())
        .collect();

    let mut arg_metadata: Vec<sys::GDExtensionClassMethodArgumentMetadata> = vec![
            sys::GDExtensionClassMethodArgumentMetadata_GDEXTENSION_METHOD_ARGUMENT_METADATA_NONE;
            decl.arg_count as usize
        ];

    // An empty Vec yields a dangling pointer, which the engine would still read; send null.
    let (arg_infos_ptr, arg_metadata_ptr) = if decl.arg_count == 0 {
        (std::ptr::null_mut(), std::ptr::null_mut())
    } else {
        (arg_infos.as_mut_ptr(), arg_metadata.as_mut_ptr())
    };

    let mut info: sys::GDExtensionClassMethodInfo = std::mem::zeroed();
    info.name = method_name.as_mut_ptr();
    info.method_userdata = userdata as *mut std::ffi::c_void;
    info.call_func = Some(method_call::<T>);
    // No ptrcall: a fully untyped signature is always dispatched through `call_func`.
    info.ptrcall_func = None;
    // The cast looks redundant on Unix and is required on Windows: the underlying type of a C
    // enum is implementation-defined, and bindgen follows it -- u32 with Clang, i32 with MSVC.
    #[allow(clippy::unnecessary_cast)]
    {
        info.method_flags = sys::GDExtensionClassMethodFlags_GDEXTENSION_METHOD_FLAG_NORMAL as u32;
    }
    info.has_return_value = true as sys::GDExtensionBool;
    info.return_value_info = &mut return_info as *mut _;
    info.return_value_metadata =
        sys::GDExtensionClassMethodArgumentMetadata_GDEXTENSION_METHOD_ARGUMENT_METADATA_NONE;
    info.argument_count = decl.arg_count;
    info.arguments_info = arg_infos_ptr;
    info.arguments_metadata = arg_metadata_ptr;

    sys::interface_fn!(classdb_register_extension_class_method)(
        sys::library(),
        class_name.as_ptr(),
        &info as *const _,
    );
}

// From `global_enums` in extension_api.json; verified against the vendored 4.7.2 dump. These
// few are hard-coded because they are needed before the code generator exists.
const PROPERTY_HINT_NONE: u32 = 0;
const PROPERTY_USAGE_DEFAULT: u32 = 6;
const PROPERTY_USAGE_NIL_IS_VARIANT: u32 = 131072;
