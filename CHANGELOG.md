# Changelog

## Unreleased — Godot 4 rewrite

Complete rewrite from GDNative (Godot 3) to GDExtension (Godot 4). Nothing carried over except
build-script platform handling and a few naming utilities; the ABI is entirely different.

The last Godot 3 release is tagged `gdnative-final-0.11.3`, and its changelog is preserved there.

### Added

- `godot-sys`: bindgen over the vendored `gdextension_interface.h`, plus the interface function
  table resolved through `get_proc_address`
- `godot-codegen` / `godot-bindings`: bindings generated from `extension_api.json` -- every
  non-editor class, and all 114 of Godot's global utility functions (`lerp`, `randi`,
  `type_convert`, `is_instance_valid`, ...) as free functions in `global`, and the builtins'
  own 210 constants (`Vector2::LEFT`, `Color::RED`, `Basis::IDENTITY`, ...) as compile-time
  values rather than engine calls. `Vector4i` exists now too; the dump has always had it, and
  `PackedVector4Array` was already here without the element type it holds
- `godot-core`: `Variant`, builtin types, `Gd<T>` with reference counting, class registration,
  `ptrcall`/varcall, and `Base<T>`, the handle a class uses to act on the object it is attached
  to -- previously a raw `GDExtensionObjectPtr` field every class had to keep and wrap by hand
- `godot-macros`: `#[godot_api]` with `#[func]`, `#[prop]`, `#[signal]`, `#[godot_virtual]`.
  A `#[prop]` with no setter is read-only: the inspector shows it and a write is refused
- `godot-async`: frame-driven executor
- `itest`: integration tests that run inside a real Godot instance

### Fixed

- Panics in user code no longer cross the FFI boundary. Twelve callbacks could still unwind out
  of an `extern "C"` function -- the entry point, the instance and closure destructors, the
  property-list callbacks, the ptrcall argument conversion -- and unwinding out of one aborts the
  editor in practice, taking unsaved work with it.
- Reporting a panic could itself panic, and that one had nowhere left to go. The report reaches
  Godot through the interface table, which engine shutdown tears down before it makes its last
  calls into the extension; it now falls back to stderr rather than aborting.
- A virtual method's object argument no longer frees the object it was given. The argument is
  borrowed -- the engine drops the event once the frame is over -- but the handle wrapped it as
  though it had been handed a reference count, so a reference-counted argument such as an
  `InputEvent` was destroyed the moment `_input` returned, while the engine was still using it.
  Nothing caught it because a headless run has no input device, so the virtual had never once
  been called; the test that now covers it pushes an event of its own.
- A class whose `Base<T>` field names a different class than its `#[godot_api(base = ...)]` is
  refused at registration. The base was only ever a name, so the two could disagree and the class
  would call one class's methods on an object the engine built as another.

- A `#[godot_virtual]` method can take the container builtins. The outgoing direction has had
  every builtin since the ptrcall marshalling was written, but the incoming one stopped at six
  types, so a virtual taking a `PackedStringArray`, a `Callable` or a `TypedArray` did not
  compile.
- `#[func(name = "other")]` no longer compiles to nothing. The argument was dropped along with
  the attribute, leaving the method exported under its Rust name -- the one thing the argument
  was written to change -- with nothing said about it. None of these attributes take arguments,
  and now they say so.

### Removed

- Everything targeting Godot 3: `gdnative*` crates, `bindings-generator`, the Godot 3 examples
  and test project
- ARVR, PluginScript, videodecoder and net bindings, which have no GDExtension counterpart
