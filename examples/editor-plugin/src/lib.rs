//! An editor plugin written in Rust: a dock panel naming the scene being edited.
//!
//! Godot's own addons are GDScript files listed in a `plugin.cfg` under `addons/`. An extension
//! needs neither: registering a class that descends from `EditorPlugin` and handing its name to
//! `add_editor_plugin` is the whole mechanism. There is no `.cfg` and no script in this project
//! -- only the `.gdextension` that loads the library.
//!
//! Because it references editor classes, this crate enables the `editor` feature. A game
//! extension must not: those classes do not exist in an exported project, and referencing them
//! there makes the extension fail to load.

use godot::classes::{
    Control, EditorInterface, EditorPlugin, EditorPluginDockSlot, Engine, Label, Object,
};
use godot::editor::{add_editor_plugin, remove_editor_plugin};
use godot::prelude::*;

struct EditorPluginExample;

impl ExtensionLibrary for EditorPluginExample {
    fn on_level_init(level: InitLevel) {
        // The Editor level is a startup phase, not a mode -- a running game goes through it as
        // well. Registering an editor plugin unconditionally would try to do so in a game.
        if level == InitLevel::Editor && is_editor() {
            unsafe {
                register_class::<SceneNamePlugin>();
                // The class has to be in ClassDB first: the engine looks the name up here and
                // instantiates the plugin itself.
                add_editor_plugin::<SceneNamePlugin>();
            }
        }
    }

    fn on_level_deinit(level: InitLevel) {
        if level == InitLevel::Editor && is_editor() {
            unsafe {
                // The editor holds a live instance, so it lets go before the class does.
                remove_editor_plugin::<SceneNamePlugin>();
                unregister_class::<SceneNamePlugin>();
            }
        }
    }
}

fn is_editor() -> bool {
    Engine::singleton().is_editor_hint()
}

godot_entry!(editor_plugin_init, EditorPluginExample);

/// Godot's `EditorPlugin.DOCK_SLOT_LEFT_UL`.
const DOCK_SLOT_LEFT_UL: i64 = 0;

/// Adds a dock naming the scene being edited, and keeps it current.
struct SceneNamePlugin {
    /// The engine object this Rust state belongs to.
    ///
    /// A `#[godot_api]` type is the state *behind* an engine object, not the object itself, so
    /// calling an inherited method means holding on to the object. `on_base_ready` is where the
    /// engine hands it over.
    base: godot::sys::GDExtensionObjectPtr,
    label: Option<Gd<Label>>,
}

#[godot_api(base = EditorPlugin)]
impl SceneNamePlugin {
    fn init() -> Self {
        Self {
            base: std::ptr::null_mut(),
            label: None,
        }
    }

    fn on_base_ready(&mut self, base: godot::sys::GDExtensionObjectPtr) {
        self.base = base;
    }

    /// The name on the dock's tab.
    ///
    /// Returns an owned `GString`. The engine reads it out of a slot it default-constructed, so
    /// the binding assigns into that slot rather than overwriting it.
    #[godot_virtual]
    fn get_plugin_name(&mut self) -> GString {
        GString::new("Scene Name")
    }

    #[godot_virtual]
    fn has_main_screen(&mut self) -> bool {
        false
    }

    #[godot_virtual]
    fn enter_tree(&mut self) {
        let label = Gd::<Label>::new().expect("Label is a registered engine class");
        label.set_name(&StringName::new("Scene Name"));
        label.set_text(&GString::new("(no scene open)"));

        // `shortcut` defaults to null in Godot, so the short form leaves it out entirely;
        // `add_control_to_dock_ex` takes it as an `Option`.
        self.plugin()
            .add_control_to_dock(EditorPluginDockSlot(DOCK_SLOT_LEFT_UL), label.upcast_ref());

        self.label = Some(label);
        self.refresh();
    }

    #[godot_virtual]
    fn exit_tree(&mut self) {
        // Godot does not free a docked control on its own; the plugin that added it takes it
        // back out and frees it.
        if let Some(label) = self.label.take() {
            self.plugin().remove_control_from_docks(label.upcast_ref());
            // SAFETY: the dock no longer holds it, and this is the only handle left.
            unsafe { label.free() };
        }
    }

    /// Whether this plugin wants to edit `object`. Returning true makes the editor call `_edit`.
    #[godot_virtual]
    fn handles(&mut self, object: Option<Gd<Object>>) -> bool {
        object.is_some()
    }

    /// Called when the editor starts editing something this plugin handles.
    #[godot_virtual]
    fn edit(&mut self, _object: Option<Gd<Object>>) {
        self.refresh();
    }

    #[godot_virtual]
    fn clear(&mut self) {
        self.refresh();
    }
}

impl SceneNamePlugin {
    /// This plugin as an engine handle.
    fn plugin(&self) -> Gd<EditorPlugin> {
        // SAFETY: `base` is the object the engine constructed for this instance, and the engine
        // only calls in while that object is alive.
        unsafe {
            Gd::from_obj_ptr(self.base)
                .expect("the engine sets the base object before it calls any virtual")
        }
    }

    fn refresh(&mut self) {
        let Some(label) = self.label.as_mut() else {
            return;
        };

        let text = match EditorInterface::singleton().get_edited_scene_root() {
            Some(root) => root.get_name().to_rust_string(),
            None => "(no scene open)".to_string(),
        };
        label.set_text(&GString::new(&text));
    }
}

/// Keeps the `Control` import honest: the dock takes one, via `upcast_ref`.
const _: fn(&Gd<Label>) -> &Gd<Control> = |l| l.upcast_ref();
