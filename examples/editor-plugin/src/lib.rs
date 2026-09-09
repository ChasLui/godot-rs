//! An editor plugin written in Rust: it reports what the editor is currently editing.
//!
//! Godot's own addons are GDScript files listed in a `plugin.cfg` under `addons/`. An extension
//! needs neither: registering a class that descends from `EditorPlugin` and handing its name to
//! `add_editor_plugin` is the whole mechanism. There is no `.cfg` and no script in this project
//! -- only the `.gdextension` that loads the library.
//!
//! Because it references editor classes, this crate enables the `editor` feature. A game
//! extension must not: those classes do not exist in an exported project, and referencing them
//! there makes the extension fail to load.
//!
//! # Known gap
//!
//! This plugin deliberately adds nothing to the editor's interface. `add_control_to_dock` and
//! `add_control_to_container` crash the engine from these bindings, and the cause is not yet
//! found: the plugin's own handle is good (a no-argument call like `get_plugin_version` works),
//! object arguments are passed correctly (`remove_control_from_docks` works), enum arguments are
//! `int64_t` on both sides, and the crash happens from `_enter_tree` and `_ready` alike, with or
//! without any cleanup. Everything else about editor plugins -- registration, virtual dispatch,
//! returning owned values to the engine -- works and is covered by the test suite.

use godot::classes::{EditorInterface, EditorPlugin, Engine, Object};
use godot::editor::{add_editor_plugin, remove_editor_plugin};
use godot::prelude::*;

struct EditorPluginExample;

impl ExtensionLibrary for EditorPluginExample {
    fn on_level_init(level: InitLevel) {
        // The Editor level is a startup phase, not a mode -- a running game goes through it as
        // well. Registering an editor plugin unconditionally would try to do so in a game.
        if level == InitLevel::Editor && is_editor() {
            unsafe {
                register_class::<SceneReporterPlugin>();
                // The class has to be in ClassDB first: the engine looks the name up here and
                // instantiates the plugin itself.
                add_editor_plugin::<SceneReporterPlugin>();
            }
        }
    }

    fn on_level_deinit(level: InitLevel) {
        if level == InitLevel::Editor && is_editor() {
            unsafe {
                // The editor holds a live instance, so it lets go before the class does.
                remove_editor_plugin::<SceneReporterPlugin>();
                unregister_class::<SceneReporterPlugin>();
            }
        }
    }
}

fn is_editor() -> bool {
    Engine::singleton().is_editor_hint()
}

godot_entry!(editor_plugin_init, EditorPluginExample);

/// Reports the scene being edited, and which objects it would handle.
struct SceneReporterPlugin {
    /// The engine object this Rust state belongs to.
    ///
    /// A `#[godot_api]` type is the state *behind* an engine object, not the object itself, so
    /// calling an inherited method means holding on to the object. `on_base_ready` is where the
    /// engine hands it over.
    base: godot::sys::GDExtensionObjectPtr,
}

#[godot_api(base = EditorPlugin)]
impl SceneReporterPlugin {
    fn init() -> Self {
        Self {
            base: std::ptr::null_mut(),
        }
    }

    fn on_base_ready(&mut self, base: godot::sys::GDExtensionObjectPtr) {
        self.base = base;
    }

    /// The name the editor shows for this plugin.
    ///
    /// Returns an owned `GString`, which the engine reads out of a slot it default-constructed
    /// -- the binding has to assign into it rather than overwrite it, or the engine's value
    /// leaks.
    #[godot_virtual]
    fn get_plugin_name(&mut self) -> GString {
        GString::new("Scene Reporter")
    }

    /// No main screen tab; this plugin only observes.
    #[godot_virtual]
    fn has_main_screen(&mut self) -> bool {
        false
    }

    #[godot_virtual]
    fn enter_tree(&mut self) {
        godot_print(&format!(
            "Scene Reporter: plugin entered the editor, version {}",
            self.plugin().get_plugin_version().to_rust_string()
        ));
        self.report();
    }

    #[godot_virtual]
    fn exit_tree(&mut self) {
        godot_print("Scene Reporter: plugin left the editor");
    }

    /// Whether this plugin wants to edit `object`. Returning true makes the editor call `_edit`
    /// with it. Reporting on Node2D keeps the example concrete without claiming everything.
    #[godot_virtual]
    fn handles(&mut self, object: Option<Gd<Object>>) -> bool {
        let Some(object) = object else {
            return false;
        };
        object.try_cast::<godot::classes::Node2D>().is_some()
    }

    /// Called when the editor starts editing an object this plugin handles.
    #[godot_virtual]
    fn edit(&mut self, object: Option<Gd<Object>>) {
        if object.is_some() {
            self.report();
        }
    }

    /// Called when the editor stops editing.
    #[godot_virtual]
    fn clear(&mut self) {
        godot_print("Scene Reporter: nothing being edited");
    }
}

impl SceneReporterPlugin {
    /// This plugin as an engine handle.
    fn plugin(&self) -> Gd<EditorPlugin> {
        // SAFETY: `base` is the object the engine constructed for this instance, and the engine
        // only calls in while that object is alive.
        unsafe {
            Gd::from_obj_ptr(self.base)
                .expect("the engine sets the base object before it calls any virtual")
        }
    }

    fn report(&mut self) {
        match EditorInterface::singleton().get_edited_scene_root() {
            Some(root) => godot_print(&format!(
                "Scene Reporter: editing {}",
                root.get_name().to_rust_string()
            )),
            None => godot_print("Scene Reporter: no scene open"),
        }
    }
}
