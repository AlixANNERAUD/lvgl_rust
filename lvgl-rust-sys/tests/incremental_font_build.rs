mod common;

use common::*;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
    time::SystemTime,
};

/// Restores the tracked fixture source even when the test fails.
struct RestoreFile {
    path: PathBuf,
    original: Vec<u8>,
}

impl Drop for RestoreFile {
    fn drop(&mut self) {
        fs::write(&self.path, &self.original).expect("restore fixture font source");
    }
}

fn build(target: &Path, config: &Path, what: &str) -> Output {
    let root = crate_root();
    run(
        cargo(
            "build",
            &root.join("tests/fixtures/font_incremental/Cargo.toml"),
            target,
        )
        .arg("-vv")
        .current_dir(&root)
        .env("DEP_LV_CONFIG_PATH", config),
        what,
    )
}

fn marker(target: &Path) -> PathBuf {
    fs::read_dir(target.join("debug/build"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("out/font-input.marker"))
        .find(|path| path.exists())
        .expect("font fixture build marker")
}

fn modified(path: &Path) -> SystemTime {
    fs::metadata(path).unwrap().modified().unwrap()
}

fn append(path: &Path, text: &[u8]) {
    let mut contents = fs::read(path).unwrap();
    contents.extend_from_slice(text);
    fs::write(path, contents).unwrap();
}

#[test]
fn font_only_edit_rebuilds_external_archive_without_rerunning_lvgl_or_bindgen() {
    let root = crate_root();
    let target = scratch_dir("font-incremental");
    let config = target.join("config");
    fs::create_dir_all(&config).unwrap();
    fs::copy(root.join("conf/lv_conf.h"), config.join("lv_conf.h")).unwrap();
    let font_source = root.join("tests/fixtures/font_incremental/fonts/profile.generated.c");
    let _restore = RestoreFile {
        path: font_source.clone(),
        original: fs::read(&font_source).unwrap(),
    };

    build(&target, &config, "initial fixture build");
    let marker = marker(&target);
    let first_marker = fs::read(&marker).unwrap();

    append(&font_source, b"\n/* isolated font-only edit */\n");
    let second = stdout_and_stderr(&build(&target, &config, "font-only rebuild"));
    assert!(
        second.contains("Fresh lvgl-rust-sys"),
        "LVGL package was not fresh after font-only edit:\n{second}"
    );
    assert_ne!(
        first_marker,
        fs::read(marker).unwrap(),
        "font archive marker did not change"
    );

    let lvgl_out = lvgl_out_dir(&target);
    let old_archive_time = modified(&lvgl_out.join("liblvgl.a"));
    let old_bindings_time = modified(&lvgl_out.join("bindings.rs"));
    append(
        &config.join("lv_conf.h"),
        b"\n/* configuration rebuild control */\n",
    );
    let control = stdout_and_stderr(&build(&target, &config, "configuration control build"));
    assert!(
        control.contains("Compiling lvgl-rust-sys"),
        "lv_conf.h edit did not rebuild the LVGL package:\n{control}"
    );
    assert!(
        modified(&lvgl_out.join("liblvgl.a")) > old_archive_time,
        "LVGL native archive timestamp did not change after config edit"
    );
    assert!(
        modified(&lvgl_out.join("bindings.rs")) > old_bindings_time,
        "bindgen output timestamp did not change after config edit"
    );

    fs::remove_dir_all(target).unwrap();
}
