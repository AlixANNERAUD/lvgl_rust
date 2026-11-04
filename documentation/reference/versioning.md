# Versioning

How `lvgl-rust-sys` and `lvgl-rust` are versioned, and which LVGL each
release vendors.

## Version numbers

The crates follow [Semantic Versioning](https://semver.org) for their Rust
API. The build metadata of the version (after the `+`) names the LVGL
release vendored in the crate: `0.1.2+lvgl.9.6.0` is version `0.1.2`,
built on LVGL 9.6.0.

- Cargo ignores build metadata when it resolves dependencies: depend on
  `lvgl-rust = "0.1"`, never on the metadata. crates.io refuses two
  versions that differ only by their metadata, so it never tells two
  releases apart.
- `lvgl-rust-sys` and `lvgl-rust` always share one version (the
  `version_group` of `release-plz.toml`), even when only one of them
  changed. `lvgl-rust 0.1.2` depends on `lvgl-rust-sys 0.1.2`.
- `lvgl-rust-gen`, the build dependency of `lvgl-rust`, has a version of
  its own.
- A build holds a single `lvgl-rust-sys` (`links = "lvgl_rust_sys"`), so
  every crate of a dependency graph uses the same LVGL.

Each release vendors one LVGL version: the `lvgl` submodule, and the
generator tables reviewed for it (`lvgl-rust-gen/tables/lvgl-<major>.<minor>/`).
Building the crates against another LVGL is not supported.

## What changes the version

Releases are made by [release-plz](https://release-plz.dev) from the
commit messages (`release.yml`).

| Change | Before 1.0 | From 1.0 |
| --- | --- | --- |
| LVGL major or minor release (9.6 to 9.7) | `0.y` to `0.(y+1)` | major |
| LVGL patch release (9.6.0 to 9.6.1) | patch | patch, or minor if it adds items |
| Breaking change of the Rust API, the Cargo features or the exported build metadata (`DEP_LVGL_RUST_SYS_*`) | `0.y` to `0.(y+1)` | major |
| New items or features | patch | minor |
| Fixes | patch | patch |

An LVGL major or minor release changes the bindings and the generated
API, and needs new generator tables, so it is always a breaking release of
both crates, even when nothing else changes. An LVGL patch release keeps
the tables; the semver check of the release pull request
(cargo-semver-checks) catches a patch that changes the API.

Your own `lv_conf.h` is not part of the version: a configuration that
disables a widget removes its API whatever the version.

## Compatibility

| `lvgl-rust-sys`, `lvgl-rust` | LVGL |
| --- | --- |
| 0.1.0 to 0.1.2 | 9.6.0 |

Versions 0.1.0 and 0.1.1 carry no build metadata.

## Reading the version in code

`lvgl_rust_sys::LVGL_VERSION_MAJOR`, `LVGL_VERSION_MINOR` and
`LVGL_VERSION_PATCH` (bound from LVGL's `lv_version.h`; `lvgl_rust::sys`
re-exports them) give the vendored LVGL. A test of each crate checks that
its version's build metadata names the same release.
