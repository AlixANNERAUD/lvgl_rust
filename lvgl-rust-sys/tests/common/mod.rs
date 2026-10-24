//! Helpers shared by the integration tests that drive nested Cargo builds.

#![allow(dead_code)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

/// Root of this crate.
pub fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Fresh, process-unique scratch directory.
pub fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lvgl-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

pub fn write(path: impl AsRef<Path>, contents: &str) {
    let path = path.as_ref();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// `cargo` command isolated from LVGL variables set in the outer environment.
pub fn cargo(subcommand: &str, manifest: &Path, target_dir: &Path) -> Command {
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command
        .arg(subcommand)
        .arg("--offline")
        .arg("--manifest-path")
        .arg(manifest)
        .arg("--target-dir")
        .arg(target_dir)
        // Tests match plain status lines such as "Fresh lvgl_rust_sys".
        .env("CARGO_TERM_COLOR", "never")
        .env("DEP_LV_CONFIG_PATH", crate_root().join("conf"))
        .env_remove("LVGL_FONTS_DIR")
        .env_remove("LVGL_CFLAGS");
    command
}

pub fn stdout_and_stderr(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Runs `command`, panicking with its output on failure.
pub fn run(command: &mut Command, what: &str) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{what} failed:\n{}",
        stdout_and_stderr(&output)
    );
    output
}

/// `OUT_DIR` of the `lvgl_rust_sys` build script under `target_dir`.
pub fn lvgl_out_dir(target_dir: &Path) -> PathBuf {
    fs::read_dir(target_dir.join("debug/build"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("out"))
        .find(|path| path.join("bindings.rs").exists())
        .expect("lvgl_rust_sys build output directory")
}

/// Output of a binutils tool (`nm`, `objdump`, ...).
pub fn binutils(tool: &str, args: &[&str], file: &Path) -> String {
    let output = Command::new(tool)
        .args(args)
        .arg(file)
        .output()
        .unwrap_or_else(|error| panic!("{tool} must be installed: {error}"));
    assert!(
        output.status.success(),
        "{tool} failed on {}",
        file.display()
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Minimal package depending on this crate.
pub fn dependent_manifest(name: &str, features: &[&str], build_script: bool) -> String {
    let features = features
        .iter()
        .map(|feature| format!("{feature:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "[package]\nname = {name:?}\nversion = \"0.1.0\"\nedition = \"2024\"\n{}\n\
         [dependencies]\nlvgl_rust_sys = {{ path = {:?}, features = [{features}] }}\n",
        if build_script {
            "build = \"build.rs\"\n"
        } else {
            ""
        },
        crate_root(),
    )
}
