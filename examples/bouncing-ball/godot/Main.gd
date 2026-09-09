extends Node2D

# The scene is deliberately thin: it creates the Rust node, listens for its signal, and shows a
# count. All the movement, bouncing and drawing happen in the extension.

var ball: Node2D
var label: Label

func _ready() -> void:
	ball = ClassDB.instantiate("Ball")
	ball.speed = 260.0
	ball.connect("bounced", _on_bounced)
	add_child(ball)

	label = Label.new()
	label.position = Vector2(8, 8)
	label.text = "bounces: 0   (space re-centres)"
	add_child(label)

	# Headless runs have no window to look at, so they finish on their own after a moment.
	if DisplayServer.get_name() == "headless":
		await get_tree().create_timer(1.5).timeout
		_report_headless()

func _on_bounced(count: int) -> void:
	label.text = "bounces: %d   (space re-centres)" % count

func _report_headless() -> void:
	var failures: Array[String] = []
	var count: int = ball.get_bounces()

	# A headless run reports a 64x64 viewport regardless of the window size above, which leaves
	# roughly 32 px of travel. At this speed that is a bounce every 0.15 s horizontally and
	# every 0.21 s vertically, so about 17 in 1.5 s. The range is tied to that speed: a much
	# faster ball crosses the area every frame, and bouncing each time is correct rather than
	# stuck.
	if count < 5:
		failures.append("only %d bounces in 1.5s; the ball is barely moving" % count)
	elif count > 60:
		failures.append("%d bounces in 1.5s; the ball is stuck against a wall" % count)

	# The sharper check: the ball must actually cover the play area. A ball jittering in a
	# corner keeps its bounce count climbing while going nowhere, which the count alone cannot
	# tell apart from real motion.
	var extent: PackedStringArray = str(ball.travelled_extent()).split(",")
	var bounds_early: Rect2 = get_viewport().get_visible_rect()
	# Travel is the viewport minus the ball on both sides; require most of it.
	var expected_w: float = bounds_early.size.x - 2.0 * 16.0
	if extent.size() == 2:
		if float(extent[0]) < expected_w * 0.6:
			failures.append("ball only covered %s px horizontally of about %.0f available"
				% [extent[0], expected_w])
		if float(extent[1]) < expected_w * 0.6:
			failures.append("ball only covered %s px vertically of about %.0f available"
				% [extent[1], expected_w])

	# It must never leave the visible area.
	var bounds: Rect2 = get_viewport().get_visible_rect()
	if not bounds.has_point(ball.position):
		failures.append("ball escaped the viewport at %s (bounds %s)" % [ball.position, bounds])

	if failures.is_empty():
		print("bouncing-ball: OK, %d bounces in 1.5s" % count)
		get_tree().quit(0)
	else:
		for f in failures:
			printerr("bouncing-ball FAIL: ", f)
		get_tree().quit(1)
