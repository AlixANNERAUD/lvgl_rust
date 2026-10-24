//! Mirrors lvgl-rust's views-only `cfg` from its exported slot count, and
//! links the target C compiler's support library (LVGL compiled by GCC calls
//! helpers such as `__gnu_thumb1_case_uqi`).

use std::env;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(lvgl_rust_views_only)");
    if env::var("DEP_LVGL_RUST_HANDLE_SLOTS").as_deref() == Ok("0") {
        println!("cargo::rustc-cfg=lvgl_rust_views_only");
    }

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("none") {
        return;
    }
    let compiler = cc::Build::new().get_compiler();
    if !compiler.is_like_gnu() {
        return;
    }
    let output = compiler
        .to_command()
        .arg("-print-libgcc-file-name")
        .output()
        .expect("cannot run the target C compiler");
    let libgcc = String::from_utf8(output.stdout).unwrap();
    let libgcc = libgcc.trim();
    if output.status.success() && !libgcc.is_empty() && libgcc != "libgcc.a" {
        println!("cargo::rustc-link-arg={libgcc}");
    }
}
