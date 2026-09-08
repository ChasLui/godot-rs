# Changelog

## Unreleased — Godot 4 rewrite

Complete rewrite from GDNative (Godot 3) to GDExtension (Godot 4). Nothing carried over except
build-script platform handling and a few naming utilities; the ABI is entirely different.

The last Godot 3 release is tagged `gdnative-final-0.11.3`, and its changelog is preserved there.

### Added

- `godot-sys`: bindgen over the vendored `gdextension_interface.h`, plus the interface function
  table resolved through `get_proc_address`
- `godot-codegen` / `godot-bindings`: bindings generated from `extension_api.json`
- `godot-core`: `Variant`, builtin types, `Gd<T>` with reference counting, class registration,
  `ptrcall`/varcall
- `godot-macros`: `#[godot_api]` with `#[func]`, `#[prop]`, `#[signal]`, `#[godot_virtual]`
- `godot-async`: frame-driven executor
- `itest`: integration tests that run inside a real Godot instance

### Removed

- Everything targeting Godot 3: `gdnative*` crates, `bindings-generator`, the Godot 3 examples
  and test project
- ARVR, PluginScript, videodecoder and net bindings, which have no GDExtension counterpart
