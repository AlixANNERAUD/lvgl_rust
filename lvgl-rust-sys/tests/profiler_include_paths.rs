//! Regression tests: a profiler-enabled `lv_conf.h` must build without the
//! `shadcn-theme` feature, whose include paths used to hide missing LVGL roots.

mod common;

use common::*;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// `LV_PROFILER_INCLUDE` spellings found in LVGL 9.6 configurations: the
/// template default (relative to `lvgl/include`) and the LVGL-root relative one.
const PROFILER_INCLUDES: [&str; 2] = [
    "lvgl/debugging/profiler/lv_profiler_builtin.h",
    "include/lvgl/debugging/profiler/lv_profiler_builtin.h",
];

/// Copy of the vendored config with the builtin profiler enabled.
fn profiler_config(dir: &Path, profiler_include: &str) -> PathBuf {
    let conf = fs::read_to_string(crate_root().join("conf/lv_conf.h")).unwrap();
    let mut patched = conf.clone();
    for (from, to) in [
        ("#define LV_USE_PROFILER 0", "#define LV_USE_PROFILER 1"),
        (
            "#define LV_USE_PROFILER_BUILTIN 0",
            "#define LV_USE_PROFILER_BUILTIN 1",
        ),
        (
            "#define LV_PROFILER_INCLUDE \"lvgl/debugging/profiler/lv_profiler_builtin.h\"",
            &format!("#define LV_PROFILER_INCLUDE \"{profiler_include}\""),
        ),
    ] {
        assert_eq!(patched.matches(from).count(), 1, "`{from}` not found once");
        patched = patched.replace(from, to);
    }
    write(dir.join("lv_conf.h"), &patched);
    dir.to_path_buf()
}

/// Manifest depending on this crate with exactly `features` and no defaults,
/// so feature unification cannot pull in `shadcn-theme`.
fn manifest(name: &str, features: &[&str], build_script: bool) -> String {
    let manifest = dependent_manifest(name, features, build_script)
        .replace("features = [", "default-features = false, features = [");
    if build_script {
        format!("{manifest}\n[build-dependencies]\ncc = \"1\"\n")
    } else {
        manifest
    }
}

fn assert_without_theme(out_dir: &Path) {
    let bindings = fs::read_to_string(out_dir.join("bindings.rs")).unwrap();
    assert!(bindings.contains("pub fn lv_init()"));
    assert!(
        !bindings.contains("lv_theme_shadcn"),
        "shadcn-theme leaked into the build"
    );
}

#[test]
fn bindgen_only_build_resolves_profiler_include() {
    let fixture = scratch_dir("profiler-bindgen-only");
    write(
        fixture.join("Cargo.toml"),
        &manifest("profiler-bindgen-only-fixture", &[], false),
    );
    write(fixture.join("src/lib.rs"), "");

    for (index, profiler_include) in PROFILER_INCLUDES.iter().enumerate() {
        let config = profiler_config(&fixture.join(format!("config-{index}")), profiler_include);
        run(
            cargo(
                "build",
                &fixture.join("Cargo.toml"),
                &fixture.join("target"),
            )
            .env("DEP_LV_CONFIG_PATH", &config),
            &format!("bindgen-only build with LV_PROFILER_INCLUDE \"{profiler_include}\""),
        );
        let out_dir = lvgl_out_dir(&fixture.join("target"));
        assert_without_theme(&out_dir);
        assert!(
            !out_dir.join("liblvgl.a").exists(),
            "bindgen-only build compiled the LVGL archive"
        );
    }
    fs::remove_dir_all(fixture).unwrap();
}

#[test]
fn native_build_and_downstream_consumer_resolve_profiler_include() {
    let fixture = scratch_dir("profiler-native");
    let config = profiler_config(&fixture.join("config"), PROFILER_INCLUDES[0]);
    write(
        fixture.join("Cargo.toml"),
        &manifest("profiler-native-fixture", &["build-lvgl"], true),
    );
    // Downstream C code compiled only from the exported metadata, like an
    // external font crate.
    write(
        fixture.join("consumer.c"),
        "#include \"lvgl.h\"\n\
         #if !LV_USE_PROFILER\n#error profiler configuration not applied\n#endif\n\
         void profiler_consumer(void) { LV_PROFILER_BEGIN; LV_PROFILER_END; }\n",
    );
    write(
        fixture.join("build.rs"),
        r#"fn main() {
    let mut build = cc::Build::new();
    build.target(&std::env::var("DEP_LVGL_RUST_SYS_TARGET").unwrap());
    for path in std::env::var("DEP_LVGL_RUST_SYS_INCLUDE_PATHS").unwrap().split(';') {
        build.include(path);
    }
    let defines = std::env::var("DEP_LVGL_RUST_SYS_DEFINES").unwrap();
    for define in defines.split(';').filter(|define| !define.is_empty()) {
        let (name, value) = define
            .split_once('=')
            .map_or((define, None), |(name, value)| (name, Some(value)));
        build.define(name, value);
    }
    build.file("consumer.c").compile("profiler_consumer");
}
"#,
    );
    write(fixture.join("src/lib.rs"), "");

    run(
        cargo(
            "build",
            &fixture.join("Cargo.toml"),
            &fixture.join("target"),
        )
        .env("DEP_LV_CONFIG_PATH", &config),
        "native profiler build",
    );
    let out_dir = lvgl_out_dir(&fixture.join("target"));
    assert_without_theme(&out_dir);
    assert!(
        binutils("nm", &["-g"], &out_dir.join("liblvgl.a")).contains("lv_profiler_builtin_init"),
        "builtin profiler missing from the LVGL archive"
    );
    fs::remove_dir_all(fixture).unwrap();
}
