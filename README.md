# godot-rust for Godot 4 (GDExtension)

Rust bindings for the [Godot 4](https://godotengine.org) game engine, built on GDExtension.

> [!Important]
> **This is a from-scratch rewrite of a fork, not the official Godot 4 binding.**
>
> This repository began as a fork of [`godot-rust/gdnative`](https://github.com/godot-rust/gdnative),
> which targeted Godot 3 and is no longer maintained. Godot 4 replaced GDNative with GDExtension —
> a different ABI, not a different API — so none of the Godot 3 code survived.
>
> **If you want a production-ready Rust binding for Godot 4, use
> [`godot-rust/gdext`](https://github.com/godot-rust/gdext).** It is actively maintained, far more
> complete, and is what the godot-rust project recommends. This repository exists as a smaller,
> self-contained implementation; see [Scope](#scope) for exactly what it does and does not do.
>
> The last Godot 3 state is preserved at the tag `gdnative-final-0.11.3`.

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

Build and run one:

```bash
cargo build -p counter
cp target/debug/libcounter.dylib examples/counter/godot/lib/
godot --path examples/counter/godot
```

## Scope

Built and covered by the integration tests:

- Class registration, instantiation from GDScript and the editor
- Methods (`#[func]`), properties (`#[prop]`), signals (`#[signal]`)
- Engine hooks: `_ready`, `_process`, `_physics_process`
- `Variant` and the builtins: `GString`, `StringName`, `Vector2/3/4`, `Vector2i/3i`, `Color`, `Rect2/2i`
- `Gd<T>` object handles with automatic reference counting
- Generated bindings for a subset of the engine API, called through `ptrcall`, plus variadic
  methods (`emit_signal`, `call`, `rpc`) through the Variant path
- Frame-driven `async` (`godot-async`)
- Editor-only classes behind the `editor` feature

**Not implemented.** These are absences, not oversights to be discovered later:

- `Array`, `Dictionary`, `Packed*Array`, `Callable`, `Signal`, `Transform2D/3D`, `Basis`,
  `Quaternion`, `Projection`, `Plane`, `AABB`, `RID`, `NodePath`
- Only 106 of the engine's 1036 classes are generated (the closure of a seed set; see
  `godot-codegen/src/lib.rs`). The build prints how many methods were skipped.
- Typed arrays, default arguments, engine enums as Rust types (they surface as `i64`)
- `EditorPlugin` beyond registration; no editor UI integration
- Hot reload: the ABI is wired up (`recreate_instance_func`), but it is untested
- Windows, Android and iOS are not covered by CI
- No API compatibility with the `gdnative` crate — Godot 3 code must be rewritten

## Development

```bash
./check.sh              # fmt, clippy, unit tests, integration tests, editor tests
./check.sh itest        # integration tests only (needs Godot 4.7.2)
```

`check.sh` finds Godot via `$GODOT4_BIN`, then `/Applications/Godot.app`, then `godot4`/`godot`
on the path.

The integration tests in [`itest/`](itest) are the real specification: assertions live on the
GDScript side and decide the process exit code, so every feature is verified through the same
path a user's code takes.

### Known issue: first headless scan crashes

Running `godot --headless --import` (or `--headless --editor`) on a project that registers a
`Node`-derived exposed class **for the first time** crashes Godot 4.7.2 after the scan completes.
The import itself succeeds — the `.godot` directory is written correctly and everything works
afterwards.

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

MIT, as inherited from godot-rust. See [LICENSE.md](LICENSE.md).
