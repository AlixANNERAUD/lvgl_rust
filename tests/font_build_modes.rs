mod common;

use common::*;
use std::{fs, path::Path, process::Output};

fn build(fixture: &Path, legacy: bool) -> Output {
    let mut command = cargo(
        "build",
        &fixture.join("Cargo.toml"),
        &fixture.join("target"),
    );
    command.arg("-vv").current_dir(fixture).env("PWD", fixture);
    if legacy {
        command
            .env("LVGL_FONTS_DIR", fixture.join("fonts"))
            .env("LVGL_CFLAGS", "LVGL_FONT_FIXTURE_ENABLED");
    }
    let what = if legacy {
        "legacy build"
    } else {
        "default build"
    };
    run(&mut command, what)
}

#[test]
fn default_build_is_font_independent_but_explicit_legacy_mode_compiles_fonts() {
    let fixture = scratch_dir("font-modes");
    write(
        fixture.join("Cargo.toml"),
        &dependent_manifest("font-mode-fixture", &[], false),
    );
    write(fixture.join("src/lib.rs"), "pub fn fixture() {}\n");
    // Only visible to bindgen when `LVGL_CFLAGS` reaches it.
    write(
        fixture.join("fonts/font_fixture.h"),
        "#ifdef LVGL_FONT_FIXTURE_ENABLED\nextern const int lvgl_font_fixture;\n#endif\n",
    );
    write(
        fixture.join("fonts/font_fixture.generated.c"),
        "#include \"font_fixture.h\"\nconst int lvgl_font_fixture = 1;\n",
    );

    let default_output = String::from_utf8_lossy(&build(&fixture, false).stdout).into_owned();
    assert!(
        !default_output.contains(&fixture.join("fonts").display().to_string()),
        "default build watched or consumed the app fonts directory:\n{default_output}"
    );
    let out_dir = lvgl_out_dir(&fixture.join("target"));
    assert!(!fs::read_to_string(out_dir.join("bindings.rs"))
        .unwrap()
        .contains("lvgl_font_fixture"));

    build(&fixture, true);
    let out_dir = lvgl_out_dir(&fixture.join("target"));
    assert!(
        fs::read_to_string(out_dir.join("bindings.rs"))
            .unwrap()
            .contains("lvgl_font_fixture"),
        "legacy font header (guarded by an LVGL_CFLAGS define) missing from bindings"
    );
    assert!(binutils("nm", &["-g"], &out_dir.join("liblvgl.a")).contains("lvgl_font_fixture"));

    fs::remove_dir_all(fixture).unwrap();
}
