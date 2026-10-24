mod common;

use common::*;
use std::fs;

#[test]
fn external_font_profiles_link_as_distinct_flash_resident_objects() {
    let target = scratch_dir("external-fonts");
    run(
        cargo(
            "run",
            &crate_root().join("examples/external-fonts/Cargo.toml"),
            &target,
        )
        .env("LVGL_CFLAGS", "LVGL_FONT_TEST_FLAG=23"),
        "external font example",
    );

    let executable = target.join("debug/external-fonts");
    let symbols = binutils("nm", &["-g"], &executable);
    assert!(symbols.contains("font_montserrat_14_latin"));
    assert!(symbols.contains("font_montserrat_14_cjk"));

    let sections = binutils("objdump", &["-t"], &executable);
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
