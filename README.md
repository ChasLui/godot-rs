# godot-rs

[![CI](https://github.com/ChasLui/godot-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/ChasLui/godot-rs/actions/workflows/ci.yml)
[![Godot 4.7.2](https://img.shields.io/badge/Godot-4.7.2-478CBF?logo=godotengine&logoColor=white)](https://godotengine.org)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE.md)

Rust bindings for the [Godot 4](https://godotengine.org) game engine, built on GDExtension.

> [!Note]
> **History.** This repository began as a fork of
> [`godot-rust/gdnative`](https://github.com/godot-rust/gdnative), which targeted Godot 3 and is no
> longer maintained. Godot 4 replaced GDNative with GDExtension — a different ABI, not a different
> API — so the Godot 4 support here is a rewrite rather than a port; none of the Godot 3 code
> survived. The last Godot 3 state is preserved at the tag `gdnative-final-0.11.3`.
>
> **Related work.** [`godot-rust/gdext`](https://github.com/godot-rust/gdext) is the godot-rust
> project's own Godot 4 binding, and covers considerably more of the engine API. This repository is
> an independent, deliberately smaller implementation: a compact codebase you can read end to end,
> with every feature verified against a real engine. [Scope](#scope) lists exactly what is and is
> not implemented.

## Supported Godot version

**Godot 4.7.2 only.** The bindings are generated from a vendored API dump
(`godot-sys/gdextension/`), and Godot 4 identifies bound methods by a hash of their signature, so
a different engine version will fail at load time rather than misbehave silently.

To target a different 4.x release, replace the two vendored files and rebuild:

```bash
cd godot-sys/gdextension
godot --headless --dump-gdextension-interface --dump-extension-api
godot --headless --version > VERSION
```

## Quick start

```rust
use godot::prelude::*;

struct MyLibrary;

impl ExtensionLibrary for MyLibrary {
    fn on_level_init(level: InitLevel) {
        // Node types can only be registered once the scene classes exist.
        if level == InitLevel::Scene {
            unsafe { register_class::<Player>(); }
        }
    }

    fn on_level_deinit(level: InitLevel) {
        if level == InitLevel::Scene {
            unsafe { unregister_class::<Player>(); }
        }
    }
}

struct Player {
    health: i64,
}

#[godot_api(base = Node)]
impl Player {
    fn init() -> Self {
        Self { health: 100 }
    }

    /// Exported to GDScript; arguments and return values convert automatically.
    #[func]
    fn take_damage(&mut self, amount: i64) -> i64 {
        self.health -= amount;
        self.health
    }

    /// Shows up in the Inspector, backed by the accessor pair.
    #[prop(set = set_health)]
    fn get_health(&mut self) -> i64 {
        self.health
    }

    #[func]
    fn set_health(&mut self, value: i64) {
        self.health = value;
    }

    /// Declared, not implemented: only the name and argument names are registered.
    #[signal]
    fn died() {}

    /// An engine hook. The macro also tells Godot the class overrides it.
    #[godot_virtual]
    fn ready(&mut self) {
        godot_print("Player ready");

        // Engine methods are called on the handle; `get_name` comes from Node, which this
        // class inherits.
        if let Some(node) = Gd::<godot::classes::Node>::new() {
            node.set_name(&StringName::new("Spawned"));
            unsafe { node.free() };
        }
    }
}

godot_entry!(my_library_init, MyLibrary);
```

The crate must be a `cdylib`:

```toml
[lib]
crate-type = ["cdylib"]
```

And the Godot project needs a `.gdextension` file whose `entry_symbol` matches the name given to
`godot_entry!`:

```ini
[configuration]
entry_symbol = "my_library_init"
compatibility_minimum = 4.7

[libraries]
macos.debug = "res://lib/libmy_library.dylib"
linux.debug.x86_64 = "res://lib/libmy_library.so"
windows.debug.x86_64 = "res://lib/my_library.dll"
```

## Examples

| Example | Shows |
|---|---|
| [`examples/hello-world`](examples/hello-world) | The smallest working extension |
| [`examples/counter`](examples/counter) | Properties, signals, and frame-driven async |
| [`examples/bouncing-ball`](examples/bouncing-ball) | A game loop: physics, custom drawing, input and signals together |
| [`examples/editor-plugin`](examples/editor-plugin) | An `EditorPlugin` in Rust with a dock panel, added without a `plugin.cfg` |

Build and run one:

```bash
cargo build -p counter
cp target/debug/libcounter.dylib examples/counter/godot/lib/
godot --path examples/counter/godot
```

The first run has to scan the project before Godot picks up the `.gdextension`; opening it in the
editor once does that. `check.sh` handles this automatically for the test project.

## The `editor` feature

Editor classes -- `EditorPlugin`, `EditorInterface`, and 80 others -- are generated only when the
`editor` feature is on:

```toml
godot = { version = "0.1", features = ["editor"] }
```

**Leave it off for anything shipped in a game.** An extension that references an editor class
fails to load in an exported project, where those classes do not exist. It belongs on an
extension that only ever runs inside the editor, such as a plugin.

Registering one takes no `plugin.cfg`:

```rust
impl ExtensionLibrary for MyPlugin {
    fn on_level_init(level: InitLevel) {
        // The Editor level is a startup phase, not a mode -- a running game goes through it
        // too, so the hint has to be checked as well.
        if level == InitLevel::Editor && Engine::singleton().is_editor_hint() {
            unsafe {
                register_class::<MyEditorPlugin>();   // must be in ClassDB first
                add_editor_plugin::<MyEditorPlugin>();
            }
        }
    }

    fn on_level_deinit(level: InitLevel) {
        if level == InitLevel::Editor && Engine::singleton().is_editor_hint() {
            unsafe {
                remove_editor_plugin::<MyEditorPlugin>();   // the editor holds an instance
                unregister_class::<MyEditorPlugin>();
            }
        }
    }
}
```

See [`examples/editor-plugin`](examples/editor-plugin).

## Scope

Built and covered by the integration tests:

- Class registration, instantiation from GDScript and the editor
- Methods (`#[func]`), properties (`#[prop]`), signals (`#[signal]`). Properties and signal
  arguments may be any builtin type or an object, and an object declares its class, so the
  inspector shows a typed slot and the connection dialog shows the signal's real shape
- Any engine virtual can be overridden -- `_ready`, `_process`, `_input`, `_enter_tree`,
  `_to_string`, `_notification`, `_get`/`_set`/`_get_property_list`, ... -- by declaring a
  `#[godot_virtual]` method
  whose Rust name is the Godot one without its leading underscore
- `Variant` and the builtins:
  - strings: `GString`, `StringName`, `NodePath`
  - math: `Vector2/3/4`, `Vector2i/3i`, `Color`, `Rect2/2i`, `Transform2D/3D`, `Basis`,
    `Quaternion`, `Plane`, `AABB`, `Projection`, `Rid`
  - containers: `VariantArray`, `Dictionary`, `TypedArray<T>`, and all ten `Packed*Array` types
  - callables: `Callable`, `Signal` -- from a registered method or from a Rust closure, so
    signals can be connected from Rust, not only GDScript
- `Gd<T>` object handles with automatic reference counting, dereferencing to the class so
  methods read as `node.add_child(&child)` and inherited ones need no base-class name
- `Gd::instance_id` / `Gd::from_instance_id` for holding an object across frames: a `Gd` to a
  freed object dangles with no way to test it, an id resolves to `None`
- `registry::rust_instance` reaches the Rust fields behind another object's handle directly,
  rather than calling back out through the engine
- Reference-counted user classes: `#[godot_api(base = Resource)]` and friends are freed when
  the last reference goes, with no `free()` call, and a custom Resource saves and loads through
  `ResourceSaver`/`ResourceLoader` with its `#[prop]` values intact
- GDScript can `extends` a Rust class: the object carries a script instance and an extension
  instance at once, each keeping its own per-instance state
- Generated bindings for every non-editor engine class -- 954 classes, ~14,900 methods --
  called through `ptrcall`, plus variadic methods (`emit_signal`, `call`, `rpc`) through the
  Variant path. The three methods taking a pointer into something the API description does not
  cover are generated `unsafe`.
- Generated methods on the builtin types themselves (`String::find`, `Array::sort`,
  `Vector2::clamp`, ...); the vector maths is kept as inlined Rust rather than an engine call
- Engine enums and bitfields as distinct Rust types, so `connect` returns an `Error` rather
  than a bare integer
- Default arguments: a method with defaults gets a short form taking only the required
  arguments, plus an `_ex` form taking all of them. Object arguments that default to null are
  `Option<&Gd<T>>` in the full form, so the null is expressible rather than only omittable.
- Panics in user code are caught at the FFI boundary and reported through Godot's error output.
  Unwinding out of an `extern "C"` callback is undefined behaviour and aborts in practice, which
  would take the editor down with any unsaved work.
- Frame-driven `async` (`godot-async`)
- Hot reload: swapping the library in the editor rebuilds each instance's Rust state while the
  engine object survives. Needs `reloadable = true` in the `.gdextension`; the engine only
  permits it in an editor build.
- Editor plugins: a Rust class descending from `EditorPlugin` is added with
  `editor::add_editor_plugin`, which takes a class name and nothing else -- no `plugin.cfg`, no
  script file, no `addons/` directory. Editor classes come with the `editor` feature, and the
  plugin can add controls to the editor's docks like any other.

**Not implemented.** These are absences, not oversights to be discovered later:

- A Rust class cannot inherit another Rust class -- `base` must name an engine class. Each
  class owns its Rust state and an object holds exactly one, so the base class's methods would
  read the derived class's fields. Registration refuses it with an error rather than allowing
  the type confusion.
- Editor classes are behind the `editor` feature and off by default, since an extension that
  references them fails to load in an exported project
- Windows, Android and iOS are not covered by CI
- No API compatibility with the `gdnative` crate — Godot 3 code must be rewritten

## Development

```bash
./check.sh              # fmt, clippy, unit tests, integration tests, editor tests
./check.sh itest        # integration tests only (needs Godot 4.7.2)
./check.sh bench        # measure call overhead against GDScript
./check.sh doc          # build docs, failing on broken links, and run doctests
```

The quick-start example above is also the crate-level documentation, and runs as a doctest, so
the two cannot drift apart.

### Performance

`./check.sh bench` builds in release and reports, on this machine:

| | GDScript | Rust |
|---|---|---|
| 200k-iteration loop | 8092 µs | 16 µs |
| call overhead, per call | 0.25 µs | 0.16 µs |
| engine method via `ptrcall` | — | 0.08 µs |
| `StringName::new` | — | 0.73 µs |

Absolute numbers vary by machine; the ratios are the point. Calling into Rust is no more
expensive than a GDScript-to-GDScript call, so the boundary is not what to design around.

Constructing a `StringName` costs several engine calls, because the engine interns it. A name
used every frame -- a signal being emitted, a method called by name -- is worth building once
and keeping rather than rebuilding in the loop.

`check.sh` finds Godot via `$GODOT4_BIN`, then `/Applications/Godot.app`, then `godot4`/`godot`
on the path. Every engine run is time-limited: Godot does not exit when a GDScript fails to
parse, so a typo in a test script would otherwise hang the run rather than report anything.

The integration tests in [`itest/`](itest) are the real specification: assertions live on the
GDScript side and decide the process exit code, so every feature is verified through the same
path a user's code takes.

### Known issue: first headless scan crashes

Running `godot --headless --import` (or `--headless --editor`) on a project that registers a
`Node`-derived exposed class **for the first time** crashes Godot 4.7.2 after the scan completes.
The scan itself succeeds — the `.godot` directory is written correctly and everything works
afterwards, which is why `check.sh` ignores the exit code of that first scan and checks for
`.godot/` instead.

Opening the project in the GUI editor does not crash, so the normal workflow is unaffected. The
crash backtrace is entirely inside the engine, with no frames from this library; it does not
reproduce with `is_exposed = false` or with a non-`Node` base class. Root cause unconfirmed.

## Layout

| Crate | Role |
|---|---|
| `godot-sys` | bindgen over the vendored `gdextension_interface.h`; interface function table |
| `godot-codegen` | Generates bindings from `extension_api.json` |
| `godot-bindings` | Hosts the generated code |
| `godot-core` | `Variant`, builtins, `Gd<T>`, registration, ptrcall |
| `godot-macros` | `#[godot_api]` and its attributes |
| `godot-async` | Frame-driven executor |
| `godot` | The facade users depend on |
| `itest` | Integration tests, run inside a real Godot |

## License

MIT, inherited from the upstream godot-rust project. See [LICENSE.md](LICENSE.md).
