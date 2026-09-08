@tool
extends EditorPlugin

# Runs inside the editor, where the game-mode test cannot reach: it verifies the other half of
# the init-level gate, namely that an editor-only class *is* registered here.

func _enter_tree() -> void:
	var failures: Array[String] = []

	if not Engine.is_editor_hint():
		failures.append("EditorPlugin ran without the editor hint set")

	if not ClassDB.class_exists("RustEditorOnlyNode"):
		failures.append("editor-only class was not registered in the editor")
	else:
		var n: Object = ClassDB.instantiate("RustEditorOnlyNode")
		if n == null or n.marker() != 1:
			failures.append("editor-only class did not work")
		elif n != null:
			n.free()

	if failures.is_empty():
		print("itest-editor: OK")
	else:
		for f in failures:
			printerr("itest-editor FAIL: ", f)
