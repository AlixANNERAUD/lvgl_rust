use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct RestoreFile {
    path: PathBuf,
    original: Vec<u8>,
}

impl Drop for RestoreFile {
    fn drop(&mut self) {
        fs::write(&self.path, &self.original).expect("restore fixture font source");
    }
}

fn build(root: &Path, target: &Path, config: &Path) -> Output {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .arg("build")
        .arg("--offline")
        .arg("-vv")
        .arg("--manifest-path")
        .arg(root.join("tests/fixtures/font_incremental/Cargo.toml"))
        .arg("--target-dir")
        .arg(target)
        .current_dir(root)
        .env("DEP_LV_CONFIG_PATH", config)
        .env_remove("LVGL_FONTS_DIR")
        .output()
        .unwrap()
}

fn marker(target: &Path) -> PathBuf {
    fs::read_dir(target.join("debug/build"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("out/font-input.marker"))
        .find(|path| path.exists())
        .expect("font fixture build marker")
}

fn lvgl_output_dir(target: &Path) -> PathBuf {
    fs::read_dir(target.join("debug/build"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("out"))
        .find(|path| path.join("liblvgl.a").exists())
        .expect("LVGL archive output directory")
}

fn combined_output(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn font_only_edit_rebuilds_external_archive_without_rerunning_lvgl_or_bindgen() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = std::env::temp_dir().join(format!("lvgl-font-incremental-{}", std::process::id()));
    let _ = fs::remove_dir_all(&target);
    let config = target.join("config");
    fs::create_dir_all(&config).unwrap();
    fs::copy(root.join("conf/lv_conf.h"), config.join("lv_conf.h")).unwrap();
    let font_source = root.join("tests/fixtures/font_incremental/fonts/profile.generated.c");
    let original = fs::read(&font_source).unwrap();
    let _restore = RestoreFile {
        path: font_source.clone(),
        original: original.clone(),
    };

    let first = build(&root, &target, &config);
    assert!(
        first.status.success(),
        "initial fixture build failed:\n{}\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    let marker = marker(&target);
    let first_marker = fs::read(&marker).unwrap();

    let mut changed = original;
    changed.extend_from_slice(b"\n/* isolated font-only edit */\n");
    fs::write(&font_source, changed).unwrap();
    let second = build(&root, &target, &config);
    assert!(
        second.status.success(),
        "font-only rebuild failed:\n{}\n{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let second_output = combined_output(&second);
    assert!(
        second_output.contains("Fresh lvgl_rust_sys"),
        "LVGL package was not fresh after font-only edit:\n{second_output}"
    );
    assert_ne!(
        first_marker,
        fs::read(marker).unwrap(),
        "font archive marker did not change"
    );

    let lvgl_out = lvgl_output_dir(&target);
    let old_archive_time = fs::metadata(lvgl_out.join("liblvgl.a"))
        .unwrap()
        .modified()
        .unwrap();
    let old_bindings_time = fs::metadata(lvgl_out.join("bindings.rs"))
        .unwrap()
        .modified()
        .unwrap();
    let mut changed_config = fs::read(config.join("lv_conf.h")).unwrap();
    changed_config.extend_from_slice(b"\n/* configuration rebuild control */\n");
    fs::write(config.join("lv_conf.h"), changed_config).unwrap();
    let control = build(&root, &target, &config);
    assert!(
        control.status.success(),
        "configuration control build failed:\n{}\n{}",
        String::from_utf8_lossy(&control.stdout),
        String::from_utf8_lossy(&control.stderr)
    );
    let control_output = combined_output(&control);
    assert!(
        control_output.contains("Compiling lvgl_rust_sys"),
        "lv_conf.h edit did not rebuild the LVGL package:\n{control_output}"
    );
    assert!(
        fs::metadata(lvgl_out.join("liblvgl.a"))
            .unwrap()
            .modified()
            .unwrap()
            > old_archive_time,
        "LVGL native archive timestamp did not change after config edit"
    );
    assert!(
        fs::metadata(lvgl_out.join("bindings.rs"))
            .unwrap()
            .modified()
            .unwrap()
            > old_bindings_time,
        "bindgen output timestamp did not change after config edit"
    );

    fs::remove_dir_all(target).unwrap();
}
