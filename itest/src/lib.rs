//! Integration tests, loaded by Godot as a real GDExtension.
//!
//! Assertions live on the GDScript side (`itest/godot/`), which decides the process exit code.
//! Round-tripping through GDScript is what makes these tests meaningful: it exercises the same
//! path a real user's code takes.

use godot::builtin::{
    Callable, Color, Dictionary, NodePath, PackedByteArray, PackedFloat32Array, PackedStringArray,
    Signal, Transform2D, TypedArray, VariantArray, Vector2, Vector3,
};
use godot::classes;
use godot::global;
use godot::prelude::*;
use godot::sys;

/// Bumps [`DROPPED`] when it goes away, so a closure's lifetime can be observed from GDScript.
struct DropGuard;

impl Drop for DropGuard {
    fn drop(&mut self) {
        DROPPED.with(|d| d.set(d.get() + 1));
    }
}

thread_local! {
    /// How many `DropGuard`s have been dropped.
    static DROPPED: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };

    /// Whether the `frames()` waiter finished.
    static FRAMES_DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };

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
                register_class::<RustTestResource>();
                // Refused, and the refusal is the point -- see RustDerivedResource. The error
                // it prints is expected.
                register_class::<RustDerivedResource>();
            },
            // Levels are startup phases, not modes: Godot runs the Editor level during a game
            // run too. Registering something editor-only therefore needs an explicit check.
            InitLevel::Editor if is_editor() => unsafe {
                register_class::<RustEditorOnlyNode>();

                // The plugin is added *after* its class is in ClassDB -- the engine looks the
                // name up and instantiates it right here.
                #[cfg(feature = "editor")]
                {
                    register_class::<RustTestPlugin>();
                    register_class::<RustPluginProbe>();
                    godot::editor::add_editor_plugin::<RustTestPlugin>();
                }
            },
            _ => {}
        }
    }

    fn on_level_deinit(level: InitLevel) {
        match level {
            InitLevel::Scene => unsafe {
                unregister_class::<RustTestNode>();
                unregister_class::<RustRuntimeOnlyNode>();
                // Inheritors first: the interface header says unregistering a parent before
                // a class that inherits it fails.
                unregister_class::<RustDerivedResource>();
                unregister_class::<RustTestResource>();
            },
            InitLevel::Editor if is_editor() => unsafe {
                // Strictly the reverse of init: the editor holds a live instance, so removing
                // the plugin has to come before the class it is an instance of goes away.
                #[cfg(feature = "editor")]
                {
                    godot::editor::remove_editor_plugin::<RustTestPlugin>();
                    unregister_class::<RustPluginProbe>();
                    unregister_class::<RustTestPlugin>();
                }

                unregister_class::<RustEditorOnlyNode>();
            },
            _ => {}
        }
    }
}

/// Whether Godot is running as an editor rather than playing the game.
fn is_editor() -> bool {
    let engine = classes::Engine::singleton();
    engine.is_editor_hint()
}

/// Counts the engine's calls into the plugin.
///
/// The editor owns the plugin instance and nothing hands it to GDScript, so a static counter is
/// the only way for the assertions to see whether the engine really called in.
#[cfg(feature = "editor")]
static PLUGIN_ENTER_TREE_CALLS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

#[cfg(feature = "editor")]
static PLUGIN_NAME_CALLS: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

/// A real editor plugin, written in Rust and added without a `plugin.cfg`.
#[cfg(feature = "editor")]
struct RustTestPlugin;

#[cfg(feature = "editor")]
#[godot_api(base = EditorPlugin)]
impl RustTestPlugin {
    fn init() -> Self {
        Self
    }

    #[godot_virtual]
    fn enter_tree(&mut self) {
        PLUGIN_ENTER_TREE_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// Returns an owned builtin, which is the path `IntoPtrcallRet` has to assign rather than
    /// overwrite. Unlike the probe in `RustTestNode`, this one is driven by the real engine.
    #[godot_virtual]
    fn get_plugin_name(&mut self) -> GString {
        PLUGIN_NAME_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        GString::new("RustTestPlugin")
    }

    #[godot_virtual]
    fn has_main_screen(&mut self) -> bool {
        false
    }
}

/// Reads the plugin counters back out for the editor-side assertions.
///
/// A separate class rather than two more methods on `RustEditorOnlyNode`: `#[godot_api]` builds
/// its method table from the impl block as written, so a `#[cfg]` on an individual method
/// removes the function while leaving the registration behind it.
#[cfg(feature = "editor")]
struct RustPluginProbe;

#[cfg(feature = "editor")]
#[godot_api(base = Node)]
impl RustPluginProbe {
    fn init() -> Self {
        Self
    }

    /// How many times the engine entered the Rust plugin into the tree. Zero means the class
    /// was registered but never actually added to the editor.
    #[func]
    fn plugin_enter_tree_calls(&mut self) -> i64 {
        PLUGIN_ENTER_TREE_CALLS.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// How many times the engine asked the plugin for its name.
    #[func]
    fn plugin_name_calls(&mut self) -> i64 {
        PLUGIN_NAME_CALLS.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// A reference-counted class, which every previous test class was not.
///
/// Everything else here descends from Node, which Godot does not reference-count: the engine
/// frees it when the tree does, or the user calls `free`. A Resource is the other half of the
/// object model -- GDScript drops the last reference and the object goes away by itself -- and
/// nothing had ever registered one.
struct RustTestResource {
    payload: i64,
}

/// Counts how many `RustTestResource` instances Godot has told us to free.
static RESOURCE_FREES: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

impl Drop for RustTestResource {
    fn drop(&mut self) {
        RESOURCE_FREES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
}

#[godot_api(base = Resource)]
impl RustTestResource {
    fn init() -> Self {
        Self { payload: 7 }
    }

    #[func]
    fn payload(&mut self) -> i64 {
        self.payload
    }

    #[func]
    fn set_payload(&mut self, value: i64) {
        self.payload = value;
    }

    /// The same value as a property, which is what ResourceSaver writes to disk. A custom
    /// Resource whose fields do not survive a save is not much of a Resource.
    #[prop(set = set_payload)]
    fn get_payload(&mut self) -> i64 {
        self.payload
    }
}

impl RustTestNode {
    /// Not a `#[func]` on the resource itself -- by the time the answer matters, the resource
    /// is meant to be gone.
    fn resource_frees() -> i64 {
        RESOURCE_FREES.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// A Rust class whose base is another Rust class, which the object model cannot support.
///
/// `base = X` is only a name, so this compiles and used to register: the derived object then
/// carried one Rust state that both classes claimed, and the base class's methods read the
/// derived class's fields. Registration must refuse it.
struct RustDerivedResource {
    extra: i64,
}

#[godot_api(base = RustTestResource)]
impl RustDerivedResource {
    fn init() -> Self {
        Self { extra: 42 }
    }

    #[func]
    fn extra(&mut self) -> i64 {
        self.extra
    }
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
    signal_hits: i64,
    last_signal_value: i64,
    enter_tree_calls: i64,
    exit_tree_calls: i64,
    input_events: i64,
    notifications: Vec<i32>,
    dynamic_sink: i64,
    panic_in_virtual: bool,
    ready_calls: i64,
    process_calls: i64,
    physics_calls: i64,
    accumulated_delta: f64,
    offset: Vector2,
    target: Option<Gd<classes::Node>>,
}

#[godot_api(base = Node)]
impl RustTestNode {
    fn init() -> Self {
        Self {
            counter: 0,
            speed: 0.0,
            label: GString::new("unset"),
            base: std::ptr::null_mut(),
            signal_hits: 0,
            last_signal_value: 0,
            enter_tree_calls: 0,
            exit_tree_calls: 0,
            input_events: 0,
            notifications: Vec::new(),
            dynamic_sink: 0,
            panic_in_virtual: false,
            ready_calls: 0,
            process_calls: 0,
            physics_calls: 0,
            accumulated_delta: 0.0,
            offset: Vector2::ZERO,
            target: None,
        }
    }

    // -- Signals and properties ---------------------------------------------------------

    /// Declared, not implemented: the body is ignored, only the name and the arguments' names
    /// and types register.
    #[signal]
    fn counter_changed(new_value: i64) {}

    /// A signal carrying an object, which is what Godot's own signals mostly do -- and the
    /// argument declares its class, so the connection dialog and `get_signal_list` show it.
    #[signal]
    fn target_changed(node: Gd<classes::Node>, label: GString) {}

    /// A float property, backed by the accessor pair below. The hint is what makes the
    /// inspector draw a slider instead of a spin box -- GDScript spells it `@export_range`.
    #[prop(set = set_speed, hint = PROPERTY_HINT_RANGE, hint_string = "0,100,0.5")]
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

        match this.emit_signal(
            &StringName::new("counter_changed"),
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

    /// Only the hot-reload path reaches this, so a sentinel here proves the instance was
    /// rebuilt rather than merely reset.
    fn on_recreated(&mut self) {
        self.counter = 100;
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

    // Beyond the three that used to be hard-coded: any engine virtual can be overridden now.
    #[godot_virtual]
    fn enter_tree(&mut self) {
        self.enter_tree_calls += 1;
    }

    #[godot_virtual]
    fn exit_tree(&mut self) {
        self.exit_tree_calls += 1;
    }

    /// Takes an object argument, so the trampoline has to unpack a `Gd` rather than a scalar.
    #[godot_virtual]
    fn input(&mut self, event: Option<Gd<classes::InputEvent>>) {
        if event.is_some() {
            self.input_events += 1;
        }
    }

    /// Returns a value, exercising the trampoline's return path.
    #[godot_virtual]
    fn to_string(&mut self) -> GString {
        GString::new("RustTestNode!")
    }

    /// The only virtual whose return value is neither `()` nor `Copy`. The engine only asks for
    /// it in the editor, so the editor suite is where it actually fires; here it exists so the
    /// trampoline for a non-`Copy` return type is at least generated and registered.
    #[godot_virtual]
    fn get_configuration_warnings(&mut self) -> PackedStringArray {
        let mut warnings = PackedStringArray::new();
        warnings.push(&GString::new("first warning"));
        warnings.push(&GString::new("second warning"));
        warnings
    }

    /// Stands in for the engine on the virtual-return path.
    ///
    /// No engine virtual that returns an owned builtin can be triggered from a running game --
    /// virtuals are not in ClassDB's callable method table, so GDScript cannot invoke one, and
    /// the only two on `Node` that qualify are editor-only. So this drives the real
    /// `IntoPtrcallRet` with a slot built the way `GDVIRTUAL_CALL` builds one: default
    /// constructed, then holding a value the callee is required to release.
    ///
    /// Returns the number of entries the slot ends up with, so a wrong answer is visible; the
    /// leak that `ptr::write` would cause is not visible here and is measured by the caller
    /// watching memory across many iterations.
    #[func]
    fn probe_virtual_return(&mut self, iterations: i64) -> i64 {
        use godot::godot_core::virtuals::IntoPtrcallRet;

        let mut last = -1;
        for _ in 0..iterations {
            // The engine's slot: default-constructed, then carrying a value with a heap
            // allocation behind it. Assigning releases it; overwriting leaks it.
            let mut slot = PackedStringArray::new();
            slot.push(&GString::new(
                "the engine's own value, which must be released",
            ));

            let mut ours = PackedStringArray::new();
            ours.push(&GString::new("first warning"));
            ours.push(&GString::new("second warning"));

            // SAFETY: `slot` is an initialized value of exactly this type, which is what the
            // engine guarantees for a virtual's return slot.
            unsafe {
                ours.into_ret(&mut slot as *mut PackedStringArray as sys::GDExtensionTypePtr);
            }

            last = slot.size();
            if last != 2 || slot.get(0).to_rust_string() != "first warning" {
                return -1;
            }
        }
        last
    }

    /// Godot's `_notification`, which has its own slot rather than going through the by-name
    /// dispatch. Records the notifications the engine sends while entering the tree.
    #[godot_virtual]
    fn notification(&mut self, what: i32, _reversed: bool) {
        self.notifications.push(what);
    }

    /// A property the class handles dynamically: `dynamic_*` names are answered here rather
    /// than being registered up front.
    #[godot_virtual]
    fn get(&mut self, property: &str) -> Option<Variant> {
        property
            .strip_prefix("dynamic_")
            .map(|rest| GString::new(&format!("got:{rest}")).to_variant())
    }

    /// Declares the dynamic properties, so the editor and reflection can see them.
    #[godot_virtual]
    fn get_property_list(&mut self) -> Vec<PropertyDesc> {
        vec![
            PropertyDesc::new(
                "dynamic_speed",
                godot::sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_STRING,
            ),
            PropertyDesc::new(
                "dynamic_sink",
                godot::sys::GDExtensionVariantType_GDEXTENSION_VARIANT_TYPE_INT,
            ),
        ]
    }

    #[godot_virtual]
    fn set(&mut self, property: &str, value: &Variant) -> bool {
        if property == "dynamic_sink" {
            self.dynamic_sink = i64::try_from_variant(value).unwrap_or(-1);
            true
        } else {
            false
        }
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

    /// Panics on purpose. Unwinding out of an `extern "C"` callback is undefined behaviour and
    /// aborts in practice, so the boundary catches it: the engine reports the panic and keeps
    /// running instead of taking the editor down with unsaved work.
    #[func]
    fn panicking_method(&mut self) -> i64 {
        let empty: Vec<i64> = Vec::new();
        empty[5]
    }

    /// The same, from a virtual method, which dispatches through a different boundary.
    #[godot_virtual]
    fn unhandled_key_input(&mut self, _event: Option<Gd<classes::InputEvent>>) {
        if self.panic_in_virtual {
            panic!("deliberate panic from a virtual method");
        }
    }

    #[func]
    fn arm_virtual_panic(&mut self) {
        self.panic_in_virtual = true;
    }

    /// Exercises the object-model APIs that had no coverage: a cast that should succeed, one
    /// that should not, and an upcast.
    ///
    /// Returns `"ok_cast,bad_cast,upcast_class"`.
    #[func]
    fn cast_behaviour(&mut self) -> GString {
        let Some(sprite) = Gd::<classes::Sprite2D>::new() else {
            return GString::new("<none>");
        };

        // Sprite2D is a Node2D, so this must succeed and address the same object.
        let ok_cast = match sprite.try_cast::<classes::Node2D>() {
            Some(as_node2d) => as_node2d.instance_id() == sprite.instance_id(),
            None => false,
        };

        // It is not a Camera3D, so this must fail rather than hand back a bogus handle.
        let bad_cast = sprite.try_cast::<classes::Camera3D>().is_none();

        // An unchecked upcast keeps the object; the engine still reports its real class.
        let cloned = sprite.clone();
        let as_object = unsafe { cloned.upcast_unchecked::<classes::Object>() };
        let upcast_class = as_object.get_class().to_rust_string();

        unsafe { sprite.free() };
        GString::new(&format!("{ok_cast},{bad_cast},{upcast_class}"))
    }

    /// Builds a `Signal` from an object and a name, and checks the engine agrees about both.
    #[func]
    fn signal_object_and_name(&mut self) -> GString {
        let Some(this) = (unsafe { Gd::<classes::Object>::from_obj_ptr(self.base) }) else {
            return GString::new("<none>");
        };

        let signal = Signal::from_object_signal(&this, &StringName::new("counter_changed"));
        let name = signal.get_name().to_rust_string();
        // Signal::get_object_id is declared signed in the API while Gd::instance_id follows
        // GDObjectInstanceID, which is unsigned; the value is the same either way.
        let same_object = signal.get_object_id() as u64 == this.instance_id();

        GString::new(&format!("{name},{same_object}"))
    }

    /// A typed array viewed as an untyped one shares the container rather than copying it.
    #[func]
    fn typed_array_untyped_view(&mut self) -> GString {
        let mut typed = TypedArray::<i64>::new();
        typed.push(&5);
        typed.push(&6);

        let untyped = typed.to_untyped();
        let same_len = untyped.len() == typed.len();

        // Reference semantics: appending through the untyped view is visible in the typed one.
        let mut untyped2 = typed.to_untyped();
        untyped2.push(&7i64.to_variant());

        GString::new(&format!("{same_len},{},{}", untyped.len(), typed.len()))
    }

    /// `Variant::is_nil`, which nothing exercised.
    #[func]
    fn variant_nil_check(&mut self, value: Variant) -> bool {
        Variant::nil().is_nil() && !value.is_nil()
    }

    /// Waits a fixed number of frames using the `frames` helper, then records completion.
    #[func]
    fn spawn_frame_waiter(&mut self, count: i64) {
        FRAMES_DONE.with(|d| d.set(false));
        AsyncRuntime::spawn(async move {
            frames(count as u32).await;
            FRAMES_DONE.with(|d| d.set(true));
        });
    }

    #[func]
    fn frame_waiter_done(&mut self) -> bool {
        FRAMES_DONE.with(|d| d.get())
    }

    // -- Benchmarks ---------------------------------------------------------------------

    /// Sums 0..n in Rust. Compared against the same loop in GDScript, this measures what the
    /// language buys once the call overhead is amortised over real work.
    #[func]
    fn bench_sum(&mut self, n: i64) -> i64 {
        let mut total: i64 = 0;
        for i in 0..n {
            total = total.wrapping_add(i);
        }
        total
    }

    /// Does nothing. Called in a loop from GDScript, this isolates the cost of crossing the
    /// boundary from the cost of the work.
    #[func]
    fn bench_noop(&mut self) {}

    /// Calls an engine method `n` times through ptrcall, to price the generated bindings.
    #[func]
    fn bench_engine_calls(&mut self, n: i64) -> i64 {
        let Some(node) = Gd::<classes::Node>::new() else {
            return -1;
        };

        let mut len = 0i64;
        for _ in 0..n {
            // get_name returns a StringName, so this covers a call plus a builtin return.
            len += node.get_name().to_rust_string().len() as i64;
        }

        unsafe { node.free() };
        len
    }

    /// Constructs `n` StringNames, which every engine call by name has to do.
    #[func]
    fn bench_stringname(&mut self, n: i64) -> i64 {
        let mut total = 0i64;
        for _ in 0..n {
            let s = StringName::new("some_method_name");
            total += s.to_rust_string().len() as i64;
        }
        total
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

    /// Property lists the engine has requested and not released. A leak shows up as growth.
    #[func]
    fn live_property_lists(&mut self) -> i64 {
        godot::registry::live_property_list_count() as i64
    }

    /// Whether the engine sent the given notification, and what `_set` last stored.
    /// Whether an object argument Godot documents as optional can actually be left out.
    ///
    /// Godot's `PtrToArg<T*>` checks the argument pointer itself before dereferencing it, so a
    /// null object is passed by passing no pointer at all -- there is nothing to construct. The
    /// bindings spell that `Option<&Gd<T>>`, with the short form omitting the argument.
    ///
    /// `Tree::create_item` distinguishes the two: with no parent it creates the root, with one
    /// it creates a child. Returns the root's child count, so passing something other than null
    /// gives a different answer rather than merely not crashing.
    #[func]
    fn optional_object_argument(&mut self) -> i64 {
        let Some(tree) = Gd::<classes::Tree>::new() else {
            return -1;
        };

        // No parent: this becomes the tree's root.
        let Some(root) = tree.create_item() else {
            unsafe { tree.free() };
            return -2;
        };

        // With a parent: a child of the root.
        let child = tree.create_item_ex(Some(&root), -1);
        let count = if child.is_some() {
            i64::from(root.clone().get_child_count())
        } else {
            -3
        };

        unsafe { tree.free() };
        count
    }

    /// Whether a mistyped argument reaches the engine as a call error.
    ///
    /// The generated shim answers `Err` so `method_call` can set
    /// GDEXTENSION_CALL_ERROR_INVALID_ARGUMENT; returning nil instead would leave GDScript
    /// with a call that looks like it worked. GDScript cannot observe the difference from the
    /// outside -- both produce null -- so the shim is asked directly.
    #[func]
    fn wrong_argument_is_reported(&mut self) -> bool {
        let wrong = [GString::new("not an int").to_variant()];
        let right = [7i64.to_variant()];

        let rejected = Self::__godot_shim_echo_int(self, &wrong).is_err();
        let accepted = Self::__godot_shim_echo_int(self, &right).is_ok();
        rejected && accepted
    }

    /// A Vector2 property. Position and velocity are what a class exports most, and until now
    /// the property types stopped at strings and numbers.
    #[prop(set = set_offset)]
    fn get_offset(&mut self) -> Vector2 {
        self.offset
    }

    #[func]
    fn set_offset(&mut self, value: Vector2) {
        self.offset = value;
    }

    /// A property that exists at runtime but is neither saved nor shown in the inspector.
    /// The default usage cannot express that, and a computed value has no business being
    /// written into a scene file.
    #[prop(set = set_transient, usage = PROPERTY_USAGE_NONE)]
    fn get_transient(&mut self) -> i64 {
        self.counter * 2
    }

    #[func]
    fn set_transient(&mut self, _value: i64) {}

    /// Saved but hidden from the inspector, which needs two flags OR'd together -- the form
    /// Godot's own usage constants are meant to be combined in.
    #[prop(set = set_hidden, usage = PROPERTY_USAGE_STORAGE | PROPERTY_USAGE_INTERNAL)]
    fn get_hidden(&mut self) -> i64 {
        self.counter
    }

    #[func]
    fn set_hidden(&mut self, _value: i64) {}

    /// An object property. The engine is told which class it holds, so the inspector shows a
    /// typed slot rather than an untyped one that accepts anything.
    #[prop(set = set_target)]
    fn get_target(&mut self) -> Gd<classes::Node> {
        // SAFETY: the base object is alive for as long as the engine is calling in.
        unsafe {
            self.target
                .clone()
                .or_else(|| Gd::<classes::Node>::from_obj_ptr(self.base))
                .expect("the base object is never null once the engine has set it")
        }
    }

    /// Takes an `Option`, so `node.target = null` clears it instead of being rejected as a
    /// type error -- clearing an object property is ordinary in Godot.
    #[func]
    fn set_target(&mut self, value: Option<Gd<classes::Node>>) {
        self.target = value;
    }

    /// Hands an object back to GDScript as a return value.
    ///
    /// The other direction from `takes_node2d`: a `Gd` has to survive being turned into a
    /// Variant and read out again on the far side.
    #[func]
    fn make_node2d(&mut self) -> Gd<classes::Node2D> {
        let node = Gd::<classes::Node2D>::new().expect("Node2D is a registered engine class");
        node.set_name(&StringName::new("MadeInRust"));
        node
    }

    /// Puts objects into the containers, which is the same Variant conversion under a different
    /// name -- and the one that decides whether a class can keep a list of nodes.
    #[func]
    fn objects_through_containers(&mut self) -> i64 {
        let Some(node) = Gd::<classes::Node2D>::new() else {
            return -1;
        };
        let id = node.instance_id();

        let mut array = VariantArray::new();
        array.push(&node.to_variant());

        let mut dict = Dictionary::new();
        dict.set(&GString::new("node").to_variant(), &node.to_variant());

        let from_array = Gd::<classes::Node2D>::try_from_variant(&array.get(0));
        let from_dict = Gd::<classes::Node2D>::try_from_variant(
            &dict.get(&GString::new("node").to_variant(), &Variant::nil()),
        );

        let mut result = 0;
        if from_array.map(|g| g.instance_id()) == Some(id) {
            result |= 1;
        }
        if from_dict.map(|g| g.instance_id()) == Some(id) {
            result |= 2;
        }
        // A container holding a Node2D must not answer a Label request.
        if Gd::<classes::Label>::try_from_variant(&array.get(0)).is_none() {
            result |= 4;
        }

        // SAFETY: the containers hold Variants, which do not own a manually-managed node.
        unsafe { node.free() };
        result
    }

    /// Reaches another Rust object's state directly, without going back through the engine.
    ///
    /// `Gd` names an engine object; the Rust fields behind it are reached with
    /// `rust_instance`. Returns the other node's counter, or a negative code, so a wrong
    /// answer is distinguishable from a refusal.
    #[func]
    fn read_other_rust_state(&mut self, other: Gd<classes::Node>) -> i64 {
        // SAFETY: the reference is used and dropped here, with nothing in between that could
        // re-enter the object.
        match unsafe { godot::registry::rust_instance::<RustTestNode, _>(&other) } {
            Some(state) => state.counter,
            None => -2,
        }
    }

    /// Takes a `Gd` of a specific class. A Variant records only that it holds *an* object, so
    /// the class has to be checked when it is converted back -- otherwise any object at all
    /// satisfies any `Gd<T>`, and the methods called on it belong to a different type.
    #[func]
    fn takes_node2d(&mut self, node: Gd<classes::Node2D>) -> i64 {
        let _ = node;
        1
    }

    /// The same call against an object that is not a `RustTestNode`, which must be refused
    /// rather than reinterpreting whatever state that object has.
    #[func]
    fn read_wrong_rust_state(&mut self, other: Gd<classes::Node>) -> i64 {
        match unsafe { godot::registry::rust_instance::<RustRuntimeOnlyNode, _>(&other) } {
            Some(_) => -3,
            None => 0,
        }
    }

    /// Whether an object id outlives the object it names.
    ///
    /// A `Gd` to a manually-managed object dangles the moment someone frees it, and nothing can
    /// test a dangling pointer. An id can be tested, which is what makes it the way to hold an
    /// object across frames.
    ///
    /// Returns a bitmask so a wrong answer is distinguishable from a crash: 1 = the live
    /// lookup found it, 2 = the class was checked rather than assumed, 4 = the lookup after
    /// `free` came back empty.
    #[func]
    fn instance_id_roundtrip(&mut self) -> i64 {
        let Some(node) = Gd::<classes::Node>::new() else {
            return 0;
        };

        let id = node.instance_id();
        let mut result = 0;

        if let Some(found) = Gd::<classes::Node>::from_instance_id(id) {
            if found.as_obj_ptr() == node.as_obj_ptr() {
                result |= 1;
            }
        }

        // The id is engine-wide and says nothing about the class: a Node is not a Resource.
        if Gd::<classes::Resource>::from_instance_id(id).is_none() {
            result |= 2;
        }

        // SAFETY: Node is not reference-counted and this is the only handle.
        unsafe { node.free() };

        // The whole point: the id survives the object, and resolving it now finds nothing.
        if Gd::<classes::Node>::from_instance_id(id).is_none() {
            result |= 4;
        }

        result
    }

    /// Adds a freshly built engine object to the live scene tree.
    ///
    /// `classdb_construct_object` builds an object but leaves it to the caller to send
    /// NOTIFICATION_POSTINITIALIZE, which the interface header requires. An object that never
    /// receives it answers ordinary method calls perfectly well -- it can be constructed, have
    /// its text set and read back, and be parented to a node that is *outside* the tree. It
    /// dies the moment the engine takes it seriously, which for a Node means entering the tree
    /// and being sent NOTIFICATION_ENTER_TREE.
    ///
    /// So this parents a new Control to a node that is already in the tree, which is the
    /// smallest thing that segfaults when the notification is skipped. Returning at all is the
    /// assertion; there is no wrong value to check for.
    #[func]
    fn fresh_object_survives_the_tree(&mut self) -> bool {
        let Some(this) = (unsafe { Gd::<classes::Node>::from_obj_ptr(self.base) }) else {
            return false;
        };
        let Some(label) = Gd::<classes::Label>::new() else {
            return false;
        };

        this.add_child(label.upcast_ref());
        let entered = label.clone().is_inside_tree();
        this.remove_child(label.upcast_ref());
        // SAFETY: removed from the tree, and this is the only handle left.
        unsafe { label.free() };

        entered
    }

    /// How many `RustTestResource` instances the engine has freed.
    #[func]
    fn resource_free_count(&mut self) -> i64 {
        Self::resource_frees()
    }

    #[func]
    fn notification_seen(&mut self, what: i64) -> bool {
        self.notifications.contains(&(what as i32))
    }

    #[func]
    fn dynamic_sink_value(&mut self) -> i64 {
        self.dynamic_sink
    }

    /// Counts for the virtuals that were impossible to override before.
    #[func]
    fn extra_virtual_counts(&mut self) -> GString {
        GString::new(&format!(
            "{},{},{}",
            self.enter_tree_calls, self.exit_tree_calls, self.input_events
        ))
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
        os.get_name()
    }

    /// A bool-returning engine call, exercising a different ptrcall return width.
    #[func]
    fn engine_is_editor_hint(&mut self) -> bool {
        let engine = classes::Engine::singleton();
        engine.is_editor_hint()
    }

    /// Round-trips through a real engine object: set a Node's name, read it back.
    #[func]
    fn node_name_roundtrip(&mut self, name: StringName) -> StringName {
        let Some(node) = Gd::<classes::Node>::new() else {
            return StringName::new("");
        };

        node.set_name(&name);
        let read_back = node.get_name();

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
            d.get(&key, &Variant::nil())
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

    /// Checks that a short form substitutes the default Godot documents, not some other value.
    ///
    /// The child is added as *internal*, which `get_child_count` only counts when explicitly
    /// asked. So the short form (`include_internal` defaulting to false) must see 0 while the
    /// full form passing true sees 1 -- a wrong default flips one of them.
    /// Returns the two counts encoded as `short * 10 + full`.
    #[func]
    fn default_arguments_match(&mut self) -> i64 {
        let Some(parent) = Gd::<classes::Node>::new() else {
            return -1;
        };
        let Some(child) = Gd::<classes::Node>::new() else {
            return -1;
        };

        parent.add_child_ex(
            &child,
            false,
            classes::NodeInternalMode::INTERNAL_MODE_FRONT,
        );

        let short = parent.get_child_count();
        let full = parent.get_child_count_ex(true);

        unsafe { parent.free() };
        (short as i64) * 10 + full as i64
    }

    /// Calls the two raw-pointer methods that can be reached without external hardware.
    ///
    /// These three methods are generated `unsafe` because the API description says nothing
    /// about what the pointer addresses. What is checked here is that the generated signature
    /// and marshalling are right -- the call goes through, returns the value the engine
    /// documents for the input, and does not corrupt anything.
    ///
    /// It is *not* a check that the pointed-to bytes reach their consumer:
    /// `transform_from_pose` returns a default Transform3D before dereferencing when no OpenXR
    /// runtime is present, which is always the case in CI. Verifying that would need an XR
    /// device.
    ///
    /// Returns `"status,origin_is_default"`.
    #[func]
    fn raw_pointer_method(&mut self) -> GString {
        let manager = classes::GDExtensionManager::singleton();

        // SAFETY: null is a defined input -- the engine reports failure rather than
        // dereferencing it.
        let status = unsafe {
            manager.load_extension_from_function(
                &GString::new("res://does_not_exist.gdextension"),
                std::ptr::null(),
            )
        };

        let origin_is_default = match Gd::<classes::OpenXRAPIExtension>::new() {
            Some(api) => {
                // XrPosef: orientation (x, y, z, w) then position (x, y, z).
                let pose: [f32; 7] = [0.0, 0.0, 0.0, 1.0, 1.5, -2.5, 3.0];

                // SAFETY: `pose` matches the layout the method expects and outlives the call.
                let t =
                    unsafe { api.transform_from_pose(pose.as_ptr() as *const std::ffi::c_void) };
                t.origin == godot::builtin::Vector3::ZERO
            }
            None => true,
        };

        GString::new(&format!("{},{origin_is_default}", status.ord()))
    }

    /// Calls methods reached through several Deref steps, to check the chain resolves to the
    /// right object rather than merely compiling.
    ///
    /// `Sprite2D` inherits Node2D -> CanvasItem -> Node -> Object, so `get_class` comes from
    /// four levels up and `set_name`/`get_name` from three.
    #[func]
    fn deref_chain(&mut self) -> GString {
        let Some(sprite) = Gd::<classes::Sprite2D>::new() else {
            return GString::new("<none>");
        };

        // From Node, three levels up.
        sprite.set_name(&StringName::new("Deep"));
        let name = sprite.get_name().to_rust_string();

        // From Object, four levels up.
        let class = sprite.get_class().to_rust_string();

        // Declared on Sprite2D itself.
        sprite.set_flip_h(true);
        let flipped = sprite.is_flipped_h();

        unsafe { sprite.free() };
        GString::new(&format!("{name},{class},{flipped}"))
    }

    /// Round-trips a non-zero enum through the engine.
    ///
    /// Enums cross ptrcall as 64-bit integers; a wrong width would still work for zero, so the
    /// value chosen here is deliberately not the default. Godot's own getter reads it back, so
    /// a truncated or sign-extended value shows up as a mismatch.
    #[func]
    fn enum_roundtrip(&mut self) -> i64 {
        let Some(node) = Gd::<classes::Node>::new() else {
            return -1;
        };

        node.set_process_mode(classes::NodeProcessMode::PROCESS_MODE_ALWAYS);
        let read_back = node.get_process_mode();

        unsafe { node.free() };
        read_back.ord()
    }

    /// A bitfield, checking the generated bit operations agree with the engine's values.
    #[func]
    fn bitfield_ops(&mut self) -> i64 {
        let combined = global::PropertyUsageFlags::PROPERTY_USAGE_STORAGE
            | global::PropertyUsageFlags::PROPERTY_USAGE_EDITOR;

        if !combined.contains(global::PropertyUsageFlags::PROPERTY_USAGE_STORAGE) {
            return -1;
        }
        combined.ord()
    }

    /// Connects a signal to a Rust *closure*, which needs no registered method behind it.
    ///
    /// The closure captures a counter shared with this instance, so GDScript can observe that
    /// the engine really invoked it, with the argument the signal carried.
    #[func]
    fn connect_closure_and_emit(&mut self) -> i64 {
        let Some(this) = (unsafe { Gd::<classes::Object>::from_obj_ptr(self.base) }) else {
            return -1;
        };

        // Shared with the closure; the closure owns one handle, this instance reads the other.
        let seen = std::rc::Rc::new(std::cell::Cell::new(0i64));
        let captured = seen.clone();

        let callable = Callable::from_closure(move |args: &[Variant]| {
            let value = args.first().and_then(i64::try_from_variant).unwrap_or(0);
            captured.set(captured.get() + value);
            Variant::nil()
        });

        let err = this.connect(&StringName::new("counter_changed"), &callable);
        if err != global::Error::OK {
            return -100 - err.ord();
        }

        let _ = this.emit_signal(&StringName::new("counter_changed"), &[3i64.to_variant()]);
        let _ = this.emit_signal(&StringName::new("counter_changed"), &[4i64.to_variant()]);

        this.disconnect(&StringName::new("counter_changed"), &callable);

        // 3 + 4 if the closure ran for both emits.
        seen.get()
    }

    /// Proves a closure callable is dropped with the callable, rather than leaked.
    ///
    /// The closure captures a guard whose Drop bumps a thread-local counter; creating and
    /// dropping N callables must bump it N times.
    #[func]
    fn closure_drop_count(&mut self, rounds: i64) -> i64 {
        DROPPED.with(|d| d.set(0));

        for _ in 0..rounds {
            let callable = Callable::from_closure(|_args: &[Variant]| {
                // Captures the guard below by holding it in the closure environment.
                Variant::nil()
            });
            drop(callable);
        }

        for _ in 0..rounds {
            let guard = DropGuard;
            let callable = Callable::from_closure(move |_args: &[Variant]| {
                let _ = &guard;
                Variant::nil()
            });
            drop(callable);
        }

        DROPPED.with(|d| d.get())
    }

    /// Connects a signal to a Rust method from Rust, then emits it.
    ///
    /// This is the whole point of Callable: before it, a Rust class could declare a signal but
    /// only GDScript could connect to it. Returns the number of times the handler ran.
    #[func]
    fn connect_and_emit_from_rust(&mut self) -> i64 {
        let Some(this) = (unsafe { Gd::<classes::Object>::from_obj_ptr(self.base) }) else {
            return -1;
        };

        self.signal_hits = 0;

        let callable = Callable::from_object_method(&this, &StringName::new("_on_own_signal"));
        if !callable.is_valid() {
            return -2;
        }

        let err = this.connect(&StringName::new("counter_changed"), &callable);
        if err != global::Error::OK {
            return -100 - err.ord();
        }

        if !this.is_connected(&StringName::new("counter_changed"), &callable) {
            return -3;
        }

        let _ = this.emit_signal(&StringName::new("counter_changed"), &[7i64.to_variant()]);
        let _ = this.emit_signal(&StringName::new("counter_changed"), &[8i64.to_variant()]);

        this.disconnect(&StringName::new("counter_changed"), &callable);

        // A third emit after disconnecting must not reach the handler.
        let _ = this.emit_signal(&StringName::new("counter_changed"), &[9i64.to_variant()]);

        self.signal_hits
    }

    /// Signal handler, reached through the Callable above.
    #[func]
    fn _on_own_signal(&mut self, value: i64) {
        self.signal_hits += 1;
        self.last_signal_value = value;
    }

    /// The value the last signal delivered, to prove arguments arrive intact.
    #[func]
    fn last_signal_value(&mut self) -> i64 {
        self.last_signal_value
    }

    /// Exercises methods that are now generated rather than hand-written, across the three
    /// shapes they come in: const with args, mutating, and one returning a builtin.
    #[func]
    fn generated_builtin_methods(&mut self) -> GString {
        // String: const method with an argument.
        let haystack = GString::new("hello world");
        let idx = haystack.find(&GString::new("world"), 0);

        // Packed array: a mutating method, then a const one, on a type that previously had no
        // accessors at all.
        let mut floats = PackedFloat32Array::new();
        floats.push_back(1.5);
        floats.push_back(2.5);
        let float_sum = floats.get(0) + floats.get(1);

        // Vector2: a generated method returning a builtin, alongside the hand-written maths
        // that was deliberately kept in Rust.
        let v = Vector2::new(3.0, 4.0);
        let clamped = v.clamp(Vector2::ZERO, Vector2::new(2.0, 2.0));
        let hand_written_length = v.length();

        GString::new(&format!(
            "{idx},{float_sum},{},{},{hand_written_length}",
            clamped.x, clamped.y
        ))
    }

    /// Builds a typed array in Rust. GDScript checks that the engine really considers it typed,
    /// not merely an Array that happens to hold ints.
    #[func]
    fn make_typed_ints(&mut self, count: i64) -> TypedArray<i64> {
        let mut a = TypedArray::<i64>::new();
        for i in 0..count {
            a.push(&(i * 3));
        }
        a
    }

    /// Reads a typed array produced by the engine, through a real engine call.
    ///
    /// `Node::get_children` returns `typedarray::Node`, so this exercises the generated
    /// signature rather than a hand-made array.
    #[func]
    fn count_children_via_typed_array(&mut self) -> i64 {
        let Some(parent) = Gd::<classes::Node>::new() else {
            return -1;
        };

        for _ in 0..3 {
            if let Some(child) = Gd::<classes::Node>::new() {
                parent.add_child(&child);
            }
        }

        let children = parent.get_children();
        let count = children.len();

        // Reading an element back proves the typed accessor works, not just the length.
        let first_is_node = children.get(0).is_some();

        unsafe { parent.free() };

        if first_is_node {
            count
        } else {
            -2
        }
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
        let name = node.get_name();
        unsafe { node.free() };
        name
    }

    #[func]
    fn probe_new_set_free(&mut self) -> bool {
        let Some(node) = Gd::<classes::Node>::new() else {
            return false;
        };
        node.set_name(&StringName::new("Probe"));
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

        let after_new = res
            .upcast_ref::<classes::RefCounted>()
            .get_reference_count();

        let copy = res.clone();
        let after_clone = copy
            .upcast_ref::<classes::RefCounted>()
            .get_reference_count();

        drop(copy);
        let after_drop = res
            .upcast_ref::<classes::RefCounted>()
            .get_reference_count();

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
