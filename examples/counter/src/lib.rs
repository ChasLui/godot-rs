//! A counter node showing the three things most extensions need: an exported property, a signal
//! GDScript can connect to, and a future driven by the frame loop.

use godot::classes;
use godot::prelude::*;
use godot::sys;

struct CounterLibrary;

impl ExtensionLibrary for CounterLibrary {
    fn on_level_init(level: InitLevel) {
        if level == InitLevel::Scene {
            unsafe {
                register_class::<Counter>();
            }
        }
    }

    fn on_level_deinit(level: InitLevel) {
        if level == InitLevel::Scene {
            unsafe {
                unregister_class::<Counter>();
            }
        }
    }
}

struct Counter {
    value: i64,
    step: i64,
    /// The engine object this instance is attached to; needed to emit signals on itself.
    base: sys::GDExtensionObjectPtr,
}

#[godot_api(base = Node)]
impl Counter {
    fn init() -> Self {
        Self {
            value: 0,
            step: 1,
            base: std::ptr::null_mut(),
        }
    }

    fn on_base_ready(&mut self, base: sys::GDExtensionObjectPtr) {
        self.base = base;
    }

    /// Emitted whenever the value changes.
    #[signal]
    fn value_changed(new_value: i64) {}

    /// How much each `increment` adds. Shows up in the Inspector.
    #[prop(set = set_step)]
    fn get_step(&mut self) -> i64 {
        self.step
    }

    #[func]
    fn set_step(&mut self, step: i64) {
        self.step = step;
    }

    #[func]
    fn get_value(&mut self) -> i64 {
        self.value
    }

    #[func]
    fn increment(&mut self) -> i64 {
        self.value += self.step;
        self.notify_changed();
        self.value
    }

    /// Counts up once per frame for `frames` frames, without blocking the game loop.
    #[func]
    fn count_over_frames(&mut self, frame_count: i64) {
        let base = self.base;
        let step = self.step;

        AsyncRuntime::spawn(async move {
            for _ in 0..frame_count {
                next_frame().await;

                // The instance may have been freed while the future was suspended, so the
                // object is looked up fresh each time rather than captured as a reference.
                let Some(obj) = (unsafe { Gd::<classes::Object>::from_obj_ptr(base) }) else {
                    return;
                };
                let _ = classes::Object::call(
                    &obj,
                    StringName::new("_advance_by"),
                    &[step.to_variant()],
                );
            }
        });
    }

    /// Called from the async task above; also usable directly from GDScript.
    #[func]
    fn _advance_by(&mut self, amount: i64) {
        self.value += amount;
        self.notify_changed();
    }

    #[godot_virtual]
    fn process(&mut self, _delta: f64) {
        // Nothing advances spawned futures on its own; the frame loop is the executor.
        AsyncRuntime::poll_all();
    }
}

impl Counter {
    fn notify_changed(&mut self) {
        let Some(this) = (unsafe { Gd::<classes::Object>::from_obj_ptr(self.base) }) else {
            return;
        };
        let _ = classes::Object::emit_signal(
            &this,
            StringName::new("value_changed"),
            &[self.value.to_variant()],
        );
    }
}

godot_entry!(counter_init, CounterLibrary);
