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
    assert!(flags.contains("LVGL_TEST_ABI=7"));
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
