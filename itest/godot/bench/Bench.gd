extends Node

# Measures what crossing into Rust costs, against what the same thing costs in GDScript.
#
# Run with:  ./check.sh bench
#
# The absolute numbers depend on the machine; the ratios are what matter. Build in release --
# a debug build is roughly two orders of magnitude slower at computation and says nothing
# useful about whether the binding itself is efficient.

const ITERS := 200000
const CALLS := 20000

func _gd_noop() -> void:
	pass

func _ready() -> void:
	var n: Object = ClassDB.instantiate("RustTestNode")

	# Computation: the reason to reach for Rust in the first place.
	var t0 := Time.get_ticks_usec()
	var gd_total := 0
	for i in range(ITERS):
		gd_total += i
	var gd_sum := Time.get_ticks_usec() - t0

	t0 = Time.get_ticks_usec()
	var rs_total: int = n.bench_sum(ITERS)
	var rs_sum := Time.get_ticks_usec() - t0

	if gd_total != rs_total:
		printerr("BENCH mismatch: gdscript=%d rust=%d" % [gd_total, rs_total])

	# Call overhead, with a GDScript-to-GDScript call as the reference point.
	t0 = Time.get_ticks_usec()
	for i in range(CALLS):
		_gd_noop()
	var gd_call := Time.get_ticks_usec() - t0

	t0 = Time.get_ticks_usec()
	for i in range(CALLS):
		n.bench_noop()
	var rs_call := Time.get_ticks_usec() - t0

	# What the generated bindings cost, and what a StringName costs -- the latter is worth
	# knowing because calling an engine method by name constructs one.
	t0 = Time.get_ticks_usec()
	n.bench_engine_calls(CALLS)
	var engine_call := Time.get_ticks_usec() - t0

	t0 = Time.get_ticks_usec()
	n.bench_stringname(CALLS)
	var stringname := Time.get_ticks_usec() - t0

	print("== godot-rs benchmarks ==")
	print("  compute %d iterations   gdscript %6dus | rust %6dus | %.0fx faster"
		% [ITERS, gd_sum, rs_sum, float(gd_sum) / maxi(rs_sum, 1)])
	print("  call overhead           gdscript %6.3fus | rust %6.3fus  (per call)"
		% [gd_call / float(CALLS), rs_call / float(CALLS)])
	print("  engine method (ptrcall)                  | rust %6.3fus  (per call)"
		% [engine_call / float(CALLS)])
	print("  StringName::new                          | rust %6.3fus  (per construction)"
		% [stringname / float(CALLS)])
	print("")
	print("  A StringName costs several engine calls, so a name used every frame is worth")
	print("  hoisting out of the loop rather than rebuilding.")

	n.free()
	get_tree().quit(0)
