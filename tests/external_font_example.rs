use std::{fs, path::PathBuf, process::Command};

#[test]
fn external_font_profiles_link_as_distinct_flash_resident_objects() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = root.join("examples/external-fonts/Cargo.toml");
    let target = std::env::temp_dir().join(format!("lvgl-external-fonts-{}", std::process::id()));
    let _ = fs::remove_dir_all(&target);

    let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .arg("run")
        .arg("--offline")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--target-dir")
        .arg(&target)
        .env("DEP_LV_CONFIG_PATH", root.join("conf"))
        .env("LVGL_CFLAGS", "LVGL_FONT_TEST_FLAG=23")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "external font example failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let executable = target.join("debug/external-fonts");
    let symbols = Command::new("nm")
        .arg("-g")
        .arg(&executable)
        .output()
        .expect("nm must be installed");
    assert!(symbols.status.success());
    let symbols = String::from_utf8_lossy(&symbols.stdout);
    assert!(symbols.contains("font_montserrat_14_latin"));
    assert!(symbols.contains("font_montserrat_14_cjk"));

    let sections = Command::new("objdump")
        .args(["-t", executable.to_str().unwrap()])
        .output()
        .expect("objdump must be installed");
    assert!(sections.status.success());
    let sections = String::from_utf8_lossy(&sections.stdout);
    for symbol in ["font_montserrat_14_latin", "font_montserrat_14_cjk"] {
        assert!(
            sections.lines().any(|line| {
                (line.contains(".rodata") || line.contains(".data.rel.ro")) && line.contains(symbol)
            }),
            "{symbol} is not in a read-only section:\n{sections}"
        );
    }
    for table in [
        "latin_glyph_bitmap",
        "latin_cmap",
        "cjk_glyph_bitmap",
        "cjk_cmap",
    ] {
        assert!(
            sections
                .lines()
                .any(|line| line.contains(".rodata") && line.contains(table)),
            "{table} is not in a read-only section:\n{sections}"
        );
    }

    fs::remove_dir_all(target).unwrap();
}
