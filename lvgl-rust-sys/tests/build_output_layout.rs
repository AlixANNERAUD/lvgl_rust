mod common;

use common::*;
use std::fs;

#[test]
fn finds_bindings_in_both_cargo_build_layouts() {
    for layout in [
        "lvgl-rust-sys-0123456789abcdef",
        "lvgl-rust-sys/0123456789abcdef",
    ] {
        let target = scratch_dir("build-layout");
        let expected = target.join("debug/build").join(layout).join("out");
        write(expected.join("bindings.rs"), "// fixture\n");
        // Cargo 1.100 also puts units other than build scripts here.
        write(target.join("debug/build/other/unit/out/unrelated.rs"), "");
        assert_eq!(lvgl_out_dir(&target), expected);
        fs::remove_dir_all(target).unwrap();
    }
}
