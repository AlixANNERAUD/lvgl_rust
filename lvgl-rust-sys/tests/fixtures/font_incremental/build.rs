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
    let includes = env::var("DEP_LVGL_RUST_SYS_INCLUDE_PATHS").unwrap();
    let defines = env::var("DEP_LVGL_RUST_SYS_DEFINES").unwrap();
    let cflags = env::var("DEP_LVGL_RUST_SYS_CFLAGS").unwrap();
    let target = env::var("DEP_LVGL_RUST_SYS_TARGET").unwrap();
    let mut build = cc::Build::new();
    build.target(&target).warnings(true);
    for include in includes.split(';').filter(|path| !path.is_empty()) {
        build.include(PathBuf::from(include));
    }
    for define in defines
        .split(';')
        .chain(cflags.split(','))
        .filter(|define| !define.is_empty())
    {
        let (name, value) = define
            .split_once('=')
            .map_or((define, None), |(name, value)| (name, Some(value)));
        build.define(name, value);
    }

    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let fonts = manifest.join("fonts");
    println!("cargo:rerun-if-changed={}", fonts.display());
    let source = fs::read(fonts.join("profile.generated.c")).unwrap();
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("font-input.marker"),
        source,
    )
    .unwrap();
    add_sources(&mut build, &fonts);
    build.compile("font_assets");
}
