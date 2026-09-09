# Contributing to godot-rs

Contributions are welcome. This is a small, independent binding, so the process is
correspondingly light.

## Before you start

Read the [Scope section of the README](README.md#scope). It states plainly what is implemented
and what is not, so you can tell whether something is a bug or a feature that was never built.

## Making a change

1. Open an issue first for anything substantial, so the design can be discussed before the work.
2. Keep the change focused; unrelated cleanups belong in their own commit.
3. Run the full check before pushing:

   ```bash
   ./check.sh
   ```

   This runs `rustfmt`, `clippy` (warnings are errors), the unit tests, and the integration
   tests against a real Godot 4.7.2. `check.sh` finds the engine via `$GODOT4_BIN`, then
   `/Applications/Godot.app`, then `godot4`/`godot` on the path.

## Tests are the specification

Anything that touches the engine boundary needs a test in [`itest/`](itest). Assertions live on
the GDScript side and decide the process exit code, so a feature is verified through the same
path a user's code takes -- not through a Rust-side mock.

When adding one, check that it can actually fail: break the implementation on purpose and
confirm the test goes red. Several bugs in this repository's history passed a test that only
looked like it was checking something.

## Working with the engine API

The bindings are generated from a vendored dump in `godot-sys/gdextension/`. That dump is the
only source of truth for signatures, hashes and memory layouts -- do not copy them from the
Godot source tree, which may be a different version.

To retarget a different Godot 4.x release:

```bash
cd godot-sys/gdextension
godot --headless --dump-gdextension-interface --dump-extension-api
godot --headless --version > VERSION
```

## Relationship to upstream

This repository began as a fork of [godot-rust/gdnative](https://github.com/godot-rust/gdnative)
and was rewritten for Godot 4; it is not affiliated with the godot-rust project, and issues
about it should be filed here rather than upstream. For their actively maintained Godot 4
binding, see [gdext](https://github.com/godot-rust/gdext).
