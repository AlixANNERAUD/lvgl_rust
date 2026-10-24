# LVGL for Rust

[![CI](https://github.com/xila-project/lvgl_rust/actions/workflows/ci.yml/badge.svg)](https://github.com/xila-project/lvgl_rust/actions/workflows/ci.yml)
[![LVGL](https://img.shields.io/badge/LVGL-9.6.0-blue)](https://github.com/lvgl/lvgl)
[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

Rust bindings to [LVGL](https://lvgl.io) 9, for microcontrollers, embedded
Linux, WebAssembly and desktop hosts.

| Crate | Description |
| --- | --- |
| [`lvgl_rust_sys`](lvgl-rust-sys) | Raw `no_std` FFI bindings. Compiles LVGL from source for your `lv_conf.h`. |
| [`lvgl-rust`](lvgl-rust) | Safe, `no_std` bindings on top of `lvgl_rust_sys` (in progress, see its [README](lvgl-rust/README.md)). |
| [`lvgl-rust-gen`](lvgl-rust-gen) | Build-time generator used by `lvgl-rust` (in progress). |

See [`lvgl-rust-sys/README.md`](lvgl-rust-sys/README.md) for usage, features and
development commands.
