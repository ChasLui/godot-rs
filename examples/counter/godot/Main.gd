extends Node

func _ready() -> void:
	var counter: Object = ClassDB.instantiate("Counter")
	add_child(counter)

	counter.connect("value_changed", _on_value_changed)

	# The `step` property is exported, so it shows up in the Inspector too.
	counter.step = 5
	print("increment -> ", counter.increment())
	print("increment -> ", counter.increment())

	# Runs across real frames without blocking.
	counter.count_over_frames(3)
	await get_tree().create_timer(0.25).timeout
	print("after async counting -> ", counter.get_value())

	get_tree().quit(0)

func _on_value_changed(new_value: int) -> void:
	print("  signal: value is now ", new_value)
