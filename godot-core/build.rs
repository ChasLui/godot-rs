use std::path::PathBuf;

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let api_json = format!("{manifest_dir}/../godot-sys/gdextension/extension_api.json");

    // Builtin methods attach to this crate's own structs, so they are generated here rather
    // than in godot-bindings. No cycle: godot-codegen depends on neither crate.
    let generated = godot_codegen::builtins::generate_builtin_methods(&api_json);

    println!(
        "cargo:warning=godot-core: {} builtin types, {} methods generated, {} skipped",
        generated.type_count, generated.method_count, generated.skipped
    );

    std::fs::write(
        PathBuf::from(&out_dir).join("builtin_methods.rs"),
        generated.code,
    )
    .expect("cannot write generated builtin methods");

    println!("cargo:rerun-if-changed={api_json}");
}
