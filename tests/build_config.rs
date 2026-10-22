//! Unit tests for the pure build script logic in `build/config.rs`.

#[allow(dead_code)]
#[path = "../build/config.rs"]
mod config;

use config::*;
use std::{fs, path::PathBuf};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lvgl-build-config-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn define_parses_names_and_values() {
    assert_eq!(
        Define::parse("FOO"),
        Some(Define::new("FOO", None::<String>))
    );
    assert_eq!(
        Define::parse(" FOO = 1 "),
        Some(Define::new("FOO", Some("1")))
    );
    assert_eq!(Define::parse("FOO="), Some(Define::new("FOO", Some(""))));
    assert_eq!(Define::parse("A=B=C"), Some(Define::new("A", Some("B=C"))));
    assert_eq!(Define::parse("  "), None);
}

#[test]
fn define_renders_metadata_and_compiler_syntax() {
    let flag = Define::new("FLAG", None::<String>);
    let value = Define::new("DEPTH", Some("16"));
    assert_eq!(flag.to_string(), "FLAG");
    assert_eq!(value.to_string(), "DEPTH=16");
    assert_eq!(flag.to_compiler_arg(), "-DFLAG");
    assert_eq!(value.to_compiler_arg(), "-DDEPTH=16");
}

#[test]
fn cflags_are_comma_separated_and_skip_blanks() {
    assert_eq!(parse_cflags(""), vec![]);
    assert_eq!(
        parse_cflags("A=1,,B, C=x "),
        vec![
            Define::new("A", Some("1")),
            Define::new("B", None::<String>),
            Define::new("C", Some("x")),
        ]
    );
}

#[test]
fn metadata_round_trips_through_cflags_parser() {
    let defines = vec![
        Define::new("A", Some("1")),
        Define::new("B", None::<String>),
    ];
    assert_eq!(join_defines(&defines, ";"), "A=1;B");
    assert_eq!(parse_cflags(&join_defines(&defines, ",")), defines);
    assert_eq!(
        join_paths(&[PathBuf::from("/a"), PathBuf::from("/b c")]),
        "/a;/b c"
    );
}

#[test]
fn color_depth_defaults_to_none() {
    let depth = ColorDepth::from_features(|_| false);
    assert_eq!(depth.selected, None);
    assert!(!depth.is_conflicting());
    assert!(depth.defines().is_empty());
}

#[test]
fn color_depth_sets_matching_format_and_depth() {
    for depth in COLOR_DEPTHS {
        let selected = ColorDepth::from_features(|d| d == depth);
        assert_eq!(selected.selected, Some(depth));
        assert_eq!(
            selected.defines(),
            vec![
                Define::new("LV_COLOR_FORMAT_DEFAULT", Some(color_format(depth))),
                Define::new("LV_COLOR_DEPTH", Some(depth.to_string())),
            ]
        );
    }
    assert_eq!(color_format(32), "LV_COLOR_FORMAT_XRGB8888");
}

#[test]
fn conflicting_color_depths_pick_the_smallest() {
    let depth = ColorDepth::from_features(|d| d == 16 || d == 32);
    assert_eq!(depth.selected, Some(16));
    assert_eq!(depth.requested, vec![16, 32]);
    assert!(depth.is_conflicting());
}

#[test]
fn config_dir_prefers_explicit_over_vendored() {
    let explicit = PathBuf::from("/explicit");
    let vendored = PathBuf::from("/vendored");
    assert_eq!(
        resolve_config_dir(Some(explicit.clone()), Some(vendored.clone())),
        Ok(explicit)
    );
    assert_eq!(
        resolve_config_dir(None, Some(vendored.clone())),
        Ok(vendored)
    );
    let error = resolve_config_dir(None, None).unwrap_err();
    assert!(error.contains(CONFIG_DIR_VAR), "{error}");
}

#[test]
fn config_dir_must_contain_lv_conf() {
    let dir = temp_dir("validate");
    assert!(validate_config_dir(&dir.join("missing")).is_err());
    assert!(validate_config_dir(&dir).unwrap_err().contains("lv_conf.h"));
    fs::write(dir.join("lv_conf.h"), "").unwrap();
    assert_eq!(validate_config_dir(&dir), Ok(()));
    assert!(validate_config_dir(&dir.join("lv_conf.h")).is_err());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn collect_files_is_recursive_filtered_and_sorted() {
    let dir = temp_dir("collect");
    for file in ["b.c", "a.c", "nested/deeper/z.c", "nested/c.h", "notes.txt"] {
        let path = dir.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
    }
    let files = collect_files(&dir, "c").unwrap();
    let relative: Vec<_> = files
        .iter()
        .map(|path| {
            path.strip_prefix(&dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    assert_eq!(relative, ["a.c", "b.c", "nested/deeper/z.c"]);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn shipped_config_lets_features_override_color_format() {
    let conf = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/conf/lv_conf.h")).unwrap();
    assert!(conf.contains("#ifndef LV_COLOR_FORMAT_DEFAULT"));
    assert!(
        validate_config_dir(&PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/conf"))).is_ok()
    );
}
