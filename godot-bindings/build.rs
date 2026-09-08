use std::path::PathBuf;

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let api_json = format!("{manifest_dir}/../godot-sys/gdextension/extension_api.json");

    let include_editor = std::env::var("CARGO_FEATURE_EDITOR").is_ok();
    let generated = godot_codegen::generate_with(&api_json, include_editor);

    // Coverage is reported rather than silently capped: the skipped count is the honest
    // measure of how much of the engine API these bindings do not expose yet.
    println!(
        "cargo:warning=godot-bindings: {} classes, {} methods generated, {} methods skipped (unsupported types or virtual){}",
        generated.class_count,
        generated.method_count,
        generated.skipped_methods,
        if include_editor { ", editor classes included" } else { "" }
    );

    std::fs::write(PathBuf::from(&out_dir).join("classes.rs"), generated.code)
        .expect("cannot write generated bindings");

    println!("cargo:rerun-if-changed={api_json}");
}
