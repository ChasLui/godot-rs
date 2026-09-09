@tool
extends EditorPlugin

# Runs inside the editor, where the game-mode suite cannot reach: it covers the editor half of
# the init-level gate, and hot reload, which the engine only permits in an editor build.

var failures: Array[String] = []

func check(condition: bool, message: String) -> void:
	if not condition:
		failures.append(message)

func _enter_tree() -> void:
	test_editor_only_class()
	test_rust_editor_plugin()
	test_hot_reload()

	if failures.is_empty():
		print("itest-editor: OK")
	else:
		for f in failures:
			printerr("itest-editor FAIL: ", f)

func test_editor_only_class() -> void:
	check(Engine.is_editor_hint(), "EditorPlugin ran without the editor hint set")
	check(ClassDB.class_exists("RustEditorOnlyNode"),
		"editor-only class was not registered in the editor")

	if ClassDB.class_exists("RustEditorOnlyNode"):
		var n: Object = ClassDB.instantiate("RustEditorOnlyNode")
		check(n != null and n.marker() == 1, "editor-only class did not work")
		if n != null:
			n.free()

func test_rust_editor_plugin() -> void:
	# A Rust EditorPlugin is added through editor_add_plugin, not through a plugin.cfg, so
	# there is no addon entry to look for -- the class simply exists and the editor holds an
	# instance of it.
	if not ClassDB.class_exists("RustTestPlugin"):
		failures.append("the Rust EditorPlugin class was not registered")
		return

	# The editor owns the plugin instance and never exposes it, so the counters kept on the
	# Rust side are the only way to see whether the engine actually called in.
	var probe: Object = ClassDB.instantiate("RustPluginProbe")
	check(probe.plugin_enter_tree_calls() > 0,
		"_enter_tree never fired: the class registered but was not added to the editor")
	check(probe.plugin_name_calls() > 0,
		"_get_plugin_name was never asked for")
	probe.free()

	# The plugin descends from EditorPlugin, and that is what makes it one.
	check(ClassDB.is_parent_class("RustTestPlugin", "EditorPlugin"),
		"RustTestPlugin does not descend from EditorPlugin")

func test_hot_reload() -> void:
	# Reloading is only enabled in an editor build, and only for an extension whose
	# .gdextension sets `reloadable = true`.
	var loaded := GDExtensionManager.get_loaded_extensions()
	if loaded.is_empty():
		failures.append("no extensions loaded, cannot test reloading")
		return

	var path: String = loaded[0]
	var node: Object = ClassDB.instantiate("RustTestNode")
	var id := node.get_instance_id()

	# Put the instance in a distinctive state, so a rebuilt one is recognisable.
	node.bump()
	node.bump()
	check(node.bump() == 3, "counter did not reach 3 before reloading")

	var status := GDExtensionManager.reload_extension(path)
	check(status == GDExtensionManager.LOAD_STATUS_OK,
		"reload_extension returned %d, expected LOAD_STATUS_OK" % status)

	# The engine object must survive: only the Rust-side state is rebuilt.
	var again: Object = instance_from_id(id)
	check(again != null, "the engine object did not survive the reload")
	if again == null:
		return

	# recreate_instance_func rebuilt the Rust state and set a sentinel only that path reaches,
	# so the counter must continue from 100 rather than from the pre-reload 3.
	var bumped: int = again.bump()
	check(bumped == 101,
		"counter was %d after reloading; 101 means recreate_instance ran, 4 means the old state survived"
			% bumped)

	# And the object is fully usable again, not just alive.
	check(again.echo_int(42) == 42, "the reloaded instance could not answer a method call")

	again.free()
