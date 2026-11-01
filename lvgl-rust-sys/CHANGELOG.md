# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.1](https://github.com/AlixANNERAUD/lvgl_rust/compare/lvgl-rust-sys-v0.1.0...lvgl-rust-sys-v0.1.1) - 2026-11-01

### Fixed

- ship the shadcn theme in the lvgl-rust-sys package (#73)

## [0.1.0](https://github.com/AlixANNERAUD/lvgl_rust/releases/tag/lvgl-rust-sys-v0.1.0) - 2026-10-27

### Added

- export bindings path and link status, add private-state shims
- event handlers and posted work
- calls on two objects, the keyboard-textarea relation
- shims for the hit test parameter
- display geometry setters, button input devices, boxed drivers (#60)

### Documentation

- Diataxis documentation, overhead measurements and rewritten READMEs (#69)

### Fixed

- build for riscv32imc and wasm32-unknown-unknown

### Miscellaneous

- turn the repository into a workspace
- move to the 2024 edition
- check the minimum Rust version
- point links at the renamed repository
- release with release-plz, coverage with Codecov
- rename the package to lvgl-rust-sys, update repository links
- wasm32-wasip1 with wasi-sdk, tests and desktop example under wasmtime
