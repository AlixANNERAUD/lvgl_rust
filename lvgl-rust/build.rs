//! Checks that the `lvgl_rust_sys` build can host the safe layer and writes
//! the build-time configuration (`$OUT_DIR/config.rs`).

use std::{env, fmt::Write as _, fs, path::PathBuf};

const SLOTS_VAR: &str = "LVGL_RUST_HANDLE_SLOTS";
const DEPTH_VAR: &str = "LVGL_RUST_MAX_CALLBACK_DEPTH";
const DEFAULT_SLOTS: u32 = 32;
const MAX_SLOTS: u32 = 32_768;
const DEFAULT_DEPTH: u32 = 16;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(lvgl_rust_views_only)");
    println!("cargo::rustc-check-cfg=cfg(lvgl_rust_docs)");

    let mut errors = Vec::new();
    let slots = match handle_slots(env_var(SLOTS_VAR).as_deref()) {
        Ok(slots) => slots,
        Err(error) => {
            errors.push(error);
            DEFAULT_SLOTS
        }
    };
    let depth = match callback_depth(env_var(DEPTH_VAR).as_deref()) {
        Ok(depth) => depth,
        Err(error) => {
            errors.push(error);
            DEFAULT_DEPTH
        }
    };
    if slots == 0 {
        println!("cargo::rustc-cfg=lvgl_rust_views_only");
    }

    // docs.rs builds `lvgl_rust_sys` with its own configuration (OBJ_ID off)
    // and links nothing: document the API, skip the checks.
    if env_var("DOCS_RS").is_some() {
        println!("cargo::rustc-cfg=lvgl_rust_docs");
    } else {
        if env_var("DEP_LVGL_RUST_SYS_LINKED").as_deref() != Some("1") {
            errors.push(
                "lvgl-rust needs the C library: enable the `build-lvgl` feature of lvgl_rust_sys"
                    .to_owned(),
            );
        }
        let bindings = env_var("DEP_LVGL_RUST_SYS_BINDINGS")
            .expect("lvgl_rust_sys did not export DEP_LVGL_RUST_SYS_BINDINGS");
        println!("cargo::rerun-if-changed={bindings}");
        let bindings = fs::read_to_string(&bindings)
            .unwrap_or_else(|error| panic!("Cannot read {bindings}: {error}"));
        // rustfmt may be missing: compare the bindings without whitespace.
        let compact: String = bindings.chars().filter(|c| !c.is_whitespace()).collect();
        if slots > 0 {
            errors.extend(check_obj_id(&compact));
        } else if constant(&compact, "LV_USE_OBJ_ID") == Some(1)
            && constant(&compact, "LV_USE_OBJ_ID_BUILTIN") == Some(0)
        {
            println!(
                "cargo::warning=views-only mode defines no OBJ_ID hooks: the application must \
                 define the five lv_obj_*_id functions, or lv_conf.h must set \
                 LV_USE_OBJ_ID_BUILTIN 1 (or LV_USE_OBJ_ID 0)"
            );
        }
    }

    let mut config = String::new();
    writeln!(config, "pub(crate) const HANDLE_SLOTS: usize = {slots};").unwrap();
    writeln!(config, "pub(crate) const MAX_CALLBACK_DEPTH: u8 = {depth};").unwrap();
    for error in errors {
        writeln!(config, "compile_error!({error:?});").unwrap();
    }
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    fs::write(out.join("config.rs"), config).unwrap();
}

/// `LVGL_RUST_HANDLE_SLOTS`: rounded up to a power of two; `0` selects the
/// views-only mode.
fn handle_slots(raw: Option<&str>) -> Result<u32, String> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_SLOTS);
    };
    match raw.trim().parse::<u32>() {
        Ok(0) => Ok(0),
        Ok(slots) if slots <= MAX_SLOTS => Ok(slots.next_power_of_two()),
        _ => Err(format!(
            "{SLOTS_VAR} must be an integer from 0 to {MAX_SLOTS}, got {raw:?}"
        )),
    }
}

/// `LVGL_RUST_MAX_CALLBACK_DEPTH`: 1 to 254, so depth counters fit in a `u8`.
fn callback_depth(raw: Option<&str>) -> Result<u32, String> {
    let Some(raw) = raw else {
        return Ok(DEFAULT_DEPTH);
    };
    match raw.trim().parse::<u32>() {
        Ok(depth @ 1..=254) => Ok(depth),
        _ => Err(format!(
            "{DEPTH_VAR} must be an integer from 1 to 254, got {raw:?}"
        )),
    }
}

/// The handle registry tracks objects through LVGL's OBJ_ID hooks, which the
/// crate defines: LVGL must call them and must not define its own.
fn check_obj_id(bindings: &str) -> Option<String> {
    let expected = [
        ("LV_USE_OBJ_ID", 1),
        ("LV_OBJ_ID_AUTO_ASSIGN", 1),
        ("LV_USE_OBJ_ID_BUILTIN", 0),
    ];
    if expected
        .iter()
        .all(|&(name, value)| constant(bindings, name) == Some(value))
    {
        return None;
    }
    Some(format!(
        "lvgl-rust tracks objects through LVGL's OBJ_ID hooks. Add to lv_conf.h:\n\
         \n    #define LV_USE_OBJ_ID 1\n    #define LV_OBJ_ID_AUTO_ASSIGN 1\n    #define LV_USE_OBJ_ID_BUILTIN 0\n\n\
         lvgl-rust then defines lv_obj_assign_id, lv_obj_free_id, lv_obj_set_id, \
         lv_obj_id_compare and lv_obj_stringify_id. To keep LVGL's own OBJ_ID \
         support instead, build with {SLOTS_VAR}=0 (views only, no handles)."
    ))
}

/// Value of an integer constant in bindgen output stripped of whitespace
/// (`pubconstNAME:T=V;`).
fn constant(compact: &str, name: &str) -> Option<i64> {
    let start = compact.find(&format!("pubconst{name}:"))?;
    let rest = &compact[start..];
    let statement = &rest[..rest.find(';')?];
    statement.split('=').nth(1)?.parse().ok()
}

fn env_var(name: &str) -> Option<String> {
    println!("cargo::rerun-if-env-changed={name}");
    env::var(name).ok().filter(|value| !value.is_empty())
}
