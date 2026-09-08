use godot_sys as sys;
pub use godot_sys::InitLevel;

/// The entry point of a GDExtension library.
///
/// Godot calls `initialize` once per initialization level in ascending order
/// (Core, Servers, Scene, Editor) and `deinitialize` once per level in descending order.
/// Classes must be registered at a level where their base class already exists -- for ordinary
/// node types that is [`InitLevel::Scene`].
///
/// **A level is a startup phase, not a mode.** Godot runs all four levels in a game run just as
/// it does in the editor, so [`InitLevel::Editor`] does *not* mean "only in the editor". To
/// register something editor-only, check `Engine::is_editor_hint()` inside that level.
pub trait ExtensionLibrary {
    /// Lowest level at which the engine should start calling this extension.
    fn min_level() -> InitLevel {
        InitLevel::Scene
    }

    /// Called once per level, ascending.
    fn on_level_init(_level: InitLevel) {}

    /// Called once per level, descending.
    fn on_level_deinit(_level: InitLevel) {}
}

unsafe extern "C" fn initialize_level<E: ExtensionLibrary>(
    _userdata: *mut std::ffi::c_void,
    level: sys::GDExtensionInitializationLevel,
) {
    if let Some(level) = InitLevel::from_sys(level) {
        E::on_level_init(level);
    }
}

unsafe extern "C" fn deinitialize_level<E: ExtensionLibrary>(
    _userdata: *mut std::ffi::c_void,
    level: sys::GDExtensionInitializationLevel,
) {
    if let Some(level) = InitLevel::from_sys(level) {
        E::on_level_deinit(level);
    }
}

/// Body of the `#[no_mangle]` entry function that Godot looks up via `entry_symbol`.
///
/// # Safety
/// Must only be called by Godot, with the arguments it passes to the entry point.
pub unsafe fn entry_point<E: ExtensionLibrary>(
    get_proc_address: sys::GDExtensionInterfaceGetProcAddress,
    library: sys::GDExtensionClassLibraryPtr,
    initialization: *mut sys::GDExtensionInitialization,
) -> sys::GDExtensionBool {
    if initialization.is_null() {
        return false as sys::GDExtensionBool;
    }

    sys::initialize(get_proc_address, library);

    (*initialization).minimum_initialization_level = E::min_level().to_sys();
    (*initialization).userdata = std::ptr::null_mut();
    (*initialization).initialize = Some(initialize_level::<E>);
    (*initialization).deinitialize = Some(deinitialize_level::<E>);

    true as sys::GDExtensionBool
}

/// Declares the `#[no_mangle]` entry function Godot looks up via `entry_symbol`.
///
/// The name given here must match `configuration/entry_symbol` in the `.gdextension` file.
///
/// ```ignore
/// struct MyLib;
/// impl ExtensionLibrary for MyLib { /* ... */ }
/// godot_entry!(my_extension_init, MyLib);
/// ```
#[macro_export]
macro_rules! godot_entry {
    ($entry_name:ident, $library:ty) => {
        #[no_mangle]
        pub unsafe extern "C" fn $entry_name(
            get_proc_address: $crate::sys::GDExtensionInterfaceGetProcAddress,
            library: $crate::sys::GDExtensionClassLibraryPtr,
            initialization: *mut $crate::sys::GDExtensionInitialization,
        ) -> $crate::sys::GDExtensionBool {
            $crate::init::entry_point::<$library>(get_proc_address, library, initialization)
        }
    };
}
