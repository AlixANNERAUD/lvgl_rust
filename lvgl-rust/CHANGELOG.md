# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0](https://github.com/AlixANNERAUD/lvgl_rust/releases/tag/lvgl-rust-v0.1.0) - 2026-10-28

### Added

- add the safe core of lvgl-rust
- generate safe wrappers from the bindings
- derive the access table from a call graph of LVGL
- enforce composite part tables, classify parameters
- typed styles, constant styles and fonts
- copy-out text getters, label text from str and integers
- retained data: images, button maps, gradients, transitions
- event handlers and posted work
- timers
- animations
- subjects, observers and bindings
- calls on two objects, the keyboard-textarea relation
- the Component trait
- display and input drivers
- focus groups
- async event queues
- closures on LVGL's heap
- held-region analysis and events.toml in the access audit
- generate the handler mask and sendable event codes from events.toml
- LVGL's heap from a Rust allocator
- formatted label text, Display for Error
- a desktop example, migration notes for Xila
- object-aware access audit, 17 setters become shared (#54)
- restricted handlers for query and draw codes
- keyboard layouts, grid templates and scale labels
- defmt and debug-counters diagnostic features
- display geometry setters, button input devices, boxed drivers (#60)
- owned image sources and event queues; fix a use-after-free in on_boxed (#61)

### Documentation

- clarify enter_callback in restricted handlers
- retained keyboard layouts, grid templates and scale labels
- Diataxis documentation, overhead measurements and rewritten READMEs (#69)

### Fixed

- close soundness gaps found in review of the safe core
- check creation and reordering, protect message box header buttons
- close holes found by the M4 handler review
- forbid 'static guards in the LvglAccess contract
- name draw task opacity accessors in full, correct handler examples
- keep panic-catching retained tests off targets where panics abort
- creating objects needs exclusive access (#66)
- repair CI on main after the #61 and #66 merges (#67)
- Owned::delete reports permanent refusals (#68)

### Miscellaneous

- turn the repository into a workspace
- format the safe core
- gate lvgl-rust's footprint on thumbv6m and riscv32imc, run ASan
- delete open dropdowns and closed message boxes under ASan
- release with release-plz, coverage with Codecov
- rename the package to lvgl-rust-sys, update repository links
- configuration matrix and Cortex-M firmware builds
- restricted handlers refuse codes outside their tier
- restricted handlers block sessions when none is active
- cover every counted drain path, and the counters in views-only mode
- run the lvgl-rust suite on targets without threads or unwinding
- wasm32-wasip1 with wasi-sdk, tests and desktop example under wasmtime
- keep the button map bounds checks running where panics abort
- compile-fail suite for lvgl-rust's ownership and access rules (#63)

### Other

- Merge the held-region audit into the stack
