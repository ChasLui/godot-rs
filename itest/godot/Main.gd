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
	"refcounted_class", "base_object", "readonly_property",
	"utility_functions", "packed_arrays", "math_types", "builtin_constants",
	"virtual_panic", "drop_panic", "init_panic", "builtin_operators",
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
	test_utility_functions()
	test_packed_arrays()
	test_math_types()
	test_builtin_constants()
	test_builtin_operators()
	test_virtual_panic()
	test_refcounted_class()
	# Counts destructors, so it must follow the test that asserts an exact destructor count.
	test_drop_panic()
	test_init_panic()
	test_base_object()
	test_readonly_property()
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
		# Armed only on success, so it cannot hide a failure. Caught, it is one error at shutdown;
		# uncaught, it aborts Godot and the nonzero exit code fails the run.
		print("  (the next error is expected: a deliberate panic inside on_level_deinit)")
		var armer: Object = ClassDB.instantiate("RustTestNode")
		armer.arm_level_deinit_panic()
		armer.free()
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

	# Reaching the object from inside the class must leave the count exactly as it was. A handle
	# that forgets to take a count frees this object early; one that forgets to release it keeps
	# the object alive forever, which the leak check at the end of this function catches.
	check(res.class_through_base() == "RustTestResource",
		"a Rust class reached the wrong object through its base: %s" % res.class_through_base())
	check(res.get_reference_count() == 1,
		"reaching the object through its base left refcount %s, expected 1"
			% res.get_reference_count())

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

func test_readonly_property() -> void:
	# A #[prop] with no setter. Godot spells read-only as an empty setter name, so the point of
	# this test is that the engine actually honours that rather than registering something that
	# silently accepts writes.
	var n: Object = ClassDB.instantiate("RustTestNode")
	check(n != null, "could not instantiate RustTestNode")
	if n == null:
		return

	check(n.readonly_marker == 7, "the read-only property read %s, expected 7" % n.readonly_marker)

	# `set()` rather than an assignment: assigning to a property Godot refuses is a runtime
	# error, which would abort this function instead of failing it -- and a function that aborts
	# reports nothing at all.
	n.set("readonly_marker", 99)
	check(n.readonly_marker == 7,
		"writing a read-only property changed it to %s" % n.readonly_marker)

	# It still has to be a property, not just a method: the inspector reads this list.
	var listed := false
	for entry in n.get_property_list():
		if entry.name == "readonly_marker":
			listed = true
	check(listed, "the read-only property is not in the property list")

	n.free()
	done("readonly_property")

func test_base_object() -> void:
	# The base handle must name *this* object. Every other test would pass just as well if it
	# named some other live object of a compatible class -- emitting a signal on the wrong
	# object still emits a signal -- so nothing else here can catch that mistake.
	var n: Object = ClassDB.instantiate("RustTestNode")
	check(n != null, "could not instantiate RustTestNode")
	if n == null:
		return

	var identity: String = n.base_identity()
	var parts: PackedStringArray = identity.split(",")
	check(parts.size() == 2, "base_identity returned %s" % identity)
	if parts.size() == 2:
		check(parts[0] == str(n.get_instance_id()),
			"the base names object %s, but the object itself is %s"
				% [parts[0], n.get_instance_id()])
		check(parts[1] == "RustTestNode", "the base object is a %s" % parts[1])

	n.free()
	done("base_object")

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
		# A headless run has no input device, so _input must not have fired at all. Asserting
		# zero is weaker than asserting a real event count, but it still catches the virtual
		# being invoked with something that is not an event.
		check(int(extra[2]) == 0, "_input fired %s times in a headless run, expected 0" % extra[2])

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

	# One Rust object reaching another's state directly, rather than calling back through
	# the engine. The engine decides the type, so a mismatched class must be refused.
	var peer: Object = ClassDB.instantiate("RustTestNode")
	peer.bump()
	peer.bump()
	check(virtual_node.read_other_rust_state(peer) == 2,
		"reading another Rust object's state gave %s, expected 2"
			% virtual_node.read_other_rust_state(peer))
	check(virtual_node.read_wrong_rust_state(peer) == 0,
		"a mismatched class was not refused")

	# A statically typed reference makes GDScript resolve calls at compile time, which is the
	# ptrcall path -- arguments arrive natively rather than as Variants and have to be rebuilt
	# from their declared types. Nothing but the static method exercised it before.
	var typed: RustTestNode = virtual_node
	check(typed.echo_int(42) == 42, "ptrcall lost an int")
	check(typed.echo_float(1.5) == 1.5, "ptrcall lost a float")
	check(typed.echo_bool(true) == true, "ptrcall lost a bool")
	check(typed.echo_string("中文 üñî") == "中文 üñî", "ptrcall lost a string")
	check(typed.add_one(41) == 42, "ptrcall lost an argument")

	# Property assignment does *not* take this path -- breaking ptrcall's argument handling
	# leaves the property checks above passing. Accessors are reached through the property
	# machinery instead, so they are covered there and not duplicated here.

	# An object as an *argument* through this path: its native form is a pointer to the
	# handle, one indirection more than a value type, and only the return direction was
	# covered before.
	# Declared as Node, not Object: the argument type has to match exactly or GDScript falls
	# back to a dynamic call and this checks nothing.
	var peer2: Node = ClassDB.instantiate("RustTestNode")
	peer2.bump()
	check(typed.read_other_rust_state(peer2) == 1,
		"ptrcall lost an object argument: got %s" % typed.read_other_rust_state(peer2))
	peer2.free()

	# A container argument, whose native form is the container itself rather than a Variant.
	check(typed.sum_array([1, 2, 3]) == 6,
		"ptrcall lost an array argument: got %s" % typed.sum_array([1, 2, 3]))

	# An object through the typed path, in and out.
	var made2: Object = typed.make_node2d()
	check(made2 != null and made2.get_class() == "Node2D", "ptrcall lost an object return")
	if made2 != null:
		made2.free()

	# The ptrcall return slot is constructed by the engine before the call, so writing a
	# container into it without releasing what is there leaks. Measured rather than reasoned
	# about: the content is identical either way.
	check(typed.make_array(3).size() == 3, "ptrcall lost an array return")
	var mem_before := Performance.get_monitor(Performance.MEMORY_STATIC)
	for i in 100000:
		typed.make_array(2)
	var mem_leaked := Performance.get_monitor(Performance.MEMORY_STATIC) - mem_before
	check(mem_leaked < 2_000_000,
		"the ptrcall return path leaked %s bytes over 100k calls" % mem_leaked)

	# A static method is called on the class, without an instance. Registered as a normal
	# method it would demand one and fail with INSTANCE_IS_NULL.
	check(RustTestNode.describe_version(4, 7) == "v4.7",
		"a static method did not answer: %s" % RustTestNode.describe_version(4, 7))

	var static_flagged := false
	for m in virtual_node.get_method_list():
		if m.name == "describe_version":
			static_flagged = (m.flags & METHOD_FLAG_STATIC) != 0
	check(static_flagged, "the static method is not declared static")

	# An exported method describes its arguments: their names as written in Rust, and their
	# types. Without this the editor offers `set_offset(arg0)` and says nothing about it.
	var method_args: Array = []
	var method_ret := -1
	var setter_return_usage := -1
	var setter_return_type := -1
	for m in virtual_node.get_method_list():
		if m.name == "read_other_rust_state":
			method_args = m.args
			method_ret = m.return.type
		elif m.name == "set_offset":
			setter_return_type = m.return.type
			setter_return_usage = m.return.usage
	check(method_args.size() == 1,
		"the method declares %s arguments, expected 1" % method_args.size())
	if method_args.size() == 1:
		check(method_args[0].name == "other",
			"the argument is called %s, expected the Rust name `other`" % method_args[0].name)
		check(method_args[0].type == TYPE_OBJECT,
			"the argument is declared as type %s" % method_args[0].type)
		check(method_args[0].class_name == "Node",
			"the argument declares class %s, expected Node" % method_args[0].class_name)
	check(method_ret == TYPE_INT,
		"the method declares return type %s, expected int" % method_ret)

	# A method returning nothing must say so. Declared as returning a Variant instead, NIL
	# carries PROPERTY_USAGE_NIL_IS_VARIANT and `var x = obj.set_offset(v)` looks sound.
	check(setter_return_type == TYPE_NIL,
		"a method returning nothing declares return type %s" % setter_return_type)
	check(setter_return_usage & PROPERTY_USAGE_NIL_IS_VARIANT == 0,
		"a method returning nothing is declared as returning any Variant (usage %s)"
			% setter_return_usage)

	# A property hint: without it the inspector draws a plain spin box, with it a slider
	# bounded by the hint string. The value round trip cannot see the difference, so the
	# declaration is what gets checked.
	var speed_hint := -1
	var speed_hint_string := ""
	for p in virtual_node.get_property_list():
		if p.name == "speed":
			speed_hint = p.hint
			speed_hint_string = p.hint_string
	# Compared against GDScript's own @export_range, declared the same way in HintControl.gd:
	# the two must be indistinguishable from the engine's side.
	var control_script: GDScript = load("res://HintControl.gd")
	var control_obj: Object = control_script.new()
	var control_hint := -1
	var control_hint_string := ""
	for p in control_obj.get_property_list():
		if p.name == "ranged":
			control_hint = p.hint
			control_hint_string = p.hint_string
	control_obj.free()
	# The hint must match; the hint string need not be spelled identically -- GDScript's
	# compiler normalises 0 to 0.0, and the engine takes either.
	check(speed_hint == control_hint,
		"a Rust ranged property declares hint %s where GDScript declares %s"
			% [speed_hint, control_hint])
	check(not control_hint_string.is_empty(),
		"the GDScript control lost its hint string, so the comparison proves nothing")
	check(speed_hint == PROPERTY_HINT_RANGE,
		"the ranged property declares hint %s, expected PROPERTY_HINT_RANGE" % speed_hint)
	check(speed_hint_string == "0,100,0.5",
		"the range hint string is %s" % speed_hint_string)

	# Property usage: a runtime-only value is neither saved nor shown, which the default
	# usage (STORAGE | EDITOR) cannot say.
	var transient_usage := -1
	var speed_usage := -1
	var hidden_usage := -1
	for p in virtual_node.get_property_list():
		if p.name == "transient":
			transient_usage = p.usage
		elif p.name == "speed":
			speed_usage = p.usage
		elif p.name == "hidden":
			hidden_usage = p.usage
	check(transient_usage == PROPERTY_USAGE_NONE,
		"a runtime-only property declares usage %s, expected NONE" % transient_usage)
	check(speed_usage == PROPERTY_USAGE_DEFAULT,
		"an ordinary property no longer declares the default usage: %s" % speed_usage)
	check(hidden_usage == PROPERTY_USAGE_STORAGE | PROPERTY_USAGE_INTERNAL,
		"OR'd usage flags came through as %s, expected STORAGE | INTERNAL" % hidden_usage)

	# A signal's arguments carry their declared types, not just names: an untyped argument is
	# a Variant, which tells the editor and GDScript nothing.
	var signal_args: Array = []
	for sig in virtual_node.get_signal_list():
		if sig.name == "target_changed":
			signal_args = sig.args
	check(signal_args.size() == 2,
		"the signal declares %s arguments, expected 2" % signal_args.size())
	if signal_args.size() == 2:
		check(signal_args[0].type == TYPE_OBJECT,
			"the object argument is declared as type %s" % signal_args[0].type)
		check(signal_args[0].class_name == "Node",
			"the object argument declares class %s, expected Node" % signal_args[0].class_name)
		check(signal_args[1].type == TYPE_STRING,
			"the string argument is declared as type %s" % signal_args[1].type)

	# Properties beyond numbers and strings. A Vector2 property is what a class exports most
	# often, and an object property has to tell the engine which class it holds.
	virtual_node.offset = Vector2(3, 4)
	check(virtual_node.offset == Vector2(3, 4),
		"a Vector2 property did not round-trip, got %s" % virtual_node.offset)

	var peer_node: Object = ClassDB.instantiate("RustTestNode")
	virtual_node.target = peer_node
	check(virtual_node.target == peer_node, "an object property did not round-trip")

	# The engine must know the property's class, or the inspector shows an untyped slot.
	var found_class := ""
	var offset_type := -1
	for p in virtual_node.get_property_list():
		if p.name == "target":
			found_class = p.class_name
		elif p.name == "offset":
			offset_type = p.type
	check(offset_type == TYPE_VECTOR2,
		"the Vector2 property is declared as type %s" % offset_type)
	check(found_class == "Node",
		"the object property declares class %s, expected Node" % found_class)
	virtual_node.target = null
	peer_node.free()

	# An object as a return value, and objects through Array and Dictionary. All three are the
	# same Variant conversion seen from different sides.
	var made: Object = virtual_node.make_node2d()
	check(made != null, "a Rust method could not return an object")
	if made != null:
		check(made.get_class() == "Node2D", "returned object is a %s" % made.get_class())
		check(made.name == "MadeInRust", "returned object lost its name: %s" % made.name)
		made.free()

	# 1 = out of an Array, 2 = out of a Dictionary, 4 = a wrong class is still refused there.
	var container_result: int = virtual_node.objects_through_containers()
	check(container_result & 1 != 0, "an object did not survive an Array")
	check(container_result & 2 != 0, "an object did not survive a Dictionary")
	check(container_result & 4 != 0, "a container handed out an object as the wrong class")

	# A Variant records that it holds an object, not which class. Converting one back into a
	# Gd<Node2D> must check: a Label is a Node but not a Node2D.
	var label: Object = ClassDB.instantiate("Label")
	var accepted := false
	if virtual_node.has_method("takes_node2d"):
		accepted = virtual_node.callv("takes_node2d", [label]) == 1
	check(not accepted, "a Label was accepted where a Node2D was required")
	label.free()
	peer.free()

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
	check(not ClassDB.class_exists("RustMismatchedBase"),
		"a class whose BASE_NAME and Base type disagree was registered")
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

	# A wrongly typed argument is reported through the engine's call-error channel, which
	# GDScript turns into an error -- calling it here would abort this test, so the check
	# lives on the Rust side where the shim's answer is visible.
	check(n.wrong_argument_is_reported(), "a mistyped argument was not reported as one")

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

func test_utility_functions() -> void:
	# Godot's global functions -- the ones GDScript calls without a receiver. Every check here
	# compares against GDScript performing the same call, so "returned something" is never
	# mistaken for "returned the right thing".
	var n: Object = ClassDB.instantiate("RustTestNode")

	# The engine has one random number generator, and these bindings must draw from it. Seeding
	# it here and drawing from Rust has to give exactly what GDScript draws from the same seed;
	# a binding using a generator of its own would answer with a number too, just never this one.
	seed(42)
	var from_rust: int = n.util_randi()
	seed(42)
	var from_gdscript: int = randi()
	check(from_rust == from_gdscript,
		"randi() gave %d through Rust and %d in GDScript from the same seed; the two are not "
			% [from_rust, from_gdscript] + "sharing the engine's generator")

	# The same generator, seeded from the Rust side this time.
	n.util_seed(42)
	var after_rust_seed: int = randi()
	check(after_rust_seed == from_gdscript,
		"seed() called from Rust then randi() in GDScript gave %d, expected the same %d"
			% [after_rust_seed, from_gdscript])

	# str/max/min are variadic *and* return a value. That combination decides whether the engine
	# is handed a return slot: these three write to it unconditionally, while the equally
	# variadic print() never touches one.
	var joined: String = n.util_str(1, "x")
	check(joined == str(1, "x"),
		"str(1, \"x\") gave %s through Rust, GDScript gives %s" % [joined, str(1, "x")])
	var largest = n.util_max(3, 9, -1)
	check(largest == 9, "max(3, 9, -1) gave %s through Rust, expected 9" % [largest])
	var smallest = n.util_min(3, 9, -1)
	check(smallest == -1, "min(3, 9, -1) gave %s through Rust, expected -1" % [smallest])

	# print() with no arguments at all. Godot's API description gives print a named argument
	# alongside its variadic flag; keeping that as a Rust parameter would have made this call
	# impossible to write.
	check(n.util_print(), "print() with no arguments did not return")

	# type_convert, with the tag passed in from here so both sides are known to be naming the
	# same type rather than each using its own numbering.
	var converted = n.util_type_convert("42", TYPE_INT)
	check(converted == type_convert("42", TYPE_INT),
		"type_convert(\"42\", TYPE_INT) gave %s through Rust, GDScript gives %s"
			% [converted, type_convert("42", TYPE_INT)])

	# The two serialisation round trips, text and binary, over a nested value.
	var value := {"answer": 42, "list": [1, 2.5, "three"]}
	var trip: Array = n.util_var_roundtrip(value)
	check(trip.size() == 4, "util_var_roundtrip returned %d entries, expected 4" % trip.size())
	if trip.size() == 4:
		check(trip[0] == var_to_str(value),
			"var_to_str gave %s through Rust, GDScript gives %s" % [trip[0], var_to_str(value)])
		check(trip[1] == value,
			"str_to_var did not restore the value, got %s" % [trip[1]])
		check(trip[2] == var_to_bytes(value).size(),
			"var_to_bytes produced %s bytes through Rust, GDScript produces %d"
				% [trip[2], var_to_bytes(value).size()])
		check(trip[3] == value,
			"bytes_to_var did not restore the value, got %s" % [trip[3]])

	# Semantics a hand-written Rust equivalent would quietly get wrong.
	var sem: Array = n.util_semantics()
	check(sem.size() == 5, "util_semantics returned %d entries, expected 5" % sem.size())
	if sem.size() == 5:
		check(sem[0] == posmod(-5, 3),
			"posmod(-5, 3) gave %s through Rust, GDScript gives %s" % [sem[0], posmod(-5, 3)])
		check(sem[4] == -2, "Rust's own -5 %% 3 came back as %s, expected -2" % [sem[4]])
		# If these ever agreed, posmod would be indistinguishable from Rust's remainder and the
		# check above would prove nothing.
		check(sem[0] != sem[4],
			"posmod and Rust's own remainder both gave %s, so this comparison proves nothing"
				% [sem[0]])
		check(is_equal_approx(sem[1], lerp_angle(0.0, 3.0, 0.25)),
			"lerp_angle(0, 3, 0.25) gave %s through Rust, GDScript gives %s"
				% [sem[1], lerp_angle(0.0, 3.0, 0.25)])
		check(is_equal_approx(sem[2], snapped(0.37, 0.1)),
			"snapped(0.37, 0.1) gave %s through Rust, GDScript gives %s"
				% [sem[2], snapped(0.37, 0.1)])
		check(is_equal_approx(sem[3], pingpong(5.0, 3.0)),
			"pingpong(5, 3) gave %s through Rust, GDScript gives %s"
				% [sem[3], pingpong(5.0, 3.0)])

	# is_instance_valid has to notice the object is gone. The Variant still names it, so a check
	# that only looked for nil would answer true both times.
	check(n.util_is_instance_valid() == "true,false",
		"is_instance_valid before and after free() gave %s, expected true,false"
			% n.util_is_instance_valid())

	n.free()
	done("utility_functions")

func test_packed_arrays() -> void:
	# Seven of the ten Packed* types had no coverage at all. Each is built in Rust and read back
	# here element by element: they store their elements as raw memory, so a wrong element width
	# scrambles values rather than failing loudly.
	var n: Object = ClassDB.instantiate("RustTestNode")
	var arrays: Array = n.make_packed_arrays()
	check(arrays.size() == 7, "make_packed_arrays returned %d entries, expected 7" % arrays.size())
	if arrays.size() != 7:
		n.free()
		return

	var colors = arrays[0]
	check(typeof(colors) == TYPE_PACKED_COLOR_ARRAY,
		"the colour array came back as type %d, expected TYPE_PACKED_COLOR_ARRAY" % typeof(colors))
	check(colors.size() == 2, "PackedColorArray has %d elements, expected 2" % colors.size())
	check(colors[0].is_equal_approx(Color(0.25, 0.5, 0.75, 1.0)),
		"PackedColorArray[0] is %s, expected (0.25, 0.5, 0.75, 1)" % [colors[0]])
	check(colors[1].is_equal_approx(Color(1.0, 0.0, 0.5, 0.25)),
		"PackedColorArray[1] is %s, expected (1, 0, 0.5, 0.25)" % [colors[1]])

	var floats = arrays[1]
	check(typeof(floats) == TYPE_PACKED_FLOAT64_ARRAY,
		"the float array came back as type %d, expected TYPE_PACKED_FLOAT64_ARRAY" % typeof(floats))
	check(floats.size() == 3, "PackedFloat64Array has %d elements, expected 3" % floats.size())
	check(floats[0] == 1.5 and floats[1] == -2.25,
		"PackedFloat64Array holds %s, expected 1.5 and -2.25" % [floats])
	# No 32-bit float can hold this, so a narrowed element type shows up as inf here.
	check(floats[2] == 1e300, "PackedFloat64Array[2] is %s, expected 1e+300" % [floats[2]])

	var ints32 = arrays[2]
	check(typeof(ints32) == TYPE_PACKED_INT32_ARRAY,
		"the int32 array came back as type %d, expected TYPE_PACKED_INT32_ARRAY" % typeof(ints32))
	check(ints32.size() == 2, "PackedInt32Array has %d elements, expected 2" % ints32.size())
	check(ints32[0] == 7 and ints32[1] == -2147483648,
		"PackedInt32Array holds %s, expected 7 and -2147483648" % [ints32])

	var ints64 = arrays[3]
	check(typeof(ints64) == TYPE_PACKED_INT64_ARRAY,
		"the int64 array came back as type %d, expected TYPE_PACKED_INT64_ARRAY" % typeof(ints64))
	check(ints64.size() == 2, "PackedInt64Array has %d elements, expected 2" % ints64.size())
	# The second value needs all 64 bits, so a narrowed element type cannot round-trip it.
	check(ints64[0] == -9 and ints64[1] == 9223372036854775807,
		"PackedInt64Array holds %s, expected -9 and i64::MAX" % [ints64])

	var v2 = arrays[4]
	check(typeof(v2) == TYPE_PACKED_VECTOR2_ARRAY,
		"the Vector2 array came back as type %d, expected TYPE_PACKED_VECTOR2_ARRAY" % typeof(v2))
	check(v2.size() == 2, "PackedVector2Array has %d elements, expected 2" % v2.size())
	check(v2[0] == Vector2(1, 2) and v2[1] == Vector2(-3, 4.5),
		"PackedVector2Array holds %s, expected (1, 2) and (-3, 4.5)" % [v2])

	var v3 = arrays[5]
	check(typeof(v3) == TYPE_PACKED_VECTOR3_ARRAY,
		"the Vector3 array came back as type %d, expected TYPE_PACKED_VECTOR3_ARRAY" % typeof(v3))
	check(v3.size() == 2, "PackedVector3Array has %d elements, expected 2" % v3.size())
	check(v3[0] == Vector3(1, 2, 3) and v3[1] == Vector3(-4, 5.5, -6),
		"PackedVector3Array holds %s, expected (1, 2, 3) and (-4, 5.5, -6)" % [v3])

	var v4 = arrays[6]
	check(typeof(v4) == TYPE_PACKED_VECTOR4_ARRAY,
		"the Vector4 array came back as type %d, expected TYPE_PACKED_VECTOR4_ARRAY" % typeof(v4))
	check(v4.size() == 2, "PackedVector4Array has %d elements, expected 2" % v4.size())
	check(v4[0] == Vector4(1, 2, 3, 4) and v4[1] == Vector4(-5, 6.5, -7, 8),
		"PackedVector4Array holds %s, expected (1, 2, 3, 4) and (-5, 6.5, -7, 8)" % [v4])

	n.free()
	done("packed_arrays")

func test_math_types() -> void:
	# The flat math builtins nothing had exercised. They cross the boundary as raw memory, so
	# every field is given a distinct value: a wrong field order or element width shows up as
	# scrambled numbers rather than as a crash.
	var n: Object = ClassDB.instantiate("RustTestNode")
	var v: Dictionary = n.make_math_values()

	check(v["vector2i"] == Vector2i(1, -2),
		"Vector2i came back as %s, expected (1, -2)" % [v["vector2i"]])
	check(v["vector3i"] == Vector3i(3, -4, 5),
		"Vector3i came back as %s, expected (3, -4, 5)" % [v["vector3i"]])
	check(v["vector4"] == Vector4(1.5, -2.5, 3.5, -4.5),
		"Vector4 came back as %s, expected (1.5, -2.5, 3.5, -4.5)" % [v["vector4"]])
	check(v["rect2"] == Rect2(Vector2(1.5, 2.5), Vector2(3.5, 4.5)),
		"Rect2 came back as %s, expected position (1.5, 2.5) size (3.5, 4.5)" % [v["rect2"]])
	check(v["rect2i"] == Rect2i(Vector2i(5, 6), Vector2i(7, 8)),
		"Rect2i came back as %s, expected position (5, 6) size (7, 8)" % [v["rect2i"]])
	check(v["quaternion"].is_equal_approx(Quaternion(0.5, -0.5, 0.5, 0.5)),
		"Quaternion came back as %s, expected (0.5, -0.5, 0.5, 0.5)" % [v["quaternion"]])
	check(v["plane"] == Plane(Vector3(0, 0, 1), 5.5),
		"Plane came back as %s, expected normal (0, 0, 1) d 5.5" % [v["plane"]])
	check(v["projection"] == Projection(
			Vector4(1, 2, 3, 4), Vector4(5, 6, 7, 8),
			Vector4(9, 10, 11, 12), Vector4(13, 14, 15, 16)),
		"Projection came back as %s, expected its columns counting 1 to 16" % [v["projection"]])

	# Basis is the one type whose Rust fields and GDScript members are not the same thing: the
	# engine stores three rows, which is what the Rust struct's x, y and z are, while GDScript's
	# .x, .y and .z are the *columns*. Rust filling rows (1,2,3), (4,5,6), (7,8,9) therefore has
	# to read back here as the transpose -- any other answer means the memory is wrong.
	check(v["basis"] == Basis(Vector3(1, 4, 7), Vector3(2, 5, 8), Vector3(3, 6, 9)),
		"a Basis whose Rust rows are (1,2,3), (4,5,6), (7,8,9) came back as %s, expected its "
			% [v["basis"]] + "transpose, since GDScript names the columns x/y/z")

	# A RID the engine issued, rather than one invented here: a RID is meaningless outside the
	# server that owns it. Rust reads the handle out of it and hands back the plain number, so a
	# layout mistake on the way in cannot be undone by the same mistake on the way out.
	var rid := get_viewport().get_viewport_rid()
	check(rid.is_valid(), "the viewport RID is invalid, so this check proves nothing")
	check(n.rid_id(rid) == rid.get_id(),
		"a RID read in Rust has id %d, GDScript sees %d" % [n.rid_id(rid), rid.get_id()])

	n.free()
	done("math_types")

func test_builtin_constants() -> void:
	# The constants are generated from the API dump, which spells every value as a flat list of
	# scalars in memory order. Comparing each against GDScript's own constant of the same name is
	# what catches a scalar that landed in the wrong field: a swapped pair still compiles and is
	# still the right size. The nested types are all covered for that reason.
	var n: Object = ClassDB.instantiate("RustTestNode")
	var got: Dictionary = n.make_constant_values()

	var expected := {
		"vector2_left": Vector2.LEFT,
		"vector2_inf": Vector2.INF,
		"vector2i_down": Vector2i.DOWN,
		"vector3_forward": Vector3.FORWARD,
		"vector3i_min": Vector3i.MIN,
		"vector4_one": Vector4.ONE,
		"vector4i_max": Vector4i.MAX,
		"color_alice_blue": Color.ALICE_BLUE,
		"color_red": Color.RED,
		"quaternion_identity": Quaternion.IDENTITY,
		"plane_yz": Plane.PLANE_YZ,
		"basis_flip_y": Basis.FLIP_Y,
		"transform2d_flip_x": Transform2D.FLIP_X,
		"transform3d_flip_z": Transform3D.FLIP_Z,
		"projection_identity": Projection.IDENTITY,
	}

	for key: String in expected:
		check(got.has(key), "the Rust side did not return a constant named '%s'" % key)
		if not got.has(key):
			continue
		check(got[key] == expected[key],
			"%s is %s in Rust, GDScript has %s" % [key, got[key], expected[key]])

	# Vector4i is new to the bindings, so the round trip is read field by field rather than
	# compared whole: that says which field is wrong, not merely that one is.
	var v: Vector4i = n.make_vector4i()
	check(v.x == 1, "Vector4i.x came back as %d, expected 1" % v.x)
	check(v.y == -2, "Vector4i.y came back as %d, expected -2" % v.y)
	check(v.z == 3, "Vector4i.z came back as %d, expected 3" % v.z)
	check(v.w == -4, "Vector4i.w came back as %d, expected -4" % v.w)

	n.free()
	done("builtin_constants")

func test_builtin_operators() -> void:
	# The builtins whose memory the engine owns get Rust's operator traits from the engine's own
	# evaluators. Rust evaluates each pair and GDScript evaluates the same expressions itself, so
	# the expected answer is always the engine's: StringName's `<` need not be alphabetical, and
	# whether a dictionary's equality cares about insertion order is the engine's call too.
	var n: Object = ClassDB.instantiate("RustTestNode")
	var typed_a: Array[int] = [1, 2]
	var typed_b: Array[int] = [1, 2]
	var typed_c: Array[int] = [1, 3]

	# Everything before the last underscore names the type; the rest only tells cases apart.
	# Equal operands are built separately, so they are equal values rather than one shared buffer.
	var pairs := {
		"string_ab": ["apple", "banana"],
		"string_same": ["apple", "app" + "le"],
		"string_ba": ["pear", "apple"],
		"string_name_ab": [&"apple", &"banana"],
		"string_name_same": [&"apple", StringName("app" + "le")],
		"string_name_ba": [&"pear", &"apple"],
		"array_ab": [[1, 2], [1, 3]],
		"array_same": [[1, "x"], [1, "x"]],
		"array_ba": [[2], [1, 5]],
		"node_path_same": [NodePath("a/b"), NodePath("a/" + "b")],
		"node_path_different": [NodePath("a/b"), NodePath("a/c")],
		"callable_same": [Callable(n, "make_array"), Callable(n, "make_array")],
		"callable_different": [Callable(n, "make_array"), Callable(n, "sum_array")],
		"signal_same": [Signal(n, "counter_changed"), Signal(n, "counter_changed")],
		"signal_different": [Signal(n, "counter_changed"), Signal(self, "ready")],
		"dictionary_reordered": [{"a": 1, "b": 2}, {"b": 2, "a": 1}],
		"dictionary_different": [{"a": 1}, {"a": 2}],
		"packed_int32_same": [PackedInt32Array([1, 2]), PackedInt32Array([1, 2])],
		"packed_int32_different": [PackedInt32Array([1, 2]), PackedInt32Array([3])],
		"packed_string_different": [PackedStringArray(["a"]), PackedStringArray(["b", "c"])],
		"packed_vector2_same": [PackedVector2Array([Vector2(1, 2)]),
			PackedVector2Array([Vector2(1, 2)])],
		"typed_array_same": [typed_a, typed_b],
		"typed_array_different": [typed_a, typed_c],
		"variant_same": [1, 1.0],
		"variant_different": [1, 2],
	}

	var got: Dictionary = n.builtin_operators(pairs)
	for key: String in pairs:
		var a = pairs[key][0]
		var b = pairs[key][1]
		var kind := key.substr(0, key.rfind("_"))
		var expected := [a == b, a != b]
		if kind in ["string", "string_name", "array"]:
			expected.append_array([a < b, a > b, a <= b, a >= b])
		if kind in ["string", "string_name", "array"] or kind.begins_with("packed_"):
			expected.append(a + b)

		check(got.has(key), "the Rust side returned no result for '%s'" % key)
		if not got.has(key):
			continue
		check(got[key] == expected, "%s: Rust computed %s, GDScript %s" % [key, got[key], expected])

	# StringName + StringName is a String in the API dump, and the Rust result must be one too.
	if got.has("string_name_ab"):
		check(typeof(got["string_name_ab"][6]) == TYPE_STRING,
			"StringName + StringName came back as type %d, expected TYPE_STRING"
				% typeof(got["string_name_ab"][6]))

	# Two equal keys built separately must share one HashMap entry: Hash has to follow the
	# contents, not the handle.
	var entries: Array = n.hash_map_entries()
	check(entries == [1, 1],
		"HashMaps keyed by two equal GStrings and two equal StringNames hold %s entries, expected [1, 1]"
			% [entries])

	n.free()
	done("builtin_operators")

func test_virtual_panic() -> void:
	# A panic inside a virtual leaves Rust by a different route than one inside a #[func]: the
	# engine calls the trampoline directly, and an unwind escaping that frame aborts the process
	# rather than being reported. Reaching the checks below is therefore part of the assertion.
	var n: Node = ClassDB.instantiate("RustTestNode")
	add_child(n)

	# Unarmed first. Without this the armed run below could not be told apart from an engine
	# that never calls the virtual at all -- which is how a test ends up asserting nothing.
	push_key_event()
	check(n.key_input_calls() == 1,
		"_unhandled_key_input fired %d times, expected 1; the panic path below would not have "
			% n.key_input_calls() + "been exercised at all")

	n.arm_virtual_panic()
	print("  (the next error is expected: a deliberate panic inside a virtual method)")
	push_key_event()

	check(n.key_input_calls() == 2,
		"the armed virtual was entered %d times, expected 2" % n.key_input_calls())
	# Containing the panic is only worth doing if the object survives it.
	check(n.echo_int(5) == 5, "the object was unusable after a panic inside a virtual")
	check(n.bump() == 1, "instance state was lost after a panic inside a virtual")

	remove_child(n)
	n.free()
	done("virtual_panic")

func push_key_event() -> void:
	var ev := InputEventKey.new()
	ev.keycode = KEY_A
	ev.pressed = true
	get_viewport().push_input(ev)

func test_init_panic() -> void:
	# A panic in `init` or `on_base_ready` makes instantiation fail -- but the engine built the
	# base object before calling either, and does nothing with the null it gets back. Unless the
	# extension destroys that object itself, every failed `new()` leaks one native object.
	var probe: Object = ClassDB.instantiate("RustTestNode")
	var before := Performance.get_monitor(Performance.OBJECT_COUNT)

	probe.arm_resource_init_panic()
	print("  (the next error is expected: a deliberate panic inside a Rust init)")
	var failed_init: Object = ClassDB.instantiate("RustTestResource")
	check(failed_init == null, "instantiation succeeded even though init panicked")

	probe.arm_resource_base_ready_panic()
	print("  (the next error is expected: a deliberate panic inside on_base_ready)")
	var failed_ready: Object = ClassDB.instantiate("RustTestResource")
	check(failed_ready == null, "instantiation succeeded even though on_base_ready panicked")

	var after := Performance.get_monitor(Performance.OBJECT_COUNT)
	check(after == before,
		"failed instantiations leaked native objects: the object count went from %s to %s"
			% [before, after])

	# The class must still instantiate normally once the flags have fired.
	var healthy: Object = ClassDB.instantiate("RustTestResource")
	check(healthy != null and healthy.payload() == 7,
		"RustTestResource was unusable after a failed instantiation")
	healthy = null

	probe.free()
	done("init_panic")

func test_drop_panic() -> void:
	# A user's `Drop` runs inside the engine's free_instance callback, which is another
	# `extern "C"` frame -- and Godot frees instances during shutdown as well as during play, so
	# an unwind escaping it takes the process down at the worst possible moment.
	var probe: Object = ClassDB.instantiate("RustTestNode")
	var before: int = probe.resource_free_count()

	var res: Object = ClassDB.instantiate("RustTestResource")
	res.arm_drop_panic()
	print("  (the next error is expected: a deliberate panic inside a Rust Drop)")
	# The last reference goes away here, so the destructor runs -- and panics.
	res = null

	check(probe.resource_free_count() == before + 1,
		"the panicking destructor did not run: the count went from %d to %d"
			% [before, probe.resource_free_count()])

	# The next resource must still be built and freed normally. A boundary that survived the
	# panic but left the engine's bookkeeping broken would show up here rather than above.
	var again: Object = ClassDB.instantiate("RustTestResource")
	check(again != null, "a Rust Resource could not be created after a panic in Drop")
	check(again.payload() == 7,
		"a Rust Resource was unusable after a panic in Drop, payload is %s" % again.payload())
	again = null
	check(probe.resource_free_count() == before + 2,
		"the destructor after the panicking one did not run: the count is %d, expected %d"
			% [probe.resource_free_count(), before + 2])

	probe.free()
	done("drop_panic")

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
