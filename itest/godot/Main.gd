extends Node

# Assertions live here rather than in Rust: this decides the process exit code, and it
# exercises the bindings from the engine side, the way a real user's code would.

var failures: Array[String] = []

# A GDScript runtime error (accessing a property that does not exist, calling a missing method)
# aborts the enclosing function without aborting the script. Every later assertion in that
# function is then silently skipped, and an empty `failures` list reads as success.
#
# Each test therefore records that it ran to completion, and `report` fails if any is missing.
var completed: Array[String] = []
const EXPECTED_TESTS := [
	"class_registration", "variant_roundtrip", "engine_calls", "object_lifecycle",
	"panic_is_contained", "previously_untested_apis",
	"reference_counting", "properties", "signals", "rust_side_connect", "init_levels",
	"math_builtins", "collections", "instance_state", "virtuals",
	"refcounted_class",
]

func check(condition: bool, message: String) -> void:
	if not condition:
		failures.append(message)

func done(name: String) -> void:
	completed.append(name)

func _ready() -> void:
	test_class_registration()
	test_variant_roundtrip()
	test_instance_state()
	test_engine_calls()
	test_object_lifecycle()
	test_reference_counting()
	test_properties()
	test_signals()
	test_rust_side_connect()
	test_init_levels()
	test_panic_is_contained()
	test_previously_untested_apis()
	test_math_builtins()
	test_collections()
	test_refcounted_class()
	# Virtual hooks need real frames to fire, so that check runs after a few of them.
	call_deferred("start_virtual_test")
	return

	report()

func report() -> void:
	for name in EXPECTED_TESTS:
		if not completed.has(name):
			failures.append("test '%s' did not run to completion (a script error aborted it)" % name)

	if failures.is_empty():
		print("itest: OK")
		get_tree().quit(0)
	else:
		for f in failures:
			printerr("itest FAIL: ", f)
		printerr("itest: %d failure(s)" % failures.size())
		get_tree().quit(1)

var virtual_node: Node

func start_virtual_test() -> void:
	# Adding the node to the tree is what makes Godot call _ready and start _process.
	virtual_node = ClassDB.instantiate("RustTestNode")
	add_child(virtual_node)
	# Let a handful of frames pass before reading the counters.
	await get_tree().create_timer(0.25).timeout
	await test_virtuals()
	report()

func test_refcounted_class() -> void:
	# Every other Rust class here descends from Node, which Godot never reference-counts.
	# A Resource is freed by dropping the last reference, a path nothing had exercised.
	check(ClassDB.class_exists("RustTestResource"), "the refcounted class was not registered")
	if not ClassDB.class_exists("RustTestResource"):
		return

	# Control: a built-in Resource through the same path, so the measurement is known to work
	# before it is used to accuse the Rust class of anything.
	var control_before := Performance.get_monitor(Performance.OBJECT_COUNT)
	var control: Object = ClassDB.instantiate("Resource")
	check(control != null, "could not instantiate a built-in Resource")
	control = null
	var control_after := Performance.get_monitor(Performance.OBJECT_COUNT)
	check(control_after == control_before,
		"the object-count measurement is unreliable: a built-in Resource went from %s to %s"
			% [control_before, control_after])

	var before := Performance.get_monitor(Performance.OBJECT_COUNT)

	var res: Object = ClassDB.instantiate("RustTestResource")
	check(res != null, "could not instantiate a refcounted Rust class")
	if res == null:
		return
	check(res.payload() == 7, "refcounted class did not initialise, got %s" % res.payload())
	res.set_payload(11)
	check(res.payload() == 11, "refcounted class did not keep state")
	# The reference count must match a built-in Resource's: the engine treats our creation
	# callback as create_instance3, which owes it an object whose refcount is already claimed.
	check(res.get_reference_count() == 1,
		"a fresh Rust Resource has refcount %s, expected 1" % res.get_reference_count())

	# Dropping the last reference must free it: no free() call, and no leak either.
	res = null

	# Dropping the last reference must actually run the Rust destructor, not merely stop
	# anyone from reaching the object.
	var probe: Object = ClassDB.instantiate("RustTestNode")
	check(probe.resource_free_count() == 1,
		"the Rust resource was never freed: %s destructors ran" % probe.resource_free_count())
	# A class instantiated by the engine is post-initialised by us, since the object is built
	# without it: the engine asks for it through the create callback's argument.
	check(probe.notification_seen(0),
		"NOTIFICATION_POSTINITIALIZE never reached a Rust class")
	probe.free()
	var after := Performance.get_monitor(Performance.OBJECT_COUNT)
	check(after == before,
		"a refcounted Rust class leaked: object count went from %s to %s" % [before, after])

	# A Rust class cannot inherit another Rust class: an object holds one Rust state, so the
	# base class's methods would read the derived class's fields. Registration refuses it, and
	# the refusal is what is checked -- registering it used to succeed and return nonsense.
	check(not ClassDB.class_exists("RustDerivedResource"),
		"a class inheriting another Rust class was registered, which is undefined behaviour")

	# The point of a custom Resource is that it can be saved and loaded again. That needs the
	# class's fields to be registered properties, not just methods.
	var to_save: Object = ClassDB.instantiate("RustTestResource")
	to_save.set_payload(99)
	var path := "user://rust_resource_roundtrip.tres"
	var save_err := ResourceSaver.save(to_save, path)
	check(save_err == OK, "saving a Rust Resource failed with %s" % save_err)
	to_save = null

	var loaded: Object = ResourceLoader.load(path, "", ResourceLoader.CACHE_MODE_IGNORE)
	check(loaded != null, "a saved Rust Resource could not be loaded back")
	if loaded != null:
		check(loaded.get_class() == "RustTestResource",
			"the loaded resource is a %s" % loaded.get_class())
		check(loaded.payload() == 99,
			"the Rust Resource did not survive a save/load round trip: payload is %s"
				% loaded.payload())
		loaded = null

	# A GDScript script extending a Rust class. The object then carries both a script instance
	# and an extension instance; Rust methods and script methods must both still work.
	var script: GDScript = load("res://DerivedInGDScript.gd")
	check(script != null, "could not load a script extending a Rust class")
	if script != null:
		var obj: Object = script.new()
		check(obj != null, "could not instantiate a GDScript class extending a Rust class")
		if obj != null:
			check(obj.bump_script_side() == 1, "the script half of the object did not work")
			# The Rust half keeps its own per-instance state, reached through the script object.
			# Each call is bound first: bump() counts, so calling it inside a message would
			# advance the very thing being reported.
			var first: int = obj.bump()
			var second: int = obj.bump()
			check(first == 1, "the Rust half started at %s, expected 1" % first)
			check(second == 2, "the Rust half did not keep state across calls, got %s" % second)

			# A second object must not share it.
			var other: Object = script.new()
			var other_first: int = other.bump()
			check(other_first == 1,
				"two script objects shared one Rust state: the second started at %s" % other_first)
			other.free()
			obj.free()

	done("refcounted_class")

func test_virtuals() -> void:
	var parts: PackedStringArray = str(virtual_node.virtual_counts()).split(",")
	if parts.size() != 4:
		failures.append("virtual_counts returned %s" % virtual_node.virtual_counts())
		return

	check(int(parts[0]) == 1, "_ready fired %s times, expected exactly 1" % parts[0])
	check(int(parts[1]) > 0, "_process never fired")
	check(int(parts[2]) > 0, "_physics_process never fired")
	check(parts[3] == "true", "accumulated delta was not a plausible frame time")

	# Virtuals that were impossible to override before the dispatch was generalised.
	var extra: PackedStringArray = str(virtual_node.extra_virtual_counts()).split(",")
	check(extra.size() == 3, "extra_virtual_counts returned %s" % virtual_node.extra_virtual_counts())
	if extra.size() == 3:
		check(int(extra[0]) == 1, "_enter_tree fired %s times, expected 1" % extra[0])
		check(int(extra[1]) == 0, "_exit_tree fired %s times before removal, expected 0" % extra[1])

	# The virtual-return path with an owned builtin. No engine virtual returning one can be
	# triggered from a running game, so the Rust side stands in for the engine and builds the
	# return slot the way GDVIRTUAL_CALL does: default-constructed, holding a value the callee
	# must release.
	check(virtual_node.probe_virtual_return(1) == 2,
		"the virtual return slot did not receive the value")

	# Content alone cannot see a leak -- overwriting the slot instead of assigning to it
	# produces the right answer and drops the engine's value on the floor. Only the memory
	# does, across enough iterations to dwarf the noise.
	var before := Performance.get_monitor(Performance.MEMORY_STATIC)
	check(virtual_node.probe_virtual_return(200000) == 2, "probe failed under repetition")
	var leaked := Performance.get_monitor(Performance.MEMORY_STATIC) - before
	check(leaked < 1_000_000,
		"the virtual return path leaked %s bytes over 200k calls" % leaked)

	# An object argument Godot documents as optional can be left out: `create_item()` with no
	# parent makes the root, `create_item_ex(root, -1)` makes a child of it.
	check(virtual_node.optional_object_argument() == 1,
		"an optional object argument did not behave as null, got %s"
			% virtual_node.optional_object_argument())

	# An instance id outlives the object it names, which is what makes it safe to store where
	# a Gd would dangle. 1 = live lookup found it, 2 = the class is checked, 4 = the lookup
	# after free came back empty.
	var id_result: int = virtual_node.instance_id_roundtrip()
	check(id_result & 1 != 0, "looking up a live object by id did not find it")
	check(id_result & 2 != 0, "an id resolved to the wrong class without complaint")
	check(id_result & 4 != 0, "an id still resolved after the object was freed")

	# An object built by Gd::new must be finished, not merely constructed: the interface
	# requires NOTIFICATION_POSTINITIALIZE after construction, and an object that never got it
	# behaves normally right up until the engine puts it in the tree.
	check(virtual_node.fresh_object_survives_the_tree(),
		"a freshly constructed Control did not survive entering the scene tree")

	# _notification has its own slot in the creation info. NOTIFICATION_ENTER_TREE fires when
	# the node is added, so by now the engine must have sent it.
	check(virtual_node.notification_seen(Node.NOTIFICATION_ENTER_TREE),
		"NOTIFICATION_ENTER_TREE was never delivered to _notification")
	check(not virtual_node.notification_seen(999999),
		"_notification reported a notification the engine never sent")

	# Dynamic properties: _get answers names the class never registered, _set consumes one.
	check(virtual_node.dynamic_speed == "got:speed",
		"_get did not answer a dynamic property, got %s" % virtual_node.dynamic_speed)
	virtual_node.dynamic_sink = 77
	check(virtual_node.dynamic_sink_value() == 77,
		"_set did not receive the value, got %d" % virtual_node.dynamic_sink_value())

	# _get_property_list makes the dynamic properties visible to reflection, which is the same
	# data the editor's Inspector reads. Without it _get/_set still work but nothing lists them.
	var dyn_names: Array[String] = []
	for p in virtual_node.get_property_list():
		dyn_names.append(p["name"])
	check(dyn_names.has("dynamic_speed"),
		"dynamic_speed missing from get_property_list()")
	check(dyn_names.has("dynamic_sink"),
		"dynamic_sink missing from get_property_list()")

	# Each requested list must be released. The binding keeps live lists in a map, so a missing
	# free shows up as growth there rather than as silent memory loss.
	for i in range(50):
		var _ignored := virtual_node.get_property_list()
	check(virtual_node.live_property_lists() == 0,
		"%d property lists were never released by the engine"
			% virtual_node.live_property_lists())

	# A virtual with a return value, reached through Godot's own str().
	check(str(virtual_node) == "RustTestNode!",
		"_to_string returned %s" % str(virtual_node))

	await test_async()

	# The `frames` helper, which nothing exercised.
	virtual_node.spawn_frame_waiter(3)
	check(not virtual_node.frame_waiter_done(), "frames() finished before any frame passed")
	await get_tree().create_timer(0.25).timeout
	check(virtual_node.frame_waiter_done(), "frames() never finished")

	# Removing from the tree must fire _exit_tree, proving the counter tracks real events.
	remove_child(virtual_node)
	var after: PackedStringArray = str(virtual_node.extra_virtual_counts()).split(",")
	check(int(after[1]) == 1, "_exit_tree fired %s times after removal, expected 1" % after[1])

	virtual_node.free()
	done("virtuals")

func test_async() -> void:
	# The future advances one step per frame, driven from the node's _process.
	check(virtual_node.spawn_frame_counter(3) == 1, "future was not queued")

	# Checked immediately, before any frame has passed: a future that ran to completion inside
	# spawn would already show progress here. This is what rules out synchronous execution,
	# without depending on exactly when _process runs relative to `await process_frame`.
	check(virtual_node.async_progress() == 0,
		"future made progress before any frame passed, so it is not frame-driven (got %d)"
			% virtual_node.async_progress())

	await get_tree().create_timer(0.25).timeout
	check(virtual_node.async_progress() == -3,
		"future did not finish, progress is %d" % virtual_node.async_progress())

func test_class_registration() -> void:
	check(ClassDB.class_exists("RustTestNode"), "ClassDB does not know RustTestNode")
	check(ClassDB.is_parent_class("RustTestNode", "Node"), "RustTestNode does not inherit Node")
	done("class_registration")

func test_variant_roundtrip() -> void:
	var n: Object = ClassDB.instantiate("RustTestNode")
	if n == null:
		failures.append("could not instantiate RustTestNode")
		return

	check(n.echo_int(42) == 42, "echo_int(42)")
	check(n.echo_int(-7) == -7, "echo_int(-7)")
	check(n.echo_int(9223372036854775807) == 9223372036854775807, "echo_int(i64::MAX)")

	check(n.echo_float(1.5) == 1.5, "echo_float(1.5)")
	check(n.echo_float(-0.25) == -0.25, "echo_float(-0.25)")

	check(n.echo_bool(true) == true, "echo_bool(true)")
	check(n.echo_bool(false) == false, "echo_bool(false)")

	check(n.echo_string("hello") == "hello", "echo_string(hello)")
	check(n.echo_string("") == "", "echo_string(empty)")
	check(n.echo_string("中文 üñî 🎮") == "中文 üñî 🎮", "echo_string(non-ascii)")

	check(n.add_one(41) == 42, "add_one(41)")

	# Wrong argument type must come back as null, not garbage or a crash.
	check(n.echo_int("not an int") == null, "echo_int(wrong type) should be null")

	n.free()
	done("variant_roundtrip")

func test_engine_calls() -> void:
	# The Rust side calls the same engine API through the generated bindings; comparing against
	# GDScript's own result is what proves the ptrcall marshalling is right, not merely that it
	# returned something.
	var n: Object = ClassDB.instantiate("RustTestNode")
	if n == null:
		failures.append("could not instantiate RustTestNode")
		return

	check(n.engine_os_name() == OS.get_name(),
		"OS.get_name() via Rust (%s) != via GDScript (%s)" % [n.engine_os_name(), OS.get_name()])

	check(n.engine_is_editor_hint() == Engine.is_editor_hint(),
		"Engine.is_editor_hint() mismatch")

	check(n.node_name_roundtrip("MyNode") == "MyNode", "Node name round trip")
	check(n.node_name_roundtrip("节点") == "节点", "Node name round trip (non-ascii)")

	n.free()
	done("engine_calls")

func test_object_lifecycle() -> void:
	# Repeated create/call/free. A ptrcall that corrupts memory typically survives the first
	# call and crashes on a later one, so repetition is the point here.
	var n: Object = ClassDB.instantiate("RustTestNode")

	for i in range(5):
		check(n.probe_new_free() == true, "probe_new_free iteration %d" % i)
	for i in range(5):
		# A freshly constructed Node has no name yet; the empty result is correct.
		check(n.probe_new_get_free() == "", "probe_new_get_free iteration %d" % i)
	for i in range(5):
		check(n.probe_new_set_free() == true, "probe_new_set_free iteration %d" % i)

	n.free()
	done("object_lifecycle")

func test_properties() -> void:
	# Properties go through the engine's own property system, not the method call path:
	# assigning with `=` is what proves the setter/getter pair is wired into ClassDB.
	var n: Object = ClassDB.instantiate("RustTestNode")

	n.speed = 12.5
	check(n.speed == 12.5, "float property round trip, got %s" % n.speed)

	n.label = "hello"
	check(n.label == "hello", "string property round trip, got %s" % n.label)

	# The property must also be visible to reflection, which is what the editor uses.
	var names: Array[String] = []
	for p in n.get_property_list():
		names.append(p["name"])
	check(names.has("speed"), "property 'speed' missing from get_property_list()")
	check(names.has("label"), "property 'label' missing from get_property_list()")

	n.free()
	done("properties")

var signal_payloads: Array = []

func test_signals() -> void:
	var n: Object = ClassDB.instantiate("RustTestNode")

	check(n.has_signal("counter_changed"), "signal 'counter_changed' was not registered")

	signal_payloads.clear()
	n.connect("counter_changed", _on_counter_changed)

	var r1 = n.bump_and_emit()
	check(r1 == 1, "bump_and_emit returned %s (negative means the emit failed)" % r1)
	check(n.bump_and_emit() == 2, "bump_and_emit did not accumulate")

	check(signal_payloads == [1, 2],
		"signal payloads were %s, expected [1, 2]" % [signal_payloads])

	n.free()
	done("signals")

func _on_counter_changed(new_value: int) -> void:
	signal_payloads.append(new_value)

func test_rust_side_connect() -> void:
	# A fresh instance, with no GDScript connection on it, so the count reflects only the
	# handler Rust connected. Called once and stored: `check` evaluates both its arguments,
	# so inlining the call in the message would run the whole emit sequence twice.
	var n: Object = ClassDB.instantiate("RustTestNode")

	var hits: int = n.connect_and_emit_from_rust()
	check(hits == 2,
		"Rust-side connect/emit ran the handler %d times; 2 emits while connected, " % hits
			+ "1 after disconnecting. A negative value means it failed before emitting.")
	check(n.last_signal_value() == 8,
		"signal argument was %d, expected the second emit's 8" % n.last_signal_value())

	# A closure needs no registered method behind it. Two emits carrying 3 and 4 must accumulate.
	var closure_sum: int = n.connect_closure_and_emit()
	check(closure_sum == 7,
		"closure callable summed to %d, expected 7 (a negative means connect failed)"
			% closure_sum)

	# The closure must be dropped with the callable, not leaked: 5 guards created, 5 dropped.
	check(n.closure_drop_count(5) == 5,
		"closure captures dropped %d times, expected 5" % n.closure_drop_count(5))

	n.free()
	done("rust_side_connect")

func test_init_levels() -> void:
	# Godot runs the Editor init level during a game run too, so a class must be gated on
	# `Engine.is_editor_hint()` to stay out of a shipped game. This asserts that gate holds.
	check(not Engine.is_editor_hint(), "this test must run as a game, not in the editor")
	check(not ClassDB.class_exists("RustEditorOnlyNode"),
		"editor-only class leaked into a game run")

	# A `runtime` class is registered at scene level and is available while the game runs.
	check(ClassDB.class_exists("RustRuntimeOnlyNode"),
		"runtime-only class was not registered during a game run")

	var n: Object = ClassDB.instantiate("RustRuntimeOnlyNode")
	check(n != null and n.marker() == 2, "runtime-only class did not work")
	if n != null:
		n.free()
	done("init_levels")

func test_panic_is_contained() -> void:
	# A panic inside a #[func] must be reported and contained. If it escaped the FFI boundary
	# the process would abort, and none of the checks after this point would run at all -- so
	# reaching them is itself part of the assertion.
	var victim: Object = ClassDB.instantiate("RustTestNode")

	var result = victim.panicking_method()
	check(result == null,
		"a panicking method should yield null, got %s" % [result])

	# The instance must still be usable: catching is only worth doing if the object survives.
	check(victim.echo_int(7) == 7, "the object was unusable after a panic")
	check(victim.bump() == 1, "instance state was lost after a panic")

	victim.free()
	done("panic_is_contained")

func test_previously_untested_apis() -> void:
	# These were implemented but never exercised: found by listing public names that no test
	# or example mentioned.
	var n: Object = ClassDB.instantiate("RustTestNode")

	# try_cast must succeed for a real base, fail for an unrelated class, and upcast_unchecked
	# must not change what the engine thinks the object is.
	check(n.cast_behaviour() == "true,true,Sprite2D",
		"cast behaviour was %s, expected true,true,Sprite2D" % n.cast_behaviour())

	# A Signal built from an object and a name must report both back.
	check(n.signal_object_and_name() == "counter_changed,true",
		"signal object/name was %s" % n.signal_object_and_name())

	# An untyped view shares the container: 2 elements, then 3 after appending through it.
	check(n.typed_array_untyped_view() == "true,3,3",
		"typed/untyped view gave %s, expected true,3,3" % n.typed_array_untyped_view())

	check(n.variant_nil_check(42), "Variant::is_nil disagreed about nil and non-nil")

	n.free()
	done("previously_untested_apis")

func test_math_builtins() -> void:
	var n: Object = ClassDB.instantiate("RustTestNode")

	# Every component distinct, so a wrong field order shows up as scrambled values rather
	# than an accidental match.
	var t := Transform2D(Vector2(1, 2), Vector2(3, 4), Vector2(5, 6))
	check(n.echo_transform2d(t) == t, "Transform2D round trip, got %s" % n.echo_transform2d(t))

	check(n.echo_vector3(Vector3(1.5, -2.5, 3.5)) == Vector3(1.5, -2.5, 3.5), "Vector3 round trip")
	check(n.echo_color(Color(0.1, 0.2, 0.3, 0.4)).is_equal_approx(Color(0.1, 0.2, 0.3, 0.4)),
		"Color round trip")

	# Cross product of the unit X and Y axes is the unit Z axis, so the length is 1.
	check(abs(n.vector3_cross_length(Vector3(1, 0, 0), Vector3(0, 1, 0)) - 1.0) < 0.0001,
		"Vector3.cross computed in Rust")

	n.free()
	done("math_builtins")

func test_collections() -> void:
	var n: Object = ClassDB.instantiate("RustTestNode")

	# Array built in Rust, read in GDScript.
	var arr: Array = n.make_array(4)
	check(arr.size() == 4, "Rust-built Array size, got %d" % arr.size())
	check(arr == [0, 10, 20, 30], "Rust-built Array contents, got %s" % [arr])

	# Array built in GDScript, read in Rust.
	check(n.sum_array([1, 2, 3, 4]) == 10, "Rust summed a GDScript Array")
	check(n.sum_array([]) == 0, "Rust handled an empty Array")

	var d: Dictionary = n.make_dictionary()
	check(d.size() == 2, "Rust-built Dictionary size")
	check(d.get("answer") == 42, "Dictionary int value")
	check(d.get("name") == "rust", "Dictionary string value")
	check(n.dictionary_lookup({"k": 7}, "k") == 7, "Rust read a GDScript Dictionary")
	check(n.dictionary_lookup({"k": 7}, "missing") == -1, "Rust handled a missing key")

	var sa: PackedStringArray = n.make_string_array()
	check(sa.size() == 3, "PackedStringArray size")
	check(sa[0] == "alpha" and sa[2] == "中文", "PackedStringArray contents, got %s" % [sa])
	check(n.string_array_join(PackedStringArray(["a", "b"])) == "a|b",
		"Rust read a GDScript PackedStringArray")

	var ba: PackedByteArray = n.make_byte_array()
	check(ba.size() == 3 and ba[2] == 255, "PackedByteArray contents, got %s" % [ba])

	check(n.node_path_roundtrip("../Sibling/Child") == NodePath("../Sibling/Child"),
		"NodePath round trip")

	# A method's short form must substitute the same default the engine documents; calling both
	# forms has to produce identical results.
	# One internal child: the short form (include_internal defaults to false) must not count it,
	# the full form passing true must. Encoded as short * 10 + full, so 1 means 0 and 1.
	check(n.default_arguments_match() == 1,
		"default argument substitution: got %d, expected 1 (short=0, full=1)"
			% n.default_arguments_match())

	# The raw-pointer methods. This deliberately asks the engine to load a path that does not
	# exist, so the two errors it prints here are the expected result, not a failure.
	print("  (the next two engine errors are expected: a deliberate bad extension path)")
	# load_extension_from_function rejects a null entry function; transform_from_pose bails out
	# with a default transform when no OpenXR runtime exists, which is the case here -- so this
	# covers the signature and marshalling, not the fate of the pointed-to bytes.
	check(n.raw_pointer_method() == "%d,true" % GDExtensionManager.LOAD_STATUS_FAILED,
		"raw-pointer methods gave %s, expected \"%d,true\""
			% [n.raw_pointer_method(), GDExtensionManager.LOAD_STATUS_FAILED])

	# Methods inherited across several Deref steps must land on the right object: the name and
	# class come from three and four levels up the hierarchy, the flip flag from Sprite2D itself.
	check(n.deref_chain() == "Deep,Sprite2D,true",
		"deref chain gave %s, expected Deep,Sprite2D,true" % n.deref_chain())

	# Enums cross ptrcall as 64-bit integers. PROCESS_MODE_ALWAYS is non-zero, so a wrong width
	# would show up here rather than passing by accident.
	check(n.enum_roundtrip() == Node.PROCESS_MODE_ALWAYS,
		"enum round trip gave %d, expected %d" % [n.enum_roundtrip(), Node.PROCESS_MODE_ALWAYS])

	# Bitfield operations, against the engine's own constants.
	check(n.bitfield_ops() == PROPERTY_USAGE_STORAGE | PROPERTY_USAGE_EDITOR,
		"bitfield or/contains gave %d" % n.bitfield_ops())

	# Methods generated from the API dump. Compared field by field against GDScript computing the
	# same thing, so the check does not depend on how each language formats floats.
	var parts: PackedStringArray = str(n.generated_builtin_methods()).split(",")
	if parts.size() != 5:
		failures.append("generated_builtin_methods returned %s" % n.generated_builtin_methods())
	else:
		check(int(parts[0]) == "hello world".find("world"), "String.find, got %s" % parts[0])
		check(float(parts[1]) == 4.0, "PackedFloat32Array push_back/get, got %s" % parts[1])
		check(float(parts[2]) == 2.0 and float(parts[3]) == 2.0,
			"Vector2.clamp, got (%s, %s)" % [parts[2], parts[3]])
		check(float(parts[4]) == Vector2(3, 4).length(),
			"hand-written Vector2.length, got %s" % parts[4])

	# A typed array must actually carry its element type, not just happen to hold ints:
	# is_typed() is what an engine API expecting Array[int] checks.
	var ti: Array = n.make_typed_ints(3)
	check(ti == [0, 3, 6], "typed array contents, got %s" % [ti])
	check(ti.is_typed(), "array built by Rust is not typed")
	check(ti.get_typed_builtin() == TYPE_INT,
		"typed array element type is %d, expected TYPE_INT" % ti.get_typed_builtin())

	# A typed array coming back from a real engine call (Node.get_children).
	check(n.count_children_via_typed_array() == 3,
		"typed array from get_children, got %d" % n.count_children_via_typed_array())

	# Repeated create/clone/drop: a destructor mistake corrupts memory on a later round.
	# 20 rounds x (1 array + 1 dict + 1 string array) = 60.
	check(n.collection_churn(20) == 60, "collection churn, got %d" % n.collection_churn(20))

	n.free()
	done("collections")

func test_reference_counting() -> void:
	var n: Object = ClassDB.instantiate("RustTestNode")

	# Exact counts, not a leak heuristic: new=1, after clone=2, after dropping the clone=1.
	check(n.refcount_probe() == "1,2,1",
		"reference counts were %s, expected 1,2,1" % n.refcount_probe())

	# Dropping the last handle must actually destroy the object.
	for i in range(3):
		check(n.refcount_destroys_at_zero() == true,
			"object survived its last reference (iteration %d)" % i)

	n.free()
	done("reference_counting")

func test_instance_state() -> void:
	var a: Object = ClassDB.instantiate("RustTestNode")
	var b: Object = ClassDB.instantiate("RustTestNode")

	check(a.bump() == 1, "first bump on a")
	check(a.bump() == 2, "second bump on a")
	check(b.bump() == 1, "b has its own state")
	check(a.bump() == 3, "a state survives b")

	a.free()
	b.free()
	done("instance_state")
