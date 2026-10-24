# AGENTS.md

Guidance for coding agents working on `lvgl_rust_sys`. `CLAUDE.md` and
`GEMINI.md` are symlinks to this file: edit `AGENTS.md` only.

## What this crate is

A `-sys` crate: it compiles [LVGL](https://github.com/lvgl/lvgl) from source
with `cc`, generates raw `no_std` FFI bindings with bindgen, and exports the
native build configuration to dependent build scripts. It exposes the C API as
is; there is no safe wrapper.

Its main consumer is the [Xila](https://github.com/xila-project) embedded OS,
which runs on microcontrollers (ESP32, ...) as well as native and WebAssembly
hosts. Xila uses `default-features = false`, `shadcn-theme`,
`color_depth_16`/`color_depth_32`, its own `lv_conf.h` and, for now, the legacy
`LVGL_FONTS_DIR` mode while it moves to an external font crate. Do not break
these paths, and do not edit Xila from this repository.

## Repository map

| Path | Role |
| --- | --- |
| `build/main.rs` | Build script entry: resolves the config, compiles LVGL, exports metadata, runs bindgen. |
| `build/config.rs` | Pure, `std`-only logic (define parsing, color depth, config dir checks, file collection). |
| `src/lib.rs` | Includes the generated bindings in a `bindings` module and re-exports them; library tests. |
| `src/string_impl.rs` | C string functions (`strlen`, `strcmp`, ...) for libc-less targets. |
| `shims/` | Small C helpers bound alongside LVGL (`lvgl_sys.h` is bindgen's root header). |
| `conf/lv_conf.h` | Vendored LVGL configuration (tests, docs.rs, `use-vendored-config`). |
| `lvgl/` | LVGL submodule (do not edit; bump the pinned commit instead). |
| `lvgl_theme_shadcn/` | shadcn theme submodule, compiled with `shadcn-theme`. |
| `examples/external-fonts/` | Separate package showing how to build fonts in their own crate. |
| `tests/` | Unit tests for `build/config.rs` and integration tests driving nested `cargo` builds. |
| `tests/common/mod.rs` | Shared helpers for the integration tests. |
| `tests/fixtures/font_incremental/` | Package used by the incremental rebuild test. |
| `mise.toml` | Toolchain (Rust + mbx build cache) and tasks mirroring CI. |

## Build model

`NativeConfig` in `build/main.rs` is the single source of truth. The same
include paths and definitions feed:

1. `cc` (the `liblvgl.a` archive, only with the `build-lvgl` feature);
2. bindgen (as `-I`/`-D` clang arguments);
3. the metadata exported through `links = "lvgl_rust_sys"`:
   `DEP_LVGL_RUST_SYS_INCLUDE_PATHS` (`;`-separated), `DEP_LVGL_RUST_SYS_DEFINES`
   (`;`-separated), `DEP_LVGL_RUST_SYS_CFLAGS` (`,`-separated `LVGL_CFLAGS`) and
   `DEP_LVGL_RUST_SYS_TARGET`.

Never add an include path or define to only one of these: the bindings, the
library and downstream C code (fonts) must agree on the ABI. The metadata names
and separators are a public contract used by dependent crates; changing them is
a breaking change.

Inputs:

| Input | Effect |
| --- | --- |
| `DEP_LV_CONFIG_PATH` | Directory containing `lv_conf.h` (required, except with `use-vendored-config` or `DOCS_RS`). Empty means unset. |
| `LVGL_CFLAGS` | Extra `NAME` / `NAME=VALUE` definitions, comma-separated. |
| `LVGL_FONTS_DIR` | Legacy mode: compile that directory's `.c` files into LVGL and bind its top-level headers. |
| `EMSDK` | Required for `*-emscripten` targets. |
| `color_depth_{1,8,16,24,32}` | Defines `LV_COLOR_FORMAT_DEFAULT` and `LV_COLOR_DEPTH`. Several enabled: smallest wins, with a warning. |
| `shadcn-theme` | Compiles and binds `lvgl_theme_shadcn`. |
| `build-lvgl` (default) | Compile the native library. Without it, only bindings are generated. |
| `use-string-functions` (default) | Link the Rust C string functions. |

Read every environment variable through `env_var()` in `build/main.rs`: it
emits `cargo:rerun-if-env-changed`. Variables that Cargo always sets (`TARGET`,
`OUT_DIR`, ...) go through `cargo_env()`. Any new input file or directory needs a
`rerun-if-changed`, otherwise edits will not trigger a rebuild.

Keep `build/config.rs` free of `cc`, `bindgen` and Cargo I/O so that
`tests/build_config.rs` can include it with `#[path]`. Put new parsing or
selection logic there, with tests.

## LVGL version

The `lvgl` submodule is pinned to **LVGL v9.6.0** (commit `80ca777e3`), the
latest stable release at the time of writing.

`git submodule status` and plain `git describe` show it as
`v9.3.0-1754-g80ca777e3`. This does **not** mean the checkout is outdated: the
upstream `v9.6.0` tag is lightweight and these commands only consider annotated
tags. Read the real version with:

- `git -C lvgl describe --tags` (prints `v9.6.0`);
- `lvgl/include/lvgl/lv_version.h` (`LVGL_VERSION_MAJOR/MINOR/PATCH`);
- the header of `lvgl/lv_conf_template.h`.

If `lvgl/` looks older than the commit recorded by the superproject (a `+` in
`git submodule status`), run `git submodule update --init --recursive` before
building: compiling old sources against the 9.6 `lv_conf.h` fails.

### Bumping LVGL

1. Check out the new release tag in `lvgl/` and stage the submodule.
2. Regenerate `conf/lv_conf.h` from `lvgl/lv_conf_template.h`. It must differ
   from the template only by:
   - `#if 1` instead of `#if 0` to enable the content;
   - `LV_COLOR_FORMAT_DEFAULT` wrapped in `#ifndef LV_COLOR_FORMAT_DEFAULT`, so
     the `color_depth_*` features can override it;
   - `LV_USE_ASSERT_NULL` and `LV_USE_ASSERT_MALLOC` set to `1`.

   Verify with `diff lvgl/lv_conf_template.h conf/lv_conf.h`.
3. Build and look for new LVGL deprecation warnings (`#warning ... is
   deprecated`). The build is expected to be warning-free with `conf/`.
4. Check the shims in `shims/` and the shadcn theme still compile, then run
   the full test suite.
5. Update the version in the README badge and feature list, in this file, and
   in the `vendored_lvgl_version_matches_readme` test in `src/lib.rs`.

## Testing

```sh
git submodule update --init --recursive
# Nested builds run offline; their packages have their own lockfile.
cargo fetch --manifest-path examples/external-fonts/Cargo.toml
cargo fetch --manifest-path tests/fixtures/font_incremental/Cargo.toml
cargo test                                         # full suite, a few minutes
cargo test --lib --test build_config               # fast loop, seconds
cargo test --lib --features shadcn-theme,color_depth_32
```

With [mise](https://mise.jdx.dev) installed, `mise install` sets up the
toolchain and `mise run fetch`, `mise run test`, `mise run test-fast`,
`mise run clippy`, `mise run fmt` and `mise run doc` wrap the same commands.
Cargo then goes through mbx ([mr boxington](https://github.com/jdx/mr-boxington)),
which caches the LVGL C objects and build script output; `mbx explain --last`
says why something was rebuilt. Keep `mise.toml` tasks and CI in sync.

`DEP_LV_CONFIG_PATH` defaults to `conf/` through `.cargo/config.toml` (only for
cargo commands run in this checkout; an explicit value wins).

What the suites cover:

- `src/lib.rs` tests: LVGL initialization, color shims, the configured color
  depth really reaching the C library, the pinned LVGL version. Tests that touch
  LVGL global state must hold `lvgl_lock()`; LVGL is not thread safe and the
  test harness is multithreaded.
- `src/string_impl.rs` tests: C semantics of the string functions.
- `tests/build_config.rs`: `build/config.rs` logic, no cargo involved.
- Integration tests (`font_build_modes`, `incremental_font_build`,
  `native_build_interface`, `external_font_example`): build fixture packages
  with a nested `cargo --offline` through `tests/common::cargo()`, then inspect
  `bindings.rs`, `liblvgl.a`, binaries (`nm`, `objdump`) or Cargo's
  `Fresh`/`Compiling` lines. Each compiles LVGL, so they are slow.

Rules for integration tests:

- Always start nested builds with `common::cargo()`. It sets `--offline`,
  `CARGO_TERM_COLOR=never` (CI forces colors, which breaks matching on
  `Fresh lvgl_rust_sys`), the vendored config, and clears `LVGL_FONTS_DIR` and
  `LVGL_CFLAGS` inherited from the outer environment.
- Use `common::scratch_dir()` for fixtures and separate target directories, and
  remove them at the end of the test.
- A fixture with its own `Cargo.lock` needs a matching `cargo fetch` in the
  `fetch` task of `mise.toml` (which CI runs), otherwise CI fails with "no
  matching package".
- Restore any tracked file a test modifies, even on failure (see `RestoreFile`
  in `tests/incremental_font_build.rs`).

## CI

`.github/workflows/ci.yml` runs with `RUSTFLAGS=-D warnings`. Every job
installs the toolchain from `mise.toml` (`jdx/mise-action`); jobs that compile
use `jdx/mr-boxington-action` for caching and run `mbx clippy`/`mbx test`.
Caches are saved only on pushes to `main`; pull requests restore them. It
checks:

- `cargo fmt --check` for the crate, the example and the fixture;
- clippy with default features, `--all-features` and `--no-default-features`;
- the full test suite with default features, plus library and build-config
  tests for `shadcn-theme`, `color_depth_32`, `color_depth_8` and without string
  functions;
- `cargo doc` in docs.rs mode (`DOCS_RS=1`, empty `DEP_LV_CONFIG_PATH`).

Before pushing, run at least:

```sh
cargo fmt --all --check
RUSTFLAGS="-D warnings" cargo clippy --all-targets --all-features
RUSTFLAGS="-D warnings" cargo test --lib --test build_config
```

or `mise run check`, which runs the same checks plus clippy for every feature
set.

## Pitfalls

- **Bindgen output lints.** Generated code lives in the `bindings` module in
  `src/lib.rs`, which carries its own `#[allow(...)]` list. Do not add
  crate-wide allows for bindgen warnings.
- **`c_char` signedness.** `c_char` is `i8` on x86 and `u8` on aarch64 and some
  embedded targets. Code like `x as u8` can look redundant to clippy on one
  host and be required on another; allow `clippy::unnecessary_cast` locally
  with a comment rather than removing the cast.
- **C semantics in `string_impl.rs`.** These functions replace libc for LVGL:
  compare bytes as `unsigned char`, never read past `n`, and treat the NUL
  terminator as part of the string for `strchr`/`strrchr`. Add a test for any
  change.
- **Color depth in `lv_conf.h`.** LVGL 9 configures `LV_COLOR_FORMAT_DEFAULT`;
  `LV_COLOR_DEPTH` is derived from it and deprecated. A config that hard-codes
  either value without `#ifndef` silently conflicts with the `color_depth_*`
  features.
- **Fonts.** The recommended mode builds fonts in a separate crate from the
  exported metadata, so font edits never rebuild LVGL. Legacy `LVGL_FONTS_DIR`
  must keep working, but must stay opt-in: never scan the working directory
  for fonts implicitly.
- **Cross-compilation.** bindgen gets `--target` only when `TARGET != HOST`;
  the C compiler comes from `cc`, which honours `CC_<target>`/`CFLAGS_<target>`.
  Host builds do not prove that a change works on embedded targets; say what
  you actually checked.
- **Submodules.** Never edit files under `lvgl/` or `lvgl_theme_shadcn/`. Fix
  problems in `build/`, `shims/` or `conf/`, or bump the pinned commit.
- **Generated files.** `bindings.rs` and `liblvgl.a` live in `target/`; never
  commit build output. The `*.generated.c` font fixtures are small hand-written
  stand-ins for `lv_font_conv` output.

## Conventions

- Rust 2021, recent stable toolchain. Format with `cargo fmt`; match the
  surrounding style and comment density.
- Commit messages follow `type: summary` (`build:`, `conf:`, `fix:`, `test:`,
  `docs:`, `ci:`, `feat:`), with a body explaining the why for anything
  non-trivial.
- Keep pull requests focused, one concern each. For larger work, prefer a
  stack of small PRs, each targeting the previous branch.
- Do not add AI attribution (no `Co-Authored-By` trailers for assistants and
  no "generated with" lines in commits or pull requests).
- Document user-visible changes (features, environment variables, metadata) in
  `README.md`, and keep this file up to date when the workflow changes.
