# lvgl-rust-gen

Build-time generator of the safe LVGL bindings of
[`lvgl-rust`](https://crates.io/crates/lvgl-rust). Its build script runs it on
the bindings `lvgl-rust-sys` generated for the current `lv_conf.h`, so the safe
API always matches the enabled widgets and features.

It reads the reviewed tables in `tables/lvgl-<major>.<minor>/` (what is
wrapped, with shared or exclusive access, and how each pointer parameter is
handled) and refuses bindings of an LVGL version without tables. You should not
need to depend on it directly.

The `cli` feature builds the `lvgl-rust-gen` maintainer tool (access audit,
LVGL version bumps), which needs clang. See the
[repository](https://github.com/AlixANNERAUD/lvgl_rust) for details.
