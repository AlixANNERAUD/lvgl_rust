mod common;

use common::*;
use std::fs;

#[test]
fn dependent_build_script_receives_lvgl_native_configuration() {
    let fixture = scratch_dir("native-interface");
    write(
        fixture.join("Cargo.toml"),
        &dependent_manifest("native-interface-fixture", &["color_depth_16"], true),
    );
    write(
        fixture.join("build.rs"),
        r#"fn main() {
    let paths = std::env::var("DEP_LVGL_RUST_SYS_INCLUDE_PATHS").expect("include paths metadata");
    assert!(paths.contains("lvgl"));
    assert!(paths.contains("conf"));
    let defines = std::env::var("DEP_LVGL_RUST_SYS_DEFINES").expect("defines metadata");
    assert!(defines.contains("LV_CONF_INCLUDE_SIMPLE=1"));
    assert!(defines.contains("LV_COLOR_DEPTH=16"));
    assert!(defines.contains("LV_COLOR_FORMAT_DEFAULT=LV_COLOR_FORMAT_RGB565"));
    assert!(!defines.contains("LVGL_TEST_ABI=7"));
    let flags = std::env::var("DEP_LVGL_RUST_SYS_CFLAGS").expect("C flags metadata");
    assert_eq!(flags, "LVGL_TEST_ABI=7");
    let compiler_flags = std::env::var("DEP_LVGL_RUST_SYS_COMPILER_FLAGS").unwrap();
    match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("riscv32" | "riscv64") => assert!(!compiler_flags.is_empty()),
        Ok("xtensa") => assert_eq!(compiler_flags, "-mlongcalls"),
        _ => assert!(compiler_flags.is_empty()),
    }
    assert_eq!(std::env::var("DEP_LVGL_RUST_SYS_TARGET").unwrap(), std::env::var("TARGET").unwrap());
    let bindings = std::env::var("DEP_LVGL_RUST_SYS_BINDINGS").expect("bindings metadata");
    let bindings = std::path::Path::new(&bindings);
    assert!(bindings.is_absolute() && bindings.ends_with("bindings.rs"));
    assert!(std::fs::read_to_string(bindings).unwrap().contains("pub fn lv_init()"));
    assert_eq!(std::env::var("DEP_LVGL_RUST_SYS_LINKED").unwrap(), "1");
    println!("cargo:warning=LVGL_NATIVE_INTERFACE_OK={paths}|{defines}|{flags}");
}
"#,
    );
    write(fixture.join("src/lib.rs"), "pub fn fixture() {}\n");

    let output = run(
        cargo(
            "build",
            &fixture.join("Cargo.toml"),
            &fixture.join("target"),
        )
        .env("LVGL_CFLAGS", "LVGL_TEST_ABI=7"),
        "fixture build",
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("LVGL_NATIVE_INTERFACE_OK"));
    fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn bindgen_only_build_skips_native_archive() {
    let fixture = scratch_dir("bindgen-only");
    write(
        fixture.join("Cargo.toml"),
        &dependent_manifest("bindgen-only-fixture", &[], true)
            .replace("features = []", "default-features = false"),
    );
    write(
        fixture.join("build.rs"),
        r#"fn main() {
    assert_eq!(std::env::var("DEP_LVGL_RUST_SYS_LINKED").unwrap(), "0");
    assert!(std::path::Path::new(&std::env::var("DEP_LVGL_RUST_SYS_BINDINGS").unwrap()).exists());
}
"#,
    );
    write(
        fixture.join("src/lib.rs"),
        "pub use lvgl_rust_sys::lv_init;\n",
    );

    run(
        &mut cargo(
            "build",
            &fixture.join("Cargo.toml"),
            &fixture.join("target"),
        ),
        "bindgen-only build",
    );
    let out_dir = lvgl_out_dir(&fixture.join("target"));
    assert!(out_dir.join("bindings.rs").exists());
    assert!(!out_dir.join("liblvgl.a").exists());
    fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn bindings_rerun_when_clang_environment_changes() {
    let fixture = scratch_dir("bindgen-environment");
    write(
        fixture.join("Cargo.toml"),
        &dependent_manifest("bindgen-environment-fixture", &[], false)
            .replace("features = []", "default-features = false"),
    );
    write(
        fixture.join("src/lib.rs"),
        "pub use lvgl_rust_sys::lv_init;\n",
    );
    let build = |clang_path: Option<&str>, what: &str| {
        let mut command = cargo(
            "build",
            &fixture.join("Cargo.toml"),
            &fixture.join("target"),
        );
        command.arg("-v").env_remove("CLANG_PATH");
        if let Some(path) = clang_path {
            command.env("CLANG_PATH", path);
        }
        stdout_and_stderr(&run(&mut command, what))
    };

    build(None, "initial build");
    // Read by clang-sys, not by the build script. A path that is not a file
    // makes clang-sys fall back to `PATH`, so the build still succeeds.
    let changed = build(Some("/nonexistent/clang"), "build with CLANG_PATH set");
    assert!(
        !changed.contains("Fresh lvgl-rust-sys"),
        "CLANG_PATH changed but the bindings were not regenerated:\n{changed}"
    );
    let again = build(
        Some("/nonexistent/clang"),
        "build with CLANG_PATH unchanged",
    );
    assert!(
        again.contains("Fresh lvgl-rust-sys"),
        "CLANG_PATH unchanged but the build script ran again:\n{again}"
    );

    // bindgen's build script tracks `BINDGEN_EXTRA_CLANG_ARGS_<TARGET>` for
    // its own target (the host), so a cross build's per-target forms must be
    // declared by the build script itself.
    let run_dir = lvgl_out_dir(&fixture.join("target"))
        .parent()
        .unwrap()
        .to_path_buf();
    // Cargo 1.100 and later: run/stdout; before: output.
    let declarations = fs::read_to_string(run_dir.join("run/stdout"))
        .or_else(|_| fs::read_to_string(run_dir.join("output")))
        .expect("lvgl-rust-sys build script output");
    let per_target = declarations
        .lines()
        .filter(|line| line.starts_with("cargo:rerun-if-env-changed=BINDGEN_EXTRA_CLANG_ARGS_"))
        .count();
    assert_eq!(
        per_target, 2,
        "expected both per-target forms of BINDGEN_EXTRA_CLANG_ARGS:\n{declarations}"
    );
    fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn missing_configuration_is_reported() {
    let fixture = scratch_dir("missing-config");
    write(
        fixture.join("Cargo.toml"),
        &dependent_manifest("missing-config-fixture", &[], false)
            .replace("features = []", "default-features = false"),
    );
    write(fixture.join("src/lib.rs"), "");

    let output = cargo(
        "build",
        &fixture.join("Cargo.toml"),
        &fixture.join("target"),
    )
    .env("DEP_LV_CONFIG_PATH", fixture.join("does-not-exist"))
    .output()
    .unwrap();
    assert!(!output.status.success());
    let log = stdout_and_stderr(&output);
    assert!(log.contains("DEP_LV_CONFIG_PATH"), "{log}");
    fs::remove_dir_all(fixture).unwrap();
}
