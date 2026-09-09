//! Integration tests, loaded by Godot as a real GDExtension.
//!
//! Assertions live on the GDScript side (`itest/godot/`), which decides the process exit code.
//! Round-tripping through GDScript is what makes these tests meaningful: it exercises the same
//! path a real user's code takes.

use godot::builtin::{
    Color, Dictionary, NodePath, PackedByteArray, PackedStringArray, Transform2D, VariantArray,
    Vector3,
};
use godot::classes;
use godot::prelude::*;
use godot::sys;

thread_local! {
    /// Progress of the spawned test future. Thread-local because the runtime is single-threaded,
    /// matching Godot's own calling convention.
    static ASYNC_RESULT: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
}

struct ItestLibrary;

impl ExtensionLibrary for ItestLibrary {
    fn min_level() -> InitLevel {
        InitLevel::Scene
    }

    fn on_level_init(level: InitLevel) {
        match level {
            InitLevel::Scene => unsafe {
                register_class::<RustTestNode>();
                register_class::<RustRuntimeOnlyNode>();
            },
            // Levels are startup phases, not modes: Godot runs the Editor level during a game
            // run too. Registering something editor-only therefore needs an explicit check.
            InitLevel::Editor if is_editor() => unsafe {
                register_class::<RustEditorOnlyNode>();
            },
            _ => {}
        }
    }

    fn on_level_deinit(level: InitLevel) {
        match level {
            InitLevel::Scene => unsafe {
                unregister_class::<RustTestNode>();
                unregister_class::<RustRuntimeOnlyNode>();
            },
            InitLevel::Editor if is_editor() => unsafe {
                unregister_class::<RustEditorOnlyNode>();
            },
            _ => {}
        }
    }
}

/// Whether Godot is running as an editor rather than playing the game.
fn is_editor() -> bool {
    let engine = classes::Engine::singleton();
    classes::Engine::is_editor_hint(&engine)
}

/// Registered only when running inside the editor, to prove the gate actually gates.
struct RustEditorOnlyNode;

#[godot_api(base = Node)]
impl RustEditorOnlyNode {
    fn init() -> Self {
        Self
    }

    #[func]
    fn marker(&mut self) -> i64 {
        1
    }
}

/// Declared `runtime`, so Godot keeps it out of the editor.
struct RustRuntimeOnlyNode;

#[godot_api(base = Node, runtime)]
impl RustRuntimeOnlyNode {
    fn init() -> Self {
        Self
    }

    #[func]
    fn marker(&mut self) -> i64 {
        2
    }
}

struct RustTestNode {
    counter: i64,
    speed: f64,
    label: GString,
    base: sys::GDExtensionObjectPtr,
    ready_calls: i64,
    process_calls: i64,
    physics_calls: i64,
    accumulated_delta: f64,
}

#[godot_api(base = Node)]
impl RustTestNode {
    fn init() -> Self {
        Self {
            counter: 0,
            speed: 0.0,
            label: GString::new("unset"),
            base: std::ptr::null_mut(),
            ready_calls: 0,
            process_calls: 0,
            physics_calls: 0,
            accumulated_delta: 0.0,
        }
    }

    // -- Signals and properties ---------------------------------------------------------

    /// Declared, not implemented: the body is ignored, only name and argument names register.
    #[signal]
    fn counter_changed(new_value: i64) {}

    /// A float property, backed by the accessor pair below.
    #[prop(set = set_speed)]
    fn get_speed(&mut self) -> f64 {
        self.speed
    }

    #[func]
    fn set_speed(&mut self, value: f64) {
        self.speed = value;
    }

    /// A string property, to check a second Variant type through the same path.
    #[prop(set = set_label)]
    fn get_label(&mut self) -> GString {
        GString::new(&self.label.to_rust_string())
    }

    #[func]
    fn set_label(&mut self, value: GString) {
        self.label = value;
    }

    /// Bumps the counter and announces it, proving a Rust class can emit on itself.
    ///
    /// Returns the new counter, or a negative code if emitting failed, so a broken signal path
    /// surfaces as a value rather than as silence.
    #[func]
    fn bump_and_emit(&mut self) -> i64 {
        self.counter += 1;

        // SAFETY: `base` is the engine object this instance is attached to, alive for as long
        // as the instance is.
        let Some(this) = (unsafe { Gd::<classes::Object>::from_obj_ptr(self.base) }) else {
            return -1;
        };

        match classes::Object::emit_signal(
            &this,
            StringName::new("counter_changed"),
            &[self.counter.to_variant()],
        ) {
            Ok(result) => match i64::try_from_variant(&result) {
                // emit_signal returns a Godot Error code; 0 is OK.
                Some(0) => self.counter,
                Some(code) => -100 - code,
                None => -2,
            },
            Err(call_error) => -200 - call_error as i64,
        }
    }

    // -- Engine hooks -------------------------------------------------------------------

    fn on_base_ready(&mut self, base: sys::GDExtensionObjectPtr) {
        self.base = base;
    }

    #[godot_virtual]
    fn ready(&mut self) {
        self.ready_calls += 1;
    }

    #[godot_virtual]
    fn process(&mut self, delta: f64) {
        self.process_calls += 1;
        self.accumulated_delta += delta;

        // Nothing advances futures on its own; the frame loop is the executor.
        AsyncRuntime::poll_all();
    }

    #[godot_virtual]
    fn physics_process(&mut self, _delta: f64) {
        self.physics_calls += 1;
    }

    // -- Variant round trips ------------------------------------------------------------

    /// Each `echo_*` proves a full round trip: GDScript value -> Variant -> Rust type ->
    /// Variant -> GDScript. A layout or ownership mistake shows up as a wrong value.
    #[func]
    fn echo_int(&mut self, value: i64) -> i64 {
        value
    }

    #[func]
    fn echo_float(&mut self, value: f64) -> f64 {
        value
    }

    #[func]
    fn echo_bool(&mut self, value: bool) -> bool {
        value
    }

    #[func]
    fn echo_string(&mut self, value: GString) -> GString {
        // Go through Rust's own String to prove the UTF-8 conversion both ways.
        GString::new(&value.to_rust_string())
    }

    #[func]
    fn add_one(&mut self, value: i64) -> i64 {
        value + 1
    }

    /// Proves per-instance state actually lives across calls.
    #[func]
    fn bump(&mut self) -> i64 {
        self.counter += 1;
        self.counter
    }

    /// Spawns a future that counts frames, so GDScript can watch it progress across real frames.
    #[func]
    fn spawn_frame_counter(&mut self, frame_count: i64) -> i64 {
        ASYNC_RESULT.with(|r| r.set(0));

        AsyncRuntime::spawn(async move {
            for i in 1..=frame_count {
                next_frame().await;
                ASYNC_RESULT.with(|r| r.set(i));
            }
            // A negative marker distinguishes "finished" from "reached the last step".
            ASYNC_RESULT.with(|r| r.set(-frame_count));
        });

        AsyncRuntime::pending_count() as i64
    }

    /// The value the spawned future has reached so far.
    #[func]
    fn async_progress(&mut self) -> i64 {
        ASYNC_RESULT.with(|r| r.get())
    }

    /// Reports how often each virtual hook fired, plus whether the accumulated `delta` looks
    /// like real frame time rather than garbage.
    #[func]
    fn virtual_counts(&mut self) -> GString {
        let delta_sane = self.accumulated_delta > 0.0 && self.accumulated_delta < 3600.0;
        GString::new(&format!(
            "{},{},{},{}",
            self.ready_calls, self.process_calls, self.physics_calls, delta_sane
        ))
    }

    // -- Engine API through the generated bindings --------------------------------------

    /// GDScript compares this against its own `OS.get_name()`, so a wrong ptrcall shows up as a
    /// mismatch rather than merely "returned something".
    #[func]
    fn engine_os_name(&mut self) -> GString {
        let os = classes::OS::singleton();
        classes::OS::get_name(&os)
    }

    /// A bool-returning engine call, exercising a different ptrcall return width.
    #[func]
    fn engine_is_editor_hint(&mut self) -> bool {
        let engine = classes::Engine::singleton();
        classes::Engine::is_editor_hint(&engine)
    }

    /// Round-trips through a real engine object: set a Node's name, read it back.
    #[func]
    fn node_name_roundtrip(&mut self, name: StringName) -> StringName {
        let Some(node) = Gd::<classes::Node>::new() else {
            return StringName::new("");
        };

        classes::Node::set_name(&node, name);
        let read_back = classes::Node::get_name(&node);

        // Node is manually managed, so it must be freed explicitly.
        unsafe { node.free() };

        read_back
    }

    // -- Math builtins ------------------------------------------------------------------

    /// Round-trips a Transform2D through Variant. A wrong field order or element size shows up
    /// as scrambled numbers rather than a crash, which is why the values are all distinct.
    #[func]
    fn echo_transform2d(&mut self, t: Variant) -> Variant {
        match Transform2D::try_from_variant(&t) {
            Some(v) => v.to_variant(),
            None => Variant::nil(),
        }
    }

    #[func]
    fn echo_vector3(&mut self, v: Vector3) -> Vector3 {
        v
    }

    #[func]
    fn echo_color(&mut self, c: Color) -> Color {
        c
    }

    /// Exercises the Rust-side math rather than just the marshalling.
    #[func]
    fn vector3_cross_length(&mut self, a: Vector3, b: Vector3) -> f64 {
        a.cross(b).length() as f64
    }

    // -- Collections --------------------------------------------------------------------

    /// Builds an Array in Rust and hands it back, so GDScript checks both contents and length.
    #[func]
    fn make_array(&mut self, count: i64) -> VariantArray {
        let mut arr = VariantArray::new();
        for i in 0..count {
            arr.push(&(i * 10).to_variant());
        }
        arr
    }

    /// Reads an Array built in GDScript.
    #[func]
    fn sum_array(&mut self, arr: VariantArray) -> i64 {
        let mut total = 0;
        for i in 0..arr.len() {
            if let Some(v) = i64::try_from_variant(&arr.get(i)) {
                total += v;
            }
        }
        total
    }

    #[func]
    fn make_dictionary(&mut self) -> Dictionary {
        let mut d = Dictionary::new();
        d.set(&"answer".to_variant(), &42i64.to_variant());
        d.set(&"name".to_variant(), &"rust".to_variant());
        d
    }

    #[func]
    fn dictionary_lookup(&mut self, d: Dictionary, key: GString) -> Variant {
        let key = key.to_variant();
        if d.has(&key) {
            d.get_or(&key, &Variant::nil())
        } else {
            (-1i64).to_variant()
        }
    }

    #[func]
    fn make_string_array(&mut self) -> PackedStringArray {
        let mut a = PackedStringArray::new();
        a.push(&GString::new("alpha"));
        a.push(&GString::new("beta"));
        a.push(&GString::new("\u{4e2d}\u{6587}"));
        a
    }

    #[func]
    fn string_array_join(&mut self, a: PackedStringArray) -> GString {
        let parts: Vec<String> = (0..a.len()).map(|i| a.get(i).to_rust_string()).collect();
        GString::new(&parts.join("|"))
    }

    #[func]
    fn make_byte_array(&mut self) -> PackedByteArray {
        let mut a = PackedByteArray::new();
        for b in [1i64, 2, 255] {
            a.push(b);
        }
        a
    }

    #[func]
    fn node_path_roundtrip(&mut self, path: GString) -> NodePath {
        NodePath::from_path(&path.to_rust_string())
    }

    /// Creates and drops containers in a loop. A destructor mistake usually survives the first
    /// call and corrupts memory later, so repetition is the point.
    #[func]
    fn collection_churn(&mut self, rounds: i64) -> i64 {
        let mut total = 0;
        for _ in 0..rounds {
            let mut arr = VariantArray::new();
            arr.push(&1i64.to_variant());
            let cloned = arr.clone();
            total += cloned.len();

            let mut d = Dictionary::new();
            d.set(&"k".to_variant(), &arr.to_variant());
            total += d.len();

            let mut sa = PackedStringArray::new();
            sa.push(&GString::new("x"));
            total += sa.len();
        }
        total
    }

    // -- Object lifetime ----------------------------------------------------------------

    /// Repeated create/free. A ptrcall that corrupts memory usually survives the first call and
    /// fails on a later one, so these probes are called in a loop from GDScript.
    #[func]
    fn probe_new_free(&mut self) -> bool {
        let Some(node) = Gd::<classes::Node>::new() else {
            return false;
        };
        unsafe { node.free() };
        true
    }

    #[func]
    fn probe_new_get_free(&mut self) -> StringName {
        let Some(node) = Gd::<classes::Node>::new() else {
            return StringName::new("<none>");
        };
        let name = classes::Node::get_name(&node);
        unsafe { node.free() };
        name
    }

    #[func]
    fn probe_new_set_free(&mut self) -> bool {
        let Some(node) = Gd::<classes::Node>::new() else {
            return false;
        };
        classes::Node::set_name(&node, StringName::new("Probe"));
        unsafe { node.free() };
        true
    }

    /// Reports the reference count after a create/clone/drop sequence, as `"1,2,1"`.
    ///
    /// Asserting the engine's own count is sharper than watching a global object counter: an
    /// off-by-one shows up immediately instead of as a slow leak.
    #[func]
    fn refcount_probe(&mut self) -> GString {
        let Some(res) = Gd::<classes::Resource>::new() else {
            return GString::new("<none>");
        };

        let after_new =
            classes::RefCounted::get_reference_count(res.upcast_ref::<classes::RefCounted>());

        let copy = res.clone();
        let after_clone =
            classes::RefCounted::get_reference_count(copy.upcast_ref::<classes::RefCounted>());

        drop(copy);
        let after_drop =
            classes::RefCounted::get_reference_count(res.upcast_ref::<classes::RefCounted>());

        GString::new(&format!("{after_new},{after_clone},{after_drop}"))
    }

    /// Drops the last handle and checks the engine no longer knows the object.
    #[func]
    fn refcount_destroys_at_zero(&mut self) -> bool {
        let Some(res) = Gd::<classes::Resource>::new() else {
            return false;
        };
        let id = res.instance_id();
        drop(res);

        // SAFETY: looking up a dead id is defined; it returns null.
        let still_alive =
            unsafe { !godot::sys::interface_fn!(object_get_instance_from_id)(id).is_null() };

        !still_alive
    }
}

godot_entry!(itest_init, ItestLibrary);
