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
    base: Base<classes::Node>,
}

#[godot_api(base = Node)]
impl Counter {
    fn init() -> Self {
        Self {
            value: 0,
            step: 1,
            base: Base::unset(),
        }
    }

    fn on_base_ready(&mut self, base: sys::GDExtensionObjectPtr) {
        // SAFETY: the engine passes the object this instance was just attached to.
        self.base = unsafe { Base::new(base) };
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
        // An id rather than the object: the instance may be freed while the future is suspended,
        // and a pointer to a freed object cannot be told from a live one -- the address may even
        // have been reused. Resolving the id asks the engine, which knows the object is gone.
        let id = self.base.instance_id();
        let step = self.step;

        AsyncRuntime::spawn(async move {
            for _ in 0..frame_count {
                next_frame().await;

                let Some(obj) = Gd::<classes::Object>::from_instance_id(id) else {
                    return;
                };
                let _ = obj.call(&StringName::new("_advance_by"), &[step.to_variant()]);
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
        let _ = self.base.emit_signal(
            &StringName::new("value_changed"),
            &[self.value.to_variant()],
        );
    }
}

godot_entry!(counter_init, CounterLibrary);
