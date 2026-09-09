use std::env;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = env::var("OUT_DIR").unwrap();
    let header = format!("{manifest_dir}/gdextension/gdextension_interface.h");
    let api_json = format!("{manifest_dir}/gdextension/extension_api.json");

    header_binding::generate(&header, &out_dir);
    interface_table::generate(&header, &out_dir);
    builtin_sizes::generate(&api_json, &out_dir);

    println!("cargo:rerun-if-changed={header}");
    println!("cargo:rerun-if-changed={api_json}");
}

/// Emits the memory layout of Godot's builtin (Variant) types as constants.
///
/// The layout depends on the engine's build configuration -- `real_t` precision crossed with
/// pointer width -- so the right one of the four tables in `extension_api.json` is selected for
/// the current target. These sizes must never be hand-written: they are the ground truth for
/// every opaque builtin wrapper in `godot-core`.
mod builtin_sizes {
    use quote::{format_ident, quote};
    use std::fs::File;
    use std::io::Write as _;
    use std::path::PathBuf;

    #[derive(serde::Deserialize)]
    struct Api {
        builtin_class_sizes: Vec<SizeTable>,
        classes: Vec<Class>,
        utility_functions: Vec<UtilityFunction>,
    }

    #[derive(serde::Deserialize)]
    struct UtilityFunction {
        name: String,
        hash: i64,
    }

    #[derive(serde::Deserialize)]
    struct Class {
        name: String,
        #[serde(default)]
        methods: Vec<Method>,
    }

    #[derive(serde::Deserialize)]
    struct Method {
        name: String,
        #[serde(default)]
        hash: Option<i64>,
    }

    #[derive(serde::Deserialize)]
    struct SizeTable {
        build_configuration: String,
        sizes: Vec<ClassSize>,
    }

    #[derive(serde::Deserialize)]
    struct ClassSize {
        name: String,
        size: usize,
    }

    /// Hashes of `RefCounted`'s reference-counting methods, taken from the API dump.
    fn refcounted_hashes(api: &Api) -> proc_macro2::TokenStream {
        let class = api
            .classes
            .iter()
            .find(|c| c.name == "RefCounted")
            .expect("RefCounted missing from extension_api.json");

        let mut out = proc_macro2::TokenStream::new();
        for wanted in ["init_ref", "reference", "unreference"] {
            let method = class
                .methods
                .iter()
                .find(|m| m.name == wanted)
                .unwrap_or_else(|| panic!("RefCounted::{wanted} missing from extension_api.json"));
            let hash = method
                .hash
                .unwrap_or_else(|| panic!("RefCounted::{wanted} has no hash"));
            let ident = format_ident!("REFCOUNTED_{}", wanted.to_uppercase());
            out.extend(quote! {
                pub const #ident: i64 = #hash;
            });
        }
        out
    }

    /// Hashes of the `ClassDB` methods `godot-core` calls directly.
    ///
    /// Registration asks the engine which virtuals a base class has, so a misspelled
    /// `#[godot_virtual]` is reported rather than silently never invoked.
    fn classdb_hashes(api: &Api) -> proc_macro2::TokenStream {
        let class = api
            .classes
            .iter()
            .find(|c| c.name == "ClassDB")
            .expect("ClassDB missing from extension_api.json");

        let wanted = "class_get_method_list";
        let method = class
            .methods
            .iter()
            .find(|m| m.name == wanted)
            .unwrap_or_else(|| panic!("ClassDB::{wanted} missing from extension_api.json"));
        let hash = method
            .hash
            .unwrap_or_else(|| panic!("ClassDB::{wanted} has no hash"));
        let ident = format_ident!("CLASSDB_{}", wanted.to_uppercase());

        quote! {
            pub const #ident: i64 = #hash;
        }
    }

    /// Hashes of the few utility functions `godot-core` calls directly.
    fn utility_hashes(api: &Api) -> proc_macro2::TokenStream {
        let mut out = proc_macro2::TokenStream::new();
        for wanted in ["print", "printerr", "push_error", "push_warning"] {
            let function = api
                .utility_functions
                .iter()
                .find(|f| f.name == wanted)
                .unwrap_or_else(|| {
                    panic!("utility function {wanted} missing from extension_api.json")
                });
            let ident = format_ident!("UTILITY_{}", wanted.to_uppercase());
            let hash = function.hash;
            out.extend(quote! {
                pub const #ident: i64 = #hash;
            });
        }
        out
    }

    fn build_configuration() -> &'static str {
        let double = std::env::var("CARGO_FEATURE_DOUBLE_PRECISION").is_ok();
        let ptr_width = std::env::var("CARGO_CFG_TARGET_POINTER_WIDTH").unwrap();

        match (double, ptr_width.as_str()) {
            (false, "32") => "float_32",
            (false, "64") => "float_64",
            (true, "32") => "double_32",
            (true, "64") => "double_64",
            (_, other) => panic!("unsupported target pointer width: {other}"),
        }
    }

    pub(crate) fn generate(api_json: &str, out_dir: &str) {
        let src = std::fs::read_to_string(api_json)
            .unwrap_or_else(|e| panic!("cannot read {api_json}: {e}"));
        let api: Api =
            serde_json::from_str(&src).unwrap_or_else(|e| panic!("cannot parse {api_json}: {e}"));

        let config = build_configuration();
        let table = api
            .builtin_class_sizes
            .iter()
            .find(|t| t.build_configuration == config)
            .unwrap_or_else(|| panic!("no builtin_class_sizes for build configuration {config}"));

        let mut consts = proc_macro2::TokenStream::new();
        for entry in &table.sizes {
            // `Variant` is listed alongside the builtin types; keep it, it is the one we need most.
            let ident = format_ident!("SIZE_{}", entry.name.to_uppercase());
            let size = entry.size;
            consts.extend(quote! {
                pub const #ident: usize = #size;
            });
        }

        // `godot-core` needs RefCounted's lifetime methods to implement `Gd<T>`, but it cannot
        // depend on the generated bindings without a dependency cycle. Emitting the hashes here
        // keeps them derived from the API dump rather than hand-copied.
        let refcount_hashes = refcounted_hashes(&api);
        let utility_hashes = utility_hashes(&api);
        let classdb_hashes = classdb_hashes(&api);

        let config_str = config;
        let generated = quote! {
            /// Sizes of Godot's builtin types, for the build configuration this crate targets.
            pub mod builtin_sizes {
                /// The engine build configuration these sizes were selected for.
                pub const BUILD_CONFIGURATION: &str = #config_str;
                #consts
            }

            /// Method hashes needed before the code generator exists.
            pub mod method_hashes {
                #refcount_hashes
                #utility_hashes
                #classdb_hashes
            }
        };

        let path = PathBuf::from(out_dir).join("builtin_sizes.rs");
        let mut file =
            File::create(&path).unwrap_or_else(|e| panic!("cannot create {path:?}: {e}"));
        write!(file, "{generated}").unwrap();
    }
}

/// bindgen over `gdextension_interface.h`.
///
/// The platform adaptation below is carried over verbatim from the Godot 3 `gdnative-sys`
/// build script: it deals with host/target toolchain quirks and is independent of which
/// Godot version the header comes from.
mod header_binding {
    use std::path::{Path, PathBuf};

    fn apple_include_path() -> Result<String, std::io::Error> {
        use std::process::Command;

        let target = std::env::var("TARGET").unwrap();
        let platform = if target.contains("apple-darwin") {
            "macosx"
        } else if target == "x86_64-apple-ios" || target == "aarch64-apple-ios-sim" {
            "iphonesimulator"
        } else if target == "aarch64-apple-ios" {
            "iphoneos"
        } else {
            panic!("not building for macOS or iOS");
        };

        let output = Command::new("xcrun")
            .args(["--sdk", platform, "--show-sdk-path"])
            .output()?
            .stdout;
        let prefix = std::str::from_utf8(&output)
            .expect("invalid output from `xcrun`")
            .trim_end();

        Ok(format!("{prefix}/usr/include"))
    }

    fn add_android_include_paths(mut builder: bindgen::Builder) -> bindgen::Builder {
        let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
        let target_triple = std::env::var("TARGET").unwrap();

        assert_eq!("android", &target_os);

        let android_sdk_root =
            std::env::var("ANDROID_SDK_ROOT").expect("ANDROID_SDK_ROOT must be set");
        let android_sdk_root = Path::new(&android_sdk_root).to_path_buf();

        // Note: cfg!(target_os) / cfg!(target_arch) here refer to the *host* (the build script's
        // own target), not to what we are cross-compiling for. Do not confuse the two.
        // Evaluated at compile time on purpose: it rejects host toolchains the NDK paths below
        // are not valid for.
        #[allow(clippy::assertions_on_constants)]
        {
            assert!(
                cfg!(target_os = "macos") || cfg!(target_arch = "x86_64"),
                "unsupported host architecture: build from x86_64 instead"
            );
        }

        let mut android_ndk_root: Option<PathBuf> = None;

        let android_ndk_folder = Path::join(&android_sdk_root, "ndk");
        if android_ndk_folder.exists() {
            let available_ndk_versions: Vec<_> = std::fs::read_dir(android_ndk_folder.clone())
                .unwrap()
                .map(|dir| dir.unwrap().path())
                .collect();

            if !available_ndk_versions.is_empty() {
                let ndk_version = std::env::var("ANDROID_NDK_VERSION");

                if let Ok(ndk_version) = ndk_version {
                    if available_ndk_versions
                        .iter()
                        .filter_map(|p| p.file_name())
                        .any(|p| p.to_string_lossy() == ndk_version.as_str())
                    {
                        android_ndk_root = Some(Path::join(&android_ndk_folder, ndk_version))
                    } else {
                        panic!(
                            "no available android ndk versions matches {ndk_version}. Available versions: {available_ndk_versions:?}"
                        )
                    }
                } else {
                    println!("cargo:warning=Multiple android ndk versions have been detected.");
                    println!("cargo:warning=You should choose one using the ANDROID_NDK_VERSION environment variable to have reproducible builds.");

                    let ndk_version = available_ndk_versions
                        .iter()
                        .filter_map(|p| p.file_name())
                        .filter_map(|v| semver::Version::parse(v.to_string_lossy().as_ref()).ok())
                        .max()
                        .unwrap();

                    println!("cargo:warning=Automatically chosen version: {ndk_version} (latest)");

                    android_ndk_root =
                        Some(Path::join(&android_ndk_folder, ndk_version.to_string()));
                }
            }
        }

        let android_ndk_bundle_folder = Path::join(&android_sdk_root, "ndk-bundle");
        if android_ndk_root.is_none() && android_ndk_bundle_folder.exists() {
            android_ndk_root = Some(android_ndk_bundle_folder);
        }

        let android_ndk_root = android_ndk_root.expect("Android ndk needs to be installed");

        let host_tag = if cfg!(target_os = "windows") {
            "windows-x86_64"
        } else if cfg!(target_os = "macos") {
            "darwin-x86_64"
        } else if cfg!(target_os = "linux") {
            "linux-x86_64"
        } else {
            panic!("unsupported host OS: build from Windows, macOS, or Linux instead");
        };

        builder = builder.clang_arg("-I").clang_arg(
            Path::join(
                &android_ndk_root,
                format!("toolchains/llvm/prebuilt/{host_tag}/sysroot/usr/include"),
            )
            .to_string_lossy(),
        );

        // Workaround for the NDK using a different naming scheme than the Rust target triple.
        let target_triple = match target_triple.as_str() {
            "armv7-linux-androideabi" => "arm-linux-androideabi",
            other => other,
        };

        builder = builder.clang_arg("-I").clang_arg(
            Path::join(
                &android_ndk_root,
                format!("toolchains/llvm/prebuilt/{host_tag}/sysroot/usr/include/{target_triple}"),
            )
            .to_string_lossy(),
        );
        builder
    }

    pub(crate) fn generate(header: &str, out_dir: &str) {
        #[allow(unused_mut)]
        let mut builder = bindgen::Builder::default()
            .header(header)
            .allowlist_type("GDExtension.*")
            .allowlist_type("GDObjectInstanceID")
            .allowlist_var("GDEXTENSION.*")
            .derive_default(true)
            .ignore_functions()
            .size_t_is_usize(true)
            .layout_tests(false);

        let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
        let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
        let target_vendor = std::env::var("CARGO_CFG_TARGET_VENDOR").unwrap();
        let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap();

        if target_vendor == "apple" {
            if let Ok(include_path) = apple_include_path() {
                builder = builder.clang_arg("-I").clang_arg(include_path);
            }
        }

        // Workaround for https://github.com/rust-lang/rust-bindgen/issues/1211: manually set the
        // target triple to `arm64-apple-ios` in place of `aarch64-apple-ios`.
        if target_arch == "aarch64" && target_os == "ios" {
            if target_env == "sim" {
                builder = builder.clang_arg("--target=arm64-apple-ios-sim");
            } else {
                builder = builder.clang_arg("--target=arm64-apple-ios");
            }
        }

        // Microsoft extensions aren't enabled by default for the `gnu` toolchain on Windows;
        // without them the MSVC headers fail to parse. The architecture macro is needed too,
        // or the build fails with "Unsupported architecture". Not an issue for `msvc`.
        if target_os == "windows" && target_env == "gnu" {
            let arch_macro = match target_arch.as_str() {
                "x86" => "_M_IX86",
                "x86_64" => "_M_X64",
                "arm" => "_M_ARM",
                "aarch64" => "_M_ARM64",
                _ => panic!("architecture {target_arch} not supported on Windows"),
            };

            builder = builder
                .clang_arg("-fms-extensions")
                .clang_arg("-fmsc-version=1300")
                .clang_arg(format!("-D{arch_macro}=100"));
        }

        if target_os == "android" {
            builder = add_android_include_paths(builder);
        }

        let bindings = builder.generate().expect("Unable to generate bindings");

        bindings
            .write_to_file(PathBuf::from(out_dir).join("bindings.rs"))
            .expect("Couldn't write bindings!");
    }
}

/// Generates the `GDExtensionInterface` function table.
///
/// Godot 4.7 has no machine-readable description of the interface (`gdextension_interface.json`
/// only exists from 4.8 on), and bindgen emits only the typedefs, not an enumerable list of
/// names. The header does however document every interface function with a
/// `@name <snake_case_name>` tag immediately followed by its `typedef`, so the pairing is
/// recovered from the header text itself.
mod interface_table {
    use proc_macro2::TokenStream;
    use quote::{format_ident, quote};
    use std::fs::File;
    use std::io::Write as _;
    use std::path::PathBuf;

    /// `(snake_case name used with get_proc_address, bindgen typedef ident)`
    fn parse(header_src: &str) -> Vec<(String, String)> {
        let mut result = Vec::new();
        let mut pending: Option<String> = None;

        for line in header_src.lines() {
            let trimmed = line.trim_start().trim_start_matches('*').trim();

            if let Some(name) = trimmed.strip_prefix("@name ") {
                pending = Some(name.trim().to_string());
                continue;
            }

            // Only a `typedef` of a function pointer closes an open `@name`.
            if let Some(name) = pending.clone() {
                if let Some(typedef) = parse_fn_ptr_typedef(line) {
                    result.push((name, typedef));
                    pending = None;
                }
            }
        }

        assert!(
            !result.is_empty(),
            "no interface functions found in gdextension_interface.h -- has the header format changed?"
        );
        result
    }

    /// Extracts `Foo` from `typedef void (*Foo)(...);`
    fn parse_fn_ptr_typedef(line: &str) -> Option<String> {
        let line = line.trim();
        if !line.starts_with("typedef ") {
            return None;
        }
        let open = line.find("(*")?;
        let close = line[open..].find(')')? + open;
        let ident = &line[open + 2..close];
        if ident.is_empty() || !ident.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return None;
        }
        Some(ident.to_string())
    }

    pub(crate) fn generate(header: &str, out_dir: &str) {
        let src =
            std::fs::read_to_string(header).unwrap_or_else(|e| panic!("cannot read {header}: {e}"));
        let functions = parse(&src);

        let mut fields = TokenStream::new();
        let mut loads = TokenStream::new();

        for (name, typedef) in &functions {
            let field = format_ident!("{}", name);
            let ty = format_ident!("{}", typedef);
            // `get_proc_address` takes a NUL-terminated C string.
            let c_name = format!("{name}\0");

            fields.extend(quote! {
                pub #field: crate::#ty,
            });

            loads.extend(quote! {
                #field: {
                    let ptr = get(#c_name.as_ptr() as *const std::os::raw::c_char);
                    if ptr.is_none() {
                        missing.push(#name);
                    }
                    std::mem::transmute::<crate::GDExtensionInterfaceFunctionPtr, crate::#ty>(ptr)
                },
            });
        }

        let count = functions.len();

        let table = quote! {
            /// All GDExtension interface functions, resolved once at extension entry.
            ///
            /// A field is `None` when the running engine does not provide that function; this is
            /// how an extension can feature-probe a newer or older Godot. `missing` lists those
            /// names for diagnostics.
            #[allow(non_snake_case)]
            pub struct GDExtensionInterface {
                #fields
                pub missing: Vec<&'static str>,
            }

            impl GDExtensionInterface {
                /// Number of interface functions known to the header this crate was built against.
                pub const FUNCTION_COUNT: usize = #count;

                /// # Safety
                /// `get_proc_address` must be the pointer Godot passed to the extension entry point.
                pub unsafe fn load(
                    get_proc_address: crate::GDExtensionInterfaceGetProcAddress,
                ) -> Self {
                    let get = get_proc_address.expect("Godot passed a null get_proc_address");
                    let mut missing = Vec::new();
                    Self {
                        #loads
                        missing,
                    }
                }
            }
        };

        let path = PathBuf::from(out_dir).join("interface.rs");
        let mut file =
            File::create(&path).unwrap_or_else(|e| panic!("cannot create {path:?}: {e}"));
        write!(file, "{table}").unwrap();
    }
}
