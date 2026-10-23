# Agent notes

`lvgl_rust_sys` builds LVGL from source and generates `no_std` Rust bindings
with bindgen. See the README for features, environment variables and the
downstream `DEP_LVGL_RUST_SYS_*` metadata contract.

## LVGL version

The `lvgl` submodule is pinned to **LVGL v9.6.0** (commit `80ca777e3`).

`git submodule status` and plain `git describe` show it as
`v9.3.0-1754-g80ca777e3`. That is misleading, not an outdated checkout: the
upstream `v9.6.0` tag is a lightweight tag, and these commands only consider
annotated ones. To check the real version, use one of:

- `git -C lvgl describe --tags` (prints `v9.6.0`)
- `lvgl/include/lvgl/lv_version.h` (`LVGL_VERSION_MAJOR/MINOR/PATCH`)
- the header of `lvgl/lv_conf_template.h` ("Configuration file for v9.6.0")

The `vendored_lvgl_version_matches_readme` test in `src/lib.rs` fails if the
submodule and the README disagree.

When bumping LVGL: update the submodule, regenerate `conf/lv_conf.h` from
`lvgl/lv_conf_template.h` (enable content with `#if 1`, wrap
`LV_COLOR_FORMAT_DEFAULT` in `#ifndef` so the `color_depth_*` features can
override it, keep `LV_USE_ASSERT_NULL`/`LV_USE_ASSERT_MALLOC` at `1`), then
update the version in the README, this file and the test.

## Layout

- `build/main.rs`: drives `cc`, bindgen and the exported metadata from a single
  `NativeConfig`.
- `build/config.rs`: pure logic, unit tested by `tests/build_config.rs`.
- `src/string_impl.rs`: C string functions for libc-less targets
  (`use-string-functions`).
- `tests/`: integration tests that build fixture crates with a nested
  `cargo --offline`.

## Checks

Run what CI runs (`.github/workflows/ci.yml`):

```sh
git submodule update --init --recursive
cargo fetch --manifest-path examples/external-fonts/Cargo.toml
cargo fetch --manifest-path tests/fixtures/font_incremental/Cargo.toml
cargo fmt --all --check
RUSTFLAGS="-D warnings" cargo clippy --all-targets --all-features
cargo test
```

`DEP_LV_CONFIG_PATH` defaults to `conf/` through `.cargo/config.toml`. The
integration tests need `nm` and `objdump` and take a few minutes.
