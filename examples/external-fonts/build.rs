use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn add_sources(build: &mut cc::Build, path: &Path) {
    for entry in fs::read_dir(path).expect("font source directory") {
        let path = entry.expect("font source entry").path();
        if path.is_dir() {
            add_sources(build, &path);
        } else if path.extension().is_some_and(|extension| extension == "c") {
            println!("cargo:rerun-if-changed={}", path.display());
            build.file(path);
        }
    }
}

fn main() {
    let include_paths = env::var("DEP_LVGL_RUST_SYS_INCLUDE_PATHS")
        .expect("lvgl_rust_sys must export DEP_LVGL_RUST_SYS_INCLUDE_PATHS");
    let defines = env::var("DEP_LVGL_RUST_SYS_DEFINES")
        .expect("lvgl_rust_sys must export DEP_LVGL_RUST_SYS_DEFINES");
    let cflags = env::var("DEP_LVGL_RUST_SYS_CFLAGS")
        .expect("lvgl_rust_sys must export DEP_LVGL_RUST_SYS_CFLAGS");
    let target = env::var("DEP_LVGL_RUST_SYS_TARGET")
        .expect("lvgl_rust_sys must export DEP_LVGL_RUST_SYS_TARGET");

    let mut build = cc::Build::new();
    build.target(&target).warnings(true);
    for path in include_paths.split(';').filter(|path| !path.is_empty()) {
        build.include(PathBuf::from(path));
    }
    for define in defines.split(';').filter(|define| !define.is_empty()) {
        let (name, value) = define
            .split_once('=')
            .map_or((define, None), |(name, value)| (name, Some(value)));
        build.define(name, value);
    }
    for define in cflags.split(',').filter(|define| !define.is_empty()) {
        let (name, value) = define
            .split_once('=')
            .map_or((define, None), |(name, value)| (name, Some(value)));
        build.define(name, value);
    }

    let fonts = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("fonts");
    println!("cargo:rerun-if-changed={}", fonts.display());
    add_sources(&mut build, &fonts);
    build.compile("font_assets");
}
