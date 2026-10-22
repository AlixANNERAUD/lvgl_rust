use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn write(path: impl AsRef<Path>, contents: &str) {
    let path = path.as_ref();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

#[test]
fn dependent_build_script_receives_lvgl_native_configuration() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture =
        std::env::temp_dir().join(format!("lvgl-native-interface-{}", std::process::id()));
    let _ = fs::remove_dir_all(&fixture);
    write(
        fixture.join("Cargo.toml"),
        &format!(
            "[package]\nname = \"native-interface-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\nbuild = \"build.rs\"\n\n[dependencies]\nlvgl_rust_sys = {{ path = {:?}, features = [\"color_depth_16\"] }}\n",
            root
        ),
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
    let flags = std::env::var("DEP_LVGL_RUST_SYS_CFLAGS").expect("C flags metadata");
    assert!(flags.contains("LVGL_TEST_ABI=7"));
    assert_eq!(std::env::var("DEP_LVGL_RUST_SYS_TARGET").unwrap(), std::env::var("TARGET").unwrap());
    println!("cargo:warning=LVGL_NATIVE_INTERFACE_OK={paths}|{defines}|{flags}");
}
"#,
    );
    write(fixture.join("src/lib.rs"), "pub fn fixture() {}\n");

    let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .arg("build")
        .arg("--manifest-path")
        .arg(fixture.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(fixture.join("target"))
        .env("DEP_LV_CONFIG_PATH", root.join("conf"))
        .env("LVGL_CFLAGS", "LVGL_TEST_ABI=7")
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "fixture build failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("LVGL_NATIVE_INTERFACE_OK"));
    fs::remove_dir_all(fixture).unwrap();
}
