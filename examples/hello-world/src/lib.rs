//! The smallest useful extension: one class, one method, callable from GDScript.

use godot::prelude::*;

struct HelloWorldLibrary;

impl ExtensionLibrary for HelloWorldLibrary {
    fn on_level_init(level: InitLevel) {
        // Node types can only be registered once the scene classes exist.
        if level == InitLevel::Scene {
            unsafe {
                register_class::<HelloWorld>();
            }
        }
    }

    fn on_level_deinit(level: InitLevel) {
        if level == InitLevel::Scene {
            unsafe {
                unregister_class::<HelloWorld>();
            }
        }
    }
}

struct HelloWorld;

#[godot_api(base = Node)]
impl HelloWorld {
    fn init() -> Self {
        Self
    }

    #[func]
    fn greet(&mut self, name: GString) -> GString {
        GString::new(&format!("Hello, {}!", name.to_rust_string()))
    }

    #[godot_virtual]
    fn ready(&mut self) {
        godot_print("HelloWorld is ready");
    }
}

godot_entry!(hello_world_init, HelloWorldLibrary);
