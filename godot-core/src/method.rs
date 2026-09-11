use crate::builtin::{GString, StringName, Variant};
use crate::registry::GodotClass;
use godot_sys as sys;

/// A method exported to Godot.
///
/// Arguments and return values pass as [`Variant`]; typed signatures are a later layer on top
/// of this, not a different mechanism.
pub struct MethodDecl<T> {
    pub name: &'static str,
    pub func: fn(Option<&mut T>, &[Variant]) -> Result<Variant, ArgError>,
    pub args: &'static [MethodArg],
    /// Whether the method is called on the class rather than on an instance.
    ///
    /// A static method receives no instance, so the engine passes null where an instance would
    /// be -- `method_call` must not reject that, and must not build a `&mut T` out of it.
    pub is_static: bool,

    /// The return value, or `None` for a method that returns nothing.
    ///
    /// Declaring `None` matters: a method registered as returning a Variant when it returns
    /// nothing tells GDScript to expect a value, and `var x = obj.set_thing(1)` then looks
    /// reasonable rather than wrong.
    pub ret: Option<MethodArg>,
}

/// An argument the engine sent that the method cannot accept.
///
/// Godot has a call-error channel for this, and using it is what makes a mistyped call an
/// error in GDScript rather than a call that quietly does nothing and answers null.
pub struct ArgError {
    /// Index of the offending argument.
    pub index: i32,
    /// The Variant type the method wanted there.
    pub expected: sys::GDExtensionVariantType,
}

/// One argument of an exported method, as the engine should describe it.
///
/// Declaring names and types is what makes a method readable from GDScript: without them the
/// editor offers `damage(arg0, arg1)` and says nothing about what either one is.
pub struct MethodArg {
    pub name: &'static str,
    pub variant_type: sys::GDExtensionVariantType,
    /// For an object argument, the class it holds; empty otherwise.
    pub class_name: &'static str,
}

/// Leaked per-method state handed to Godot as `method_userdata`.
struct MethodUserdata<T> {
    func: fn(Option<&mut T>, &[Variant]) -> Result<Variant, ArgError>,
    arg_count: u32,
    args: &'static [MethodArg],
    ret: Option<sys::GDExtensionVariantType>,
    is_static: bool,
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

    if instance.is_null() && !userdata.is_static {
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

    // A static method gets no instance, and the signature says so rather than inventing one:
    // a `&mut T` pointing at nothing would be undefined behaviour even where nothing reads it.
    let this = if userdata.is_static {
        None
    } else {
        Some(&mut *(instance as *mut T))
    };
    let result = crate::panics::catch(
        || format!("{}::{}", T::CLASS_NAME, userdata.name),
        Ok(Variant::nil()),
        || (userdata.func)(this, &owned_args),
    );

    match result {
        Ok(value) => {
            value.move_into(r_return);
            (*r_error).error = sys::GDExtensionCallErrorType_GDEXTENSION_CALL_OK;
        }
        Err(bad) => {
            (*r_error).error =
                sys::GDExtensionCallErrorType_GDEXTENSION_CALL_ERROR_INVALID_ARGUMENT;
            (*r_error).argument = bad.index;
            (*r_error).expected = bad.expected as i32;
        }
    }
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

    /// Builds a descriptor of the given type, or an "any Variant" one when the type is NIL.
    fn as_typed_info(
        &mut self,
        variant_type: sys::GDExtensionVariantType,
    ) -> sys::GDExtensionPropertyInfo {
        let mut info = self.as_variant_info();
        if variant_type != sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_NIL {
            info.type_ = variant_type;
            // NIL_IS_VARIANT only belongs on an argument that really is any type.
            info.usage = PROPERTY_USAGE_DEFAULT;
        }
        info
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

/// Godot's typed entry point for an exported method.
///
/// Providing this is not an optimisation. The engine reaches a method either through
/// `call_func` or through `ptrcall_func`, and picks the second whenever the call is resolved at
/// compile time -- which GDScript does for every static method and much else besides. A method
/// registered with `ptrcall_func` left null crashes the engine on that path, since nothing
/// checks it before calling through it.
///
/// Arguments arrive in their native representation rather than as Variants, so each is rebuilt
/// from the type the method declared. That is what the declared argument types are for beyond
/// documentation.
unsafe extern "C" fn method_ptrcall<T: GodotClass>(
    method_userdata: *mut std::ffi::c_void,
    instance: sys::GDExtensionClassInstancePtr,
    args: *const sys::GDExtensionConstTypePtr,
    r_return: sys::GDExtensionTypePtr,
) {
    let userdata = &*(method_userdata as *const MethodUserdata<T>);

    if instance.is_null() && !userdata.is_static {
        return;
    }

    let mut owned_args = Vec::with_capacity(userdata.arg_count as usize);
    for (i, arg) in userdata.args.iter().enumerate() {
        let ptr = *args.add(i);
        owned_args.push(
            if arg.variant_type == sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_NIL {
                // A `Variant` argument is already one; there is nothing to rebuild.
                Variant::from_sys_copy(ptr as sys::GDExtensionConstVariantPtr)
            } else {
                Variant::from_builtin(arg.variant_type, ptr as sys::GDExtensionTypePtr)
            },
        );
    }

    let this = if userdata.is_static {
        None
    } else {
        Some(&mut *(instance as *mut T))
    };

    let result = crate::panics::catch(
        || format!("{}::{}", T::CLASS_NAME, userdata.name),
        Ok(Variant::nil()),
        || (userdata.func)(this, &owned_args),
    );

    // A method declared as returning nothing is given no slot to write into.
    let (Some(ret_type), false) = (userdata.ret, r_return.is_null()) else {
        return;
    };
    let value = result.unwrap_or_else(|_| Variant::nil());
    if ret_type == sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_NIL {
        value.move_into(r_return as sys::GDExtensionVariantPtr);
    } else {
        value.to_builtin(ret_type, r_return);
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
        arg_count: decl.args.len() as u32,
        args: decl.args,
        ret: decl.ret.as_ref().map(|r| r.variant_type),
        is_static: decl.is_static,
        name: decl.name,
    }));

    let mut return_strings = PropertyStrings::new("ret");
    if let Some(ret) = &decl.ret {
        return_strings.class_name = StringName::new(ret.class_name);
    }
    let mut return_info = match &decl.ret {
        Some(ret) => return_strings.as_typed_info(ret.variant_type),
        None => return_strings.as_variant_info(),
    };

    // Kept in scope so the pointers inside `arg_infos` stay valid across the call below.
    let mut arg_strings: Vec<PropertyStrings> = decl
        .args
        .iter()
        .map(|a| {
            let mut strings = PropertyStrings::new(a.name);
            strings.class_name = StringName::new(a.class_name);
            strings
        })
        .collect();

    let mut arg_infos: Vec<sys::GDExtensionPropertyInfo> = arg_strings
        .iter_mut()
        .zip(decl.args)
        .map(|(s, arg)| s.as_typed_info(arg.variant_type))
        .collect();

    let mut arg_metadata: Vec<sys::GDExtensionClassMethodArgumentMetadata> = vec![
            sys::GDExtensionClassMethodArgumentMetadata_GDEXTENSION_METHOD_ARGUMENT_METADATA_NONE;
            decl.args.len()
        ];

    // An empty Vec yields a dangling pointer, which the engine would still read; send null.
    let (arg_infos_ptr, arg_metadata_ptr) = if decl.args.is_empty() {
        (std::ptr::null_mut(), std::ptr::null_mut())
    } else {
        (arg_infos.as_mut_ptr(), arg_metadata.as_mut_ptr())
    };

    let mut info: sys::GDExtensionClassMethodInfo = std::mem::zeroed();
    info.name = method_name.as_mut_ptr();
    info.method_userdata = userdata as *mut std::ffi::c_void;
    info.call_func = Some(method_call::<T>);
    info.ptrcall_func = Some(method_ptrcall::<T>);
    // The cast looks redundant on Unix and is required on Windows: the underlying type of a C
    // enum is implementation-defined, and bindgen follows it -- u32 with Clang, i32 with MSVC.
    #[allow(clippy::unnecessary_cast)]
    {
        info.method_flags = if decl.is_static {
            sys::GDExtensionClassMethodFlags_GDEXTENSION_METHOD_FLAG_STATIC as u32
        } else {
            sys::GDExtensionClassMethodFlags_GDEXTENSION_METHOD_FLAG_NORMAL as u32
        };
    }
    info.has_return_value = decl.ret.is_some() as sys::GDExtensionBool;
    info.return_value_info = &mut return_info as *mut _;
    info.return_value_metadata =
        sys::GDExtensionClassMethodArgumentMetadata_GDEXTENSION_METHOD_ARGUMENT_METADATA_NONE;
    info.argument_count = decl.args.len() as u32;
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
