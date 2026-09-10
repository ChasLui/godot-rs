# A GDScript class extending a Rust class: the engine gives the object both a script instance
# and an extension instance, and each has to keep working.
extends RustTestNode

var script_side := 0

func bump_script_side() -> int:
	script_side += 1
	return script_side
