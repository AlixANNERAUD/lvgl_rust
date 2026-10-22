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

fn build(root: &Path, target: &Path) -> Output {
    Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .arg("build")
        .arg("--offline")
        .arg("-vv")
        .arg("--manifest-path")
        .arg(root.join("tests/fixtures/font_incremental/Cargo.toml"))
        .arg("--target-dir")
        .arg(target)
        .current_dir(root)
        .env("DEP_LV_CONFIG_PATH", root.join("conf"))
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

#[test]
fn font_only_edit_rebuilds_external_archive_without_rerunning_lvgl_or_bindgen() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let target = std::env::temp_dir().join(format!("lvgl-font-incremental-{}", std::process::id()));
    let _ = fs::remove_dir_all(&target);
    let font_source = root.join("tests/fixtures/font_incremental/fonts/profile.generated.c");
    let original = fs::read(&font_source).unwrap();
    let _restore = RestoreFile {
        path: font_source.clone(),
        original: original.clone(),
    };

    let first = build(&root, &target);
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
    let second = build(&root, &target);
    assert!(
        second.status.success(),
        "font-only rebuild failed:\n{}\n{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let second_output = format!(
        "{}{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    assert!(
        second_output.contains("Fresh lvgl_rust_sys"),
        "LVGL package was not fresh after font-only edit:\n{second_output}"
    );
    assert_ne!(
        first_marker,
        fs::read(marker).unwrap(),
        "font archive marker did not change"
    );

    fs::remove_dir_all(target).unwrap();
}
