# lvgl-rust-gen: build-time generator of the safe LVGL bindings

[![crates.io](https://img.shields.io/crates/v/lvgl-rust-gen.svg)](https://crates.io/crates/lvgl-rust-gen)
[![CI](https://github.com/AlixANNERAUD/lvgl_rust/actions/workflows/ci.yml/badge.svg)](https://github.com/AlixANNERAUD/lvgl_rust/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](https://github.com/AlixANNERAUD/lvgl_rust/blob/main/LICENSE)

Host-only code generator and access audit behind
[`lvgl-rust`](https://crates.io/crates/lvgl-rust), the safe, `no_std` Rust
bindings to LVGL 9. **You should not need to depend on it directly:**
`lvgl-rust`'s build script runs it for you.

## What it does

- **Generates the safe API.** It parses the `bindings.rs` that
  [`lvgl-rust-sys`](https://crates.io/crates/lvgl-rust-sys) generated for the
  current `lv_conf.h`, and the LVGL headers, then writes `gen.rs` (widgets,
  enums, functions and style properties) and `coverage.txt`, which says what
  happened to every bound function. The safe API therefore always matches
  the widgets and features your configuration enables.
- **Applies reviewed tables.** `tables/lvgl-<major>.<minor>/` records what is
  wrapped, with shared or exclusive access, and how each pointer parameter
  is handled. Bindings of an LVGL version without tables are refused.
- **Audits LVGL's access rules** (feature `cli`, maintainers only). The
  `lvgl-rust-gen audit` tool compiles LVGL to LLVM IR with clang, builds its
  call graph and decides which functions can delete objects or free memory
  that event handlers hold. It writes the access and event tables the
  generator reads, and also checks the tables on LVGL version bumps.

How the generated code is organised and why the audit exists:
[generated bindings](https://github.com/AlixANNERAUD/lvgl_rust/blob/main/documentation/explanation/generated-bindings.md)
and [access model](https://github.com/AlixANNERAUD/lvgl_rust/blob/main/documentation/explanation/access-model.md).
The maintainer tasks (`mise run audit`, `mise run bump`) are listed in
[development tasks](https://github.com/AlixANNERAUD/lvgl_rust/blob/main/documentation/reference/development-tasks.md).

## License

MIT, see [LICENSE](https://github.com/AlixANNERAUD/lvgl_rust/blob/main/LICENSE).
