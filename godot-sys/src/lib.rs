//! Raw FFI bindings to the Godot 4 GDExtension C API.
//!
//! This crate is internal to the `godot` facade and carries no stability guarantees.
//! It is generated against the vendored header in `gdextension/`, dumped from the Godot
//! version recorded in `gdextension/VERSION`.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(clippy::missing_safety_doc)]

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
include!(concat!(env!("OUT_DIR"), "/interface.rs"));
include!(concat!(env!("OUT_DIR"), "/builtin_sizes.rs"));

/// Process-wide GDExtension state, initialized once at the extension entry point.
struct Binding {
    interface: GDExtensionInterface,
    library: GDExtensionClassLibraryPtr,
}

// Godot calls the entry point once, on the main thread, before any other extension code runs.
// Everything after that is read-only, so a plain static is sound as long as `initialize` is
// not called twice -- which `initialize` asserts.
static mut BINDING: Option<Binding> = None;

/// # Safety
/// Must be called exactly once, from the GDExtension entry point, before any other use of
/// this crate.
pub unsafe fn initialize(
    get_proc_address: GDExtensionInterfaceGetProcAddress,
    library: GDExtensionClassLibraryPtr,
) {
    assert!(
        (*std::ptr::addr_of!(BINDING)).is_none(),
        "godot-sys initialized twice"
    );

    let interface = GDExtensionInterface::load(get_proc_address);

    if !interface.missing.is_empty() {
        // Not fatal: an older engine simply lacks newer functions. It becomes fatal only if
        // something actually calls one, which `interface_fn!` reports by name.
        eprintln!(
            "godot-sys: {} of {} interface functions unavailable in this engine build: {:?}",
            interface.missing.len(),
            GDExtensionInterface::FUNCTION_COUNT,
            interface.missing
        );
    }

    BINDING = Some(Binding { interface, library });
}

/// # Safety
/// Only valid after [`initialize`].
#[inline]
pub unsafe fn interface() -> &'static GDExtensionInterface {
    match &*std::ptr::addr_of!(BINDING) {
        Some(b) => &b.interface,
        None => panic!("godot-sys used before initialization"),
    }
}

/// The library pointer Godot handed to the entry point; needed to register classes.
///
/// # Safety
/// Only valid after [`initialize`].
#[inline]
pub unsafe fn library() -> GDExtensionClassLibraryPtr {
    match &*std::ptr::addr_of!(BINDING) {
        Some(b) => b.library,
        None => panic!("godot-sys used before initialization"),
    }
}

/// Whether the extension has been initialized. Useful for teardown paths.
pub fn is_initialized() -> bool {
    unsafe { (*std::ptr::addr_of!(BINDING)).is_some() }
}

/// # Safety
/// Must be called only from the extension's deinitialize callback.
pub unsafe fn deinitialize() {
    BINDING = None;
}

/// Calls a GDExtension interface function by its snake_case name.
///
/// ```ignore
/// interface_fn!(string_new_with_utf8_chars)(dest, ptr);
/// ```
#[macro_export]
macro_rules! interface_fn {
    ($name:ident) => {{
        match $crate::interface().$name {
            Some(f) => f,
            None => panic!(
                "GDExtension function `{}` is not available in this Godot build",
                stringify!($name)
            ),
        }
    }};
}

/// Godot's initialization levels, in the order the engine runs them.
///
/// `initialize` is called once per level in ascending order, `deinitialize` once per level in
/// descending order.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum InitLevel {
    Core,
    Servers,
    Scene,
    Editor,
}

impl InitLevel {
    pub fn from_sys(level: GDExtensionInitializationLevel) -> Option<Self> {
        match level {
            GDExtensionInitializationLevel_GDEXTENSION_INITIALIZATION_CORE => Some(Self::Core),
            GDExtensionInitializationLevel_GDEXTENSION_INITIALIZATION_SERVERS => {
                Some(Self::Servers)
            }
            GDExtensionInitializationLevel_GDEXTENSION_INITIALIZATION_SCENE => Some(Self::Scene),
            GDExtensionInitializationLevel_GDEXTENSION_INITIALIZATION_EDITOR => Some(Self::Editor),
            _ => None,
        }
    }

    pub fn to_sys(self) -> GDExtensionInitializationLevel {
        match self {
            Self::Core => GDExtensionInitializationLevel_GDEXTENSION_INITIALIZATION_CORE,
            Self::Servers => GDExtensionInitializationLevel_GDEXTENSION_INITIALIZATION_SERVERS,
            Self::Scene => GDExtensionInitializationLevel_GDEXTENSION_INITIALIZATION_SCENE,
            Self::Editor => GDExtensionInitializationLevel_GDEXTENSION_INITIALIZATION_EDITOR,
        }
    }
}
