# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0](https://github.com/AlixANNERAUD/lvgl_rust/releases/tag/lvgl-rust-gen-v0.1.0) - 2026-10-27

### Added

- generate safe wrappers from the bindings
- derive the access table from a call graph of LVGL
- generate out parameters, keep style selectors as values
- enforce composite part tables, classify parameters
- typed styles, constant styles and fonts
- copy-out text getters, label text from str and integers
- retained data: images, button maps, gradients, transitions
- lvgl-rust-gen bump
- event handlers and posted work
- subjects, observers and bindings
- calls on two objects, the keyboard-textarea relation
- display and input drivers
- focus groups
- held-region analysis and events.toml in the access audit
- generate the handler mask and sendable event codes from events.toml
- object-aware access audit, 17 setters become shared (#54)
- shims for the hit test parameter
- generate the restricted handler mask and query codes
- audit the hit test shims
- keyboard layouts, grid templates and scale labels
- defmt and debug-counters diagnostic features
- display geometry setters, button input devices, boxed drivers (#60)

### Documentation

- Diataxis documentation, overhead measurements and rewritten READMEs (#69)

### Fixed

- check creation and reordering, protect message box header buttons
- close holes found by the M4 handler review
- creating objects needs exclusive access (#66)

### Miscellaneous

- turn the repository into a workspace
- release with release-plz, coverage with Codecov
- fix the release, Miri and coverage jobs on main
- rename the package to lvgl-rust-sys, update repository links

### Other

- Merge the held-region audit into the stack
