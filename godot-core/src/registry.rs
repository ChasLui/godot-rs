use crate::builtin::{GString, StringName};
use crate::property_flags::PROPERTY_USAGE_DEFAULT;
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

    /// Every virtual this class overrides, spelled the way the engine spells it (`_ready`).
    ///
    /// Only used to diagnose names the base class does not have; dispatch itself goes through
    /// [`Self::virtual_trampoline`].
    const VIRTUAL_NAMES: &'static [&'static str] = &[];

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

    /// Called instead of nothing when the engine rebuilds this instance during a hot reload.
    ///
    /// The object is the same one; only the Rust state is new. Distinguishing "rebuilt" from
    /// "state happened to be reset" needs a hook the reload path alone can reach.
    fn on_recreated(&mut self) {}

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

    /// Godot's `_get_property_list`: declares the dynamic properties.
    ///
    /// Without this, [`Self::godot_get`] and [`Self::godot_set`] still work from code, but the
    /// properties are invisible to the editor and to reflection.
    fn godot_get_property_list(&mut self) -> Vec<PropertyDesc> {
        Vec::new()
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
    notify_postinitialize: sys::GDExtensionBool,
) -> sys::GDExtensionObjectPtr {
    let userdata = &*(class_userdata as *const ClassUserdata);

    // `classdb_construct_object3`, not `2`: the engine treats this callback as
    // `create_instance3`, whose contract is that it returns a reference-counted object with a
    // refcount of 1 that the caller already owns. Version 2 constructs *without* claiming the
    // refcount, so the reference the constructor establishes is never accounted for and the
    // object outlives its last user -- reachable only through a RefCounted base, which is why
    // every Node-derived class was fine.
    let base_name = StringName::new(T::BASE_NAME);
    let object = sys::interface_fn!(classdb_construct_object3)(base_name.as_ptr());

    let instance = match crate::panics::catch(
        || format!("{}::init", T::CLASS_NAME),
        None,
        || Some(Box::into_raw(Box::new(T::init()))),
    ) {
        Some(ptr) => ptr,
        // Without Rust state the object would fault on its first call; better an object the
        // engine reports as missing than one that crashes later.
        None => return std::ptr::null_mut(),
    };

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

    // `classdb_construct_object3` builds the object *without* post-initialising it, and the
    // engine uses this argument to say whether finishing the job is ours. Sending it after
    // `object_set_instance` means the class sees its own NOTIFICATION_POSTINITIALIZE, the same
    // one a GDScript class receives.
    if notify_postinitialize != 0 {
        let reversed = false;
        let args: [sys::GDExtensionConstTypePtr; 2] = [
            &crate::obj::NOTIFICATION_POSTINITIALIZE as *const i32 as sys::GDExtensionConstTypePtr,
            &reversed as *const bool as sys::GDExtensionConstTypePtr,
        ];
        crate::obj::object_notification().ptrcall_void(object, &args);
    }

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
    // Lets a test tell a rebuilt instance from one that merely kept its state.
    (*instance).on_recreated();
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
    match crate::panics::catch(
        || format!("{}::to_string", T::CLASS_NAME),
        None,
        || this.godot_to_string(),
    ) {
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

/// One entry of a dynamic property list.
pub struct PropertyDesc {
    pub name: String,
    /// A `GDEXTENSION_VARIANT_TYPE_*` value; see `godot_sys`.
    pub variant_type: sys::GDExtensionVariantType,
    pub hint: u32,
    pub hint_string: String,
    pub usage: u32,
}

impl PropertyDesc {
    /// A plainly stored and editable property of the given type.
    pub fn new(name: &str, variant_type: sys::GDExtensionVariantType) -> Self {
        Self {
            name: name.to_string(),
            variant_type,
            hint: 0,
            hint_string: String::new(),
            usage: PROPERTY_USAGE_DEFAULT,
        }
    }
}

/// Backing storage for one property list handed to the engine.
///
/// `GDExtensionPropertyInfo` holds bare pointers into StringNames and Strings, so those must
/// outlive the array itself -- until `free_property_list` says the engine is done with it.
struct PropertyListStorage {
    /// Never read back -- held so the array the engine is using stays allocated.
    _infos: Vec<sys::GDExtensionPropertyInfo>,
    _strings: Vec<(StringName, StringName, GString)>,
}

thread_local! {
    /// Live property lists, keyed by the array pointer the engine was given.
    ///
    /// The engine hands back only that pointer, so the strings cannot be reached from it
    /// directly; this map is what connects the two. Single-threaded, like every other engine
    /// callback.
    static PROPERTY_LISTS: std::cell::RefCell<
        std::collections::HashMap<usize, PropertyListStorage>,
    > = std::cell::RefCell::new(std::collections::HashMap::new());
}

unsafe extern "C" fn get_property_list<T: GodotClass>(
    instance: sys::GDExtensionClassInstancePtr,
    count: *mut u32,
) -> *const sys::GDExtensionPropertyInfo {
    if !count.is_null() {
        *count = 0;
    }
    if instance.is_null() {
        return std::ptr::null();
    }

    let this = &mut *(instance as *mut T);
    let descs = crate::panics::catch(
        || format!("{}::get_property_list", T::CLASS_NAME),
        Vec::new(),
        || this.godot_get_property_list(),
    );
    if descs.is_empty() {
        return std::ptr::null();
    }

    let mut strings = Vec::with_capacity(descs.len());
    for desc in &descs {
        strings.push((
            StringName::new(&desc.name),
            StringName::new(""),
            GString::new(&desc.hint_string),
        ));
    }

    let infos: Vec<sys::GDExtensionPropertyInfo> = descs
        .iter()
        .zip(strings.iter_mut())
        .map(
            |(desc, (name, class_name, hint_string))| sys::GDExtensionPropertyInfo {
                type_: desc.variant_type,
                name: name.as_mut_ptr(),
                class_name: class_name.as_mut_ptr(),
                hint: desc.hint,
                hint_string: hint_string.as_mut_ptr(),
                usage: desc.usage,
            },
        )
        .collect();

    let ptr = infos.as_ptr();
    if !count.is_null() {
        *count = infos.len() as u32;
    }

    PROPERTY_LISTS.with(|lists| {
        lists.borrow_mut().insert(
            ptr as usize,
            PropertyListStorage {
                _infos: infos,
                _strings: strings,
            },
        );
    });

    ptr
}

unsafe extern "C" fn free_property_list(
    _instance: sys::GDExtensionClassInstancePtr,
    list: *const sys::GDExtensionPropertyInfo,
    _count: u32,
) {
    if list.is_null() {
        return;
    }
    // Dropping the storage releases both the array and the strings it points into.
    PROPERTY_LISTS.with(|lists| {
        lists.borrow_mut().remove(&(list as usize));
    });
}

/// How many property lists the engine has asked for and not yet released.
///
/// Should return to zero once the engine is done with them; a number that only grows is a leak
/// in `free_property_list`.
pub fn live_property_list_count() -> usize {
    PROPERTY_LISTS.with(|lists| lists.borrow().len())
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
    crate::panics::catch(
        || format!("{}::notification", T::CLASS_NAME),
        (),
        || this.godot_notification(what, reversed != 0),
    );
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

    match crate::panics::catch(
        || format!("{}::get", T::CLASS_NAME),
        None,
        || this.godot_get(&name),
    ) {
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

    crate::panics::catch(
        || format!("{}::set", T::CLASS_NAME),
        false,
        || this.godot_set(&name, &value),
    ) as sys::GDExtensionBool
}

/// Registers `T` with Godot's ClassDB.
///
/// # Safety
/// Must be called from an extension initialization callback, at a level where ClassDB is ready
/// (`InitLevel::Scene` for ordinary node types).
pub unsafe fn register_class<T: GodotClass>() {
    if !base_is_registerable::<T>() {
        return;
    }

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
    info.get_property_list_func = Some(get_property_list::<T>);
    info.free_property_list_func = Some(free_property_list);
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
    registered_classes()
        .lock()
        .expect("registry lock poisoned")
        .insert(T::CLASS_NAME);

    // After registration, so the class itself is in ClassDB and the engine can answer.
    check_virtual_names::<T>();

    T::register_methods();
    T::register_signals();
    T::register_properties();
}

/// Removes `T` from ClassDB. Must happen during deinitialization of the level that registered it.
///
/// # Safety
/// Only valid for a class previously registered with [`register_class`].
pub unsafe fn unregister_class<T: GodotClass>() {
    registered_classes()
        .lock()
        .expect("registry lock poisoned")
        .remove(T::CLASS_NAME);

    let class_name = StringName::new(T::CLASS_NAME);
    sys::interface_fn!(classdb_unregister_extension_class)(sys::library(), class_name.as_ptr());
}

/// Class names this extension has registered, so a class can be told from an engine one.
fn registered_classes() -> &'static std::sync::Mutex<std::collections::HashSet<&'static str>> {
    static REGISTERED: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashSet<&'static str>>,
    > = std::sync::OnceLock::new();
    REGISTERED.get_or_init(Default::default)
}

/// Whether `T` may be registered at all.
///
/// A class may not inherit another class from this extension. `base = X` is only a name, so
/// pointing it at one of our own classes compiles, registers, and runs -- and is undefined
/// behaviour. Each class keeps its Rust state in its own `Box`, but an object has exactly one
/// `object_set_instance`, so the base class's methods reinterpret the derived class's memory as
/// their own. With compatible layouts that silently returns the wrong field; with incompatible
/// ones it is a type confusion that corrupts memory.
///
/// Supporting it would mean layering per-class state behind one instance pointer. Refusing is
/// what the object model actually supports, so it is refused loudly rather than left to
/// misbehave quietly.
///
/// This only catches the base registered *before* the inheritor, which is the order that works
/// at all: registering an inheritor first fails in the engine anyway, since ClassDB does not yet
/// have the base.
fn base_is_registerable<T: GodotClass>() -> bool {
    let registered = registered_classes().lock().expect("registry lock poisoned");
    if registered.contains(T::BASE_NAME) {
        crate::logging::godot_error(&format!(
            "{} cannot inherit {}: a class registered by this extension cannot be a base class. \
             Each class owns its Rust state, and an object has only one, so the base class's \
             methods would read the derived class's fields. Inherit an engine class instead.",
            T::CLASS_NAME,
            T::BASE_NAME,
        ));
        return false;
    }
    true
}

/// Reports `#[godot_virtual]` methods whose names the base class does not have.
///
/// Godot resolves virtuals by name and simply never asks for one it does not recognise, so a
/// misspelling produces a class that registers cleanly, runs, and silently does nothing. That is
/// tolerable for `_ready`; it is not for `_forward_canvas_force_draw_over_viewport`.
///
/// The engine is the authority here rather than a table generated alongside the bindings: a
/// generated table can be wrong, and a wrong table rejects correct code, which is worse than the
/// silence it set out to fix. `class_get_method_list` is what the engine itself consults.
///
/// Note that `class_has_method` cannot be used -- virtuals are not callable methods and it
/// answers `false` for every one of them. Only the method *list* includes them.
unsafe fn check_virtual_names<T: GodotClass>() {
    if T::VIRTUAL_NAMES.is_empty() {
        return;
    }

    let Some(known) = base_virtuals(T::BASE_NAME) else {
        // No list means the engine could not be asked (a base class registered later, say).
        // Staying quiet is right: a diagnostic nobody can act on is worse than none.
        return;
    };

    for name in T::VIRTUAL_NAMES {
        if !known.iter().any(|k| k == name) {
            crate::logging::godot_error(&format!(
                "{}: `{}` is not a virtual method of {}, so Godot will never call it. \
                 Check the spelling against the {} documentation.",
                T::CLASS_NAME,
                name,
                T::BASE_NAME,
                T::BASE_NAME,
            ));
        }
    }
}

/// Every method name the engine lists for `class`, including inherited ones.
unsafe fn base_virtuals(class: &str) -> Option<Vec<String>> {
    use crate::builtin::{FromGodot, ToGodot, Variant, VariantArray};
    use crate::ptrcall::MethodBind;

    static METHOD: std::sync::OnceLock<MethodBind> = std::sync::OnceLock::new();
    let method = METHOD.get_or_init(|| {
        MethodBind::resolve(
            "ClassDB",
            "class_get_method_list",
            sys::method_hashes::CLASSDB_CLASS_GET_METHOD_LIST,
        )
    });

    let singleton_name = StringName::new("ClassDB");
    let singleton = sys::interface_fn!(global_get_singleton)(singleton_name.as_ptr());
    if singleton.is_null() {
        return None;
    }

    // `no_inheritance = false`: a class may legitimately override a virtual it inherits, so the
    // whole chain counts.
    let args = [GString::new(class).to_variant(), false.to_variant()];
    let result = method.varcall(singleton, &args).ok()?;
    let list = VariantArray::try_from_variant(&result)?;

    let key = GString::new("name").to_variant();
    let empty = Variant::nil();
    let mut names = Vec::with_capacity(list.size() as usize);
    for i in 0..list.size() {
        let entry = crate::builtin::Dictionary::try_from_variant(&list.get(i))?;
        if let Some(name) = GString::try_from_variant(&entry.get(&key, &empty)) {
            names.push(name.to_rust_string());
        }
    }
    Some(names)
}
