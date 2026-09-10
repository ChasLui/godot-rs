# Control for the property-hint check: GDScript's own @export_range, read back the same way.
extends Node

@export_range(0, 100, 0.5) var ranged: float = 1.0
