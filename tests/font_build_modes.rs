use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn write(path: impl AsRef<Path>, contents: &str) {
    let path = path.as_ref();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn build(fixture: &Path, root: &Path, legacy: bool) -> Output {
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command
        .arg("build")
        .arg("--offline")
        .arg("-vv")
        .arg("--manifest-path")
        .arg(fixture.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(fixture.join("target"))
        .current_dir(fixture)
        .env("PWD", fixture)
        .env("DEP_LV_CONFIG_PATH", root.join("conf"))
        .env_remove("LVGL_FONTS_DIR");
    if legacy {
        command.env("LVGL_FONTS_DIR", fixture.join("fonts"));
    }
    command.output().unwrap()
}

fn build_output(fixture: &Path, root: &Path) -> String {
    let output = build(fixture, root, false);
    assert!(
        output.status.success(),
        "default build failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn lvgl_output_dir(fixture: &Path) -> PathBuf {
    fs::read_dir(fixture.join("target/debug/build"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("out"))
        .find(|path| path.join("liblvgl.a").exists())
        .expect("LVGL archive output directory")
}

#[test]
fn default_build_is_font_independent_but_explicit_legacy_mode_compiles_fonts() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = std::env::temp_dir().join(format!("lvgl-font-modes-{}", std::process::id()));
    let _ = fs::remove_dir_all(&fixture);
    write(
        fixture.join("Cargo.toml"),
        &format!(
            "[package]\nname = \"font-mode-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nlvgl_rust_sys = {{ path = {:?} }}\n",
            root
        ),
    );
    write(fixture.join("src/lib.rs"), "pub fn fixture() {}\n");
    write(
        fixture.join("fonts/font_fixture.h"),
        "extern const int lvgl_font_fixture;\n",
    );
    write(
        fixture.join("fonts/font_fixture.generated.c"),
        "#include \"font_fixture.h\"\nconst int lvgl_font_fixture = 1;\n",
    );

    let default_output = build_output(&fixture, &root);
    assert!(
        !default_output.contains(&fixture.join("fonts").display().to_string()),
        "default build watched or consumed the app fonts directory:\n{default_output}"
    );
    assert!(
        !fs::read_to_string(lvgl_output_dir(&fixture).join("bindings.rs"))
            .unwrap()
            .contains("lvgl_font_fixture")
    );

    let legacy = build(&fixture, &root, true);
    assert!(
        legacy.status.success(),
        "legacy build failed:\n{}\n{}",
        String::from_utf8_lossy(&legacy.stdout),
        String::from_utf8_lossy(&legacy.stderr)
    );
    let output_dir = lvgl_output_dir(&fixture);
    assert!(fs::read_to_string(output_dir.join("bindings.rs"))
        .unwrap()
        .contains("lvgl_font_fixture"));
    let symbols = Command::new("nm")
        .arg("-g")
        .arg(output_dir.join("liblvgl.a"))
        .output()
        .expect("nm must be installed for the native archive test");
    assert!(symbols.status.success());
    assert!(String::from_utf8_lossy(&symbols.stdout).contains("lvgl_font_fixture"));

    fs::remove_dir_all(fixture).unwrap();
}
