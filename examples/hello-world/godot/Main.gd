extends Node

func _ready() -> void:
	var hello: Object = ClassDB.instantiate("HelloWorld")
	add_child(hello)
	print(hello.greet("Godot"))
	get_tree().quit(0)
