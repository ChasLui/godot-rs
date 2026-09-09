use crate::builtin::{GString, StringName};
use godot_sys as sys;

/// A Rust type that is registered with Godot's ClassDB as a native class.
///
/// In Godot 4 there is no NativeScript indirection: a registered type *is* an engine class,
/// instantiable from GDScript and the editor like any built-in one.
pub trait GodotClass: Sized + 'static {
    /// Name this class is registered under. Must be unique across the whole engine.
    const CLASS_NAME: &'static str;

    /// Name of the engine class to inherit from, e.g. `"Node"` or `"RefCounted"`.
    const BASE_NAME: &'static str;

    /// When true, the class exists only while the game is running, not in the editor.
    ///
    /// GDExtension classes are editor-visible by default -- unlike GDScript, which needs
    /// `@tool` to opt in. Set this for a class whose logic must not run inside the editor.
    const IS_RUNTIME: bool = false;

    /// Constructs the Rust-side state for a new instance.
    fn init() -> Self;

    /// Declares exported methods. Called once, right after the class itself is registered.
    fn register_methods() {}

    // The hooks below have their own fields in `GDExtensionClassCreationInfo6` and are never
    // requested by name, so they do not go through `virtual_trampoline`.

    /// Godot's `_to_string`. Returning `None` leaves Godot's default representation.
    fn godot_to_string(&mut self) -> Option<crate::builtin::GString> {
        None
    }

    /// Godot's `_notification`.
    ///
    /// `what` is one of the `NOTIFICATION_*` constants; `reversed` is set while the engine walks
    /// the tree in reverse, which it does for exit-style notifications.
    fn godot_notification(&mut self, _what: i32, _reversed: bool) {}

    /// Godot's `_get`: reads a property the class handles dynamically.
    ///
    /// Returning `None` means "not mine", and the engine falls back to the registered
    /// properties.
    fn godot_get(&mut self, _property: &str) -> Option<crate::builtin::Variant> {
        None
    }

    /// Godot's `_set`: writes a property the class handles dynamically.
    ///
    /// Returns whether the write was handled; `false` lets the engine try elsewhere.
    fn godot_set(&mut self, _property: &str, _value: &crate::builtin::Variant) -> bool {
        false
    }

    /// Resolves a virtual method Godot asks for, by its engine name (`"_ready"`, `"_input"`).
    ///
    /// Returning `None` tells the engine the class does not override it, which is what keeps
    /// `_process` from running every frame on a class that does not implement it. The macro
    /// generates this from the methods marked `#[godot_virtual]`.
    fn virtual_trampoline(_name: &str) -> Option<crate::virtuals::VirtualTrampoline> {
        None
    }

    /// Hands the instance a pointer to the engine object it is attached to.
    ///
    /// Called once, immediately after construction and before the object is visible to anyone
    /// else. A class needs this to act on itself -- emitting a signal, for instance, is a call
    /// *on the object*, and the Rust state otherwise has no way to reach it.
    ///
    /// Wrap it with `Gd::from_obj_ptr` to use it; the object outlives the Rust state, so keeping
    /// the pointer is sound as long as it is not used after `free`.
    fn on_base_ready(&mut self, _base: sys::GDExtensionObjectPtr) {}

    /// Declares signals. Called once, right after the class is registered.
    fn register_signals() {}

    /// Declares properties. Called once, after methods are registered, since a property refers
    /// to its setter and getter by method name.
    fn register_properties() {}
}

/// Per-class data handed to Godot as `class_userdata` and returned with every callback.
///
/// It is deliberately leaked: Godot may call `free_instance_func` during engine shutdown,
/// after Rust statics would already have been torn down.
struct ClassUserdata {
    class_name: StringName,
}

/// Instance binding callbacks.
///
/// Godot uses these to associate an engine object with language-side data. For an extension
/// class the binding is our own instance pointer, so creation is a passthrough and there is
/// nothing extra to free -- `free_instance_func` already owns that lifetime.
unsafe extern "C" fn binding_create(
    _token: *mut std::ffi::c_void,
    instance: *mut std::ffi::c_void,
) -> *mut std::ffi::c_void {
    instance
}

unsafe extern "C" fn binding_free(
    _token: *mut std::ffi::c_void,
    _instance: *mut std::ffi::c_void,
    _binding: *mut std::ffi::c_void,
) {
}

unsafe extern "C" fn binding_reference(
    _token: *mut std::ffi::c_void,
    _binding: *mut std::ffi::c_void,
    _reference: sys::GDExtensionBool,
) -> sys::GDExtensionBool {
    // Returning true means "keep the object alive"; refcount handling proper arrives with Gd<T>.
    true as sys::GDExtensionBool
}

static BINDING_CALLBACKS: sys::GDExtensionInstanceBindingCallbacks =
    sys::GDExtensionInstanceBindingCallbacks {
        create_callback: Some(binding_create),
        free_callback: Some(binding_free),
        reference_callback: Some(binding_reference),
    };

/// Godot calls this to construct an instance of `T`.
///
/// The engine object itself is built by `classdb_construct_object2` from the *base* class; the
/// Rust state is then attached to it. Both halves must be wired up before the object escapes,
/// or any virtual dispatch into `T` would find no instance.
unsafe extern "C" fn create_instance<T: GodotClass>(
    class_userdata: *mut std::ffi::c_void,
    _notify_postinitialize: sys::GDExtensionBool,
) -> sys::GDExtensionObjectPtr {
    let userdata = &*(class_userdata as *const ClassUserdata);

    let base_name = StringName::new(T::BASE_NAME);
    let object = sys::interface_fn!(classdb_construct_object2)(base_name.as_ptr());

    let instance = Box::into_raw(Box::new(T::init()));

    // Give the instance its own object before anything can call into it.
    (*instance).on_base_ready(object);

    sys::interface_fn!(object_set_instance)(
        object,
        userdata.class_name.as_ptr(),
        instance as sys::GDExtensionClassInstancePtr,
    );

    sys::interface_fn!(object_set_instance_binding)(
        object,
        sys::library(),
        instance as *mut std::ffi::c_void,
        &BINDING_CALLBACKS as *const _,
    );

    object
}

unsafe extern "C" fn free_instance<T: GodotClass>(
    _class_userdata: *mut std::ffi::c_void,
    instance: sys::GDExtensionClassInstancePtr,
) {
    if instance.is_null() {
        return;
    }
    drop(Box::from_raw(instance as *mut T));
}

/// Godot calls this when an extension is hot-reloaded: the engine object survives, the
/// language-side state is rebuilt.
///
/// Wired up from the start because it belongs to the same ABI struct as create/free; filling in
/// the body later must not require reshaping the object model.
unsafe extern "C" fn recreate_instance<T: GodotClass>(
    _class_userdata: *mut std::ffi::c_void,
    object: sys::GDExtensionObjectPtr,
) -> sys::GDExtensionClassInstancePtr {
    let instance = Box::into_raw(Box::new(T::init()));
    (*instance).on_base_ready(object);
    instance as sys::GDExtensionClassInstancePtr
}

/// Godot's dedicated `to_string` hook, which bypasses the by-name virtual dispatch.
unsafe extern "C" fn to_string<T: GodotClass>(
    instance: sys::GDExtensionClassInstancePtr,
    is_valid: *mut sys::GDExtensionBool,
    out: sys::GDExtensionStringPtr,
) {
    if instance.is_null() {
        if !is_valid.is_null() {
            *is_valid = false as sys::GDExtensionBool;
        }
        return;
    }

    let this = &mut *(instance as *mut T);
    match this.godot_to_string() {
        Some(s) => {
            if !is_valid.is_null() {
                *is_valid = true as sys::GDExtensionBool;
            }
            // The engine's slot is uninitialized; assigning a copy leaves it owning the value.
            std::ptr::write(out as *mut crate::builtin::GString, s);
        }
        None => {
            if !is_valid.is_null() {
                *is_valid = false as sys::GDExtensionBool;
            }
        }
    }
}

unsafe extern "C" fn notification<T: GodotClass>(
    instance: sys::GDExtensionClassInstancePtr,
    what: i32,
    reversed: sys::GDExtensionBool,
) {
    if instance.is_null() {
        return;
    }
    let this = &mut *(instance as *mut T);
    this.godot_notification(what, reversed != 0);
}

unsafe extern "C" fn get_property<T: GodotClass>(
    instance: sys::GDExtensionClassInstancePtr,
    name: sys::GDExtensionConstStringNamePtr,
    ret: sys::GDExtensionVariantPtr,
) -> sys::GDExtensionBool {
    if instance.is_null() {
        return false as sys::GDExtensionBool;
    }
    let this = &mut *(instance as *mut T);
    let name = StringName::from_sys_copy(name).to_rust_string();

    match this.godot_get(&name) {
        Some(value) => {
            // The engine's slot is uninitialized and takes ownership of what is written.
            value.move_into(ret);
            true as sys::GDExtensionBool
        }
        None => false as sys::GDExtensionBool,
    }
}

unsafe extern "C" fn set_property<T: GodotClass>(
    instance: sys::GDExtensionClassInstancePtr,
    name: sys::GDExtensionConstStringNamePtr,
    value: sys::GDExtensionConstVariantPtr,
) -> sys::GDExtensionBool {
    if instance.is_null() {
        return false as sys::GDExtensionBool;
    }
    let this = &mut *(instance as *mut T);
    let name = StringName::from_sys_copy(name).to_rust_string();
    let value = crate::builtin::Variant::from_sys_copy(value);

    this.godot_set(&name, &value) as sys::GDExtensionBool
}

/// Registers `T` with Godot's ClassDB.
///
/// # Safety
/// Must be called from an extension initialization callback, at a level where ClassDB is ready
/// (`InitLevel::Scene` for ordinary node types).
pub unsafe fn register_class<T: GodotClass>() {
    let class_name = StringName::new(T::CLASS_NAME);
    let base_name = StringName::new(T::BASE_NAME);

    // Leaked on purpose: Godot may still call into these callbacks during shutdown.
    let userdata = Box::into_raw(Box::new(ClassUserdata {
        class_name: StringName::new(T::CLASS_NAME),
    }));

    // `icon_path` is a String pointer the engine reads unconditionally; a zeroed struct leaves
    // it null and the engine dereferences it. Same trap as the property-info strings.
    let icon_path = GString::new("");

    let mut info: sys::GDExtensionClassCreationInfo6 = std::mem::zeroed();
    info.icon_path = icon_path.as_ptr();
    info.is_virtual = false as sys::GDExtensionBool;
    info.is_abstract = false as sys::GDExtensionBool;
    info.is_exposed = true as sys::GDExtensionBool;
    info.is_runtime = T::IS_RUNTIME as sys::GDExtensionBool;
    info.to_string_func = Some(to_string::<T>);
    info.notification_func = Some(notification::<T>);
    info.get_func = Some(get_property::<T>);
    info.set_func = Some(set_property::<T>);
    info.create_instance_func = Some(create_instance::<T>);
    info.free_instance_func = Some(free_instance::<T>);
    info.recreate_instance_func = Some(recreate_instance::<T>);
    // Resolved once per class by Godot, then cached -- see `virtuals` for why this pair is used
    // instead of `get_virtual_func`.
    info.get_virtual_call_data_func = Some(crate::virtuals::get_virtual_call_data::<T>);
    info.call_virtual_with_data_func = Some(crate::virtuals::call_virtual_with_data);
    info.class_userdata = userdata as *mut std::ffi::c_void;

    sys::interface_fn!(classdb_register_extension_class6)(
        sys::library(),
        class_name.as_ptr(),
        base_name.as_ptr(),
        &info as *const _,
    );

    // Methods can only be attached once the class exists in ClassDB, and a property refers to
    // its accessors by name, so it has to come after them.
    T::register_methods();
    T::register_signals();
    T::register_properties();
}

/// Removes `T` from ClassDB. Must happen during deinitialization of the level that registered it.
///
/// # Safety
/// Only valid for a class previously registered with [`register_class`].
pub unsafe fn unregister_class<T: GodotClass>() {
    let class_name = StringName::new(T::CLASS_NAME);
    sys::interface_fn!(classdb_unregister_extension_class)(sys::library(), class_name.as_ptr());
}
