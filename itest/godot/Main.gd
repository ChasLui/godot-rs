extends Node

# Assertions live here rather than in Rust: this decides the process exit code, and it
# exercises the bindings from the engine side, the way a real user's code would.

var failures: Array[String] = []

func check(condition: bool, message: String) -> void:
	if not condition:
		failures.append(message)

func _ready() -> void:
	test_class_registration()
	test_variant_roundtrip()
	test_instance_state()
	test_engine_calls()
	test_object_lifecycle()
	test_reference_counting()
	test_properties()
	test_signals()
	test_init_levels()
	# Virtual hooks need real frames to fire, so that check runs after a few of them.
	call_deferred("start_virtual_test")
	return

	report()

func report() -> void:
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
	test_virtuals()
	report()

func test_virtuals() -> void:
	var parts: PackedStringArray = str(virtual_node.virtual_counts()).split(",")
	if parts.size() != 4:
		failures.append("virtual_counts returned %s" % virtual_node.virtual_counts())
		return

	check(int(parts[0]) == 1, "_ready fired %s times, expected exactly 1" % parts[0])
	check(int(parts[1]) > 0, "_process never fired")
	check(int(parts[2]) > 0, "_physics_process never fired")
	check(parts[3] == "true", "accumulated delta was not a plausible frame time")

	await test_async()

	virtual_node.free()

func test_async() -> void:
	# The future advances one step per frame, driven from the node's _process. Watching it move
	# across real frames is the point: a future that completed instantly would also "work".
	check(virtual_node.spawn_frame_counter(3) == 1, "future was not queued")

	await get_tree().process_frame
	await get_tree().process_frame
	var midway: int = virtual_node.async_progress()
	check(midway > 0, "future never advanced")
	check(midway < 3, "future advanced too fast; it is not waiting per frame (got %d)" % midway)

	await get_tree().create_timer(0.25).timeout
	check(virtual_node.async_progress() == -3,
		"future did not finish, progress is %d" % virtual_node.async_progress())

func test_class_registration() -> void:
	check(ClassDB.class_exists("RustTestNode"), "ClassDB does not know RustTestNode")
	check(ClassDB.is_parent_class("RustTestNode", "Node"), "RustTestNode does not inherit Node")

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

func _on_counter_changed(new_value: int) -> void:
	signal_payloads.append(new_value)

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

func test_instance_state() -> void:
	var a: Object = ClassDB.instantiate("RustTestNode")
	var b: Object = ClassDB.instantiate("RustTestNode")

	check(a.bump() == 1, "first bump on a")
	check(a.bump() == 2, "second bump on a")
	check(b.bump() == 1, "b has its own state")
	check(a.bump() == 3, "a state survives b")

	a.free()
	b.free()
