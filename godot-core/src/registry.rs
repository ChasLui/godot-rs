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

    /// Godot names of the virtual methods this class overrides, e.g. `&["_ready", "_process"]`.
    ///
    /// Must be listed explicitly: Rust cannot tell whether a defaulted trait method was
    /// overridden, and Godot needs to know so it can skip classes that do not implement a hook
    /// -- `_process` in particular would otherwise run every frame for nothing.
    const OVERRIDDEN_VIRTUALS: &'static [&'static str] = &[];

    /// Called when the node enters the scene tree and all its children are ready.
    fn ready(&mut self) {}

    /// Called every frame; `delta` is the elapsed time in seconds.
    fn process(&mut self, _delta: f64) {}

    /// Called every physics tick; `delta` is the fixed step in seconds.
    fn physics_process(&mut self, _delta: f64) {}

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
    info.create_instance_func = Some(create_instance::<T>);
    info.free_instance_func = Some(free_instance::<T>);
    info.recreate_instance_func = Some(recreate_instance::<T>);
    // Resolved once per class by Godot, then cached -- see `virtuals` for why this pair is used
    // instead of `get_virtual_func`.
    info.get_virtual_call_data_func = Some(crate::virtuals::get_virtual_call_data::<T>);
    info.call_virtual_with_data_func = Some(crate::virtuals::call_virtual_with_data::<T>);
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
