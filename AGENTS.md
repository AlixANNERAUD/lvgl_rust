# AGENTS.md

Guidance for coding agents working on this repository. `CLAUDE.md` and
`GEMINI.md` are symlinks to this file: edit `AGENTS.md` only.

## What this repository is

A Cargo workspace with three crates:

- `lvgl-rust-sys/` (package `lvgl_rust_sys`): a `-sys` crate. It compiles
  [LVGL](https://github.com/lvgl/lvgl) from source with `cc`, generates raw
  `no_std` FFI bindings with bindgen, and exports the native build
  configuration to dependent build scripts. It exposes the C API as is.
- `lvgl-rust/`: the safe, `no_std` layer on top of it (in progress).
- `lvgl-rust-gen/`: the host-only generator used by `lvgl-rust`'s build
  script (in progress).

The main consumer is the [Xila](https://github.com/xila-project) embedded OS,
which runs on microcontrollers (ESP32, ...) as well as native and WebAssembly
hosts. Xila depends on `lvgl_rust_sys` through git (Cargo finds the package by
name inside the workspace) and uses `default-features = false`, `shadcn-theme`,
`color_depth_16`/`color_depth_32`, its own `lv_conf.h` and, for now, the legacy
`LVGL_FONTS_DIR` mode while it moves to an external font crate. Do not break
these paths, and do not edit Xila from this repository.

## Repository map

Paths in the following sections are relative to `lvgl-rust-sys/` unless they
start at the repository root (`mise.toml`, `.github/`, `.cargo/`).

| Path | Role |
| --- | --- |
| `Cargo.toml` (root) | Workspace manifest; shared `edition`, `rust-version`, licence. |
| `.cargo/config.toml` (root) | Default `DEP_LV_CONFIG_PATH` (`lvgl-rust-sys/conf`). |
| `lvgl-rust-sys/build/main.rs` | Build script entry: resolves the config, compiles LVGL, exports metadata, runs bindgen. |
| `lvgl-rust-sys/build/config.rs` | Pure, `std`-only logic (define parsing, color depth, config dir checks, file collection). |
| `lvgl-rust-sys/src/lib.rs` | Includes the generated bindings in a `bindings` module and re-exports them; library tests. |
| `lvgl-rust-sys/src/string_impl.rs` | C string functions (`strlen`, `strcmp`, ...) for libc-less targets. |
| `lvgl-rust-sys/shims/freestanding/` | Headers for targets without a C library (`inttypes.h`). |
| `lvgl-rust-sys/shims/` | Small C helpers bound alongside LVGL (`lvgl_sys.h` is bindgen's root header). `lvgl_sys.c` may include `lvgl_private.h` to reach private fields; keep private types out of `lvgl_sys.h`. |
| `lvgl-rust-sys/conf/lv_conf.h` | Vendored LVGL configuration (tests, docs.rs, `use-vendored-config`). |
| `lvgl-rust-sys/lvgl/` | LVGL submodule (do not edit; bump the pinned commit instead). |
| `lvgl-rust-sys/lvgl_theme_shadcn/` | shadcn theme submodule, compiled with `shadcn-theme`. |
| `lvgl-rust-sys/examples/external-fonts/` | Separate package showing how to build fonts in their own crate. |
| `lvgl-rust-sys/tests/` | Unit tests for `build/config.rs` and integration tests driving nested `cargo` builds. |
| `lvgl-rust-sys/tests/common/mod.rs` | Shared helpers for the integration tests. |
| `lvgl-rust-sys/tests/fixtures/font_incremental/` | Package used by the incremental rebuild test. |
| `lvgl-rust/src/base/` | Safe core: session and token (`session.rs`), handle registry (`registry.rs`), OBJ_ID hooks (`hook.rs`), deferred requests (`drain.rs`), structural checks (`structural.rs`), views and handles. |
| `lvgl-rust/src/base/ffi.rs` | Every LVGL call the core makes; `ffi_stub.rs` is its Rust model for Miri and loom. |
| `lvgl-rust/conf/lv_conf.h` | Second vendored configuration, with the OBJ_ID hooks the registry needs. |
| `lvgl-rust/build.rs` | Checks the `-sys` build (linked, OBJ_ID config) and writes `config.rs` (`LVGL_RUST_HANDLE_SLOTS`, `LVGL_RUST_MAX_CALLBACK_DEPTH`). |
| `lvgl-rust/size-gate/` | Firmware and script (`gate.sh`) checking lvgl-rust's footprint on thumbv6m and riscv32imc. |
| `lvgl-rust-gen/src/` | Generator: parses `bindings.rs` (`parse.rs`) and the headers (`headers.rs`), loads the tables (`tables.rs`), classifies every function (`classify.rs`) and emits `gen.rs` (`emit.rs`). |
| `lvgl-rust-gen/tables/lvgl-9.6/` | Reviewed tables: `access.toml` (what is wrapped, shared or exclusive), `classes.toml` (hierarchy, names, crate-owned and hand-written functions, composite parts), `relations.toml`, `overrides/**/*.toml`, `universe.txt`. |
| `lvgl-rust-gen/tests/golden/` | Expected generator output for the synthetic input of `tests/generate.rs`. |
| `mise.toml` (root) | Toolchain (Rust + mbx build cache) and tasks mirroring CI. |

## Build model

`NativeConfig` in `build/main.rs` is the single source of truth. The same
include paths and definitions feed:

1. `cc` (the `liblvgl.a` archive, only with the `build-lvgl` feature);
2. bindgen (as `-I`/`-D` clang arguments);
3. the metadata exported through `links = "lvgl_rust_sys"`:
   `DEP_LVGL_RUST_SYS_INCLUDE_PATHS` (`;`-separated), `DEP_LVGL_RUST_SYS_DEFINES`
   (`;`-separated), `DEP_LVGL_RUST_SYS_CFLAGS` (`,`-separated `LVGL_CFLAGS`) and
   `DEP_LVGL_RUST_SYS_TARGET`. Two keys describe the build itself:
   `DEP_LVGL_RUST_SYS_BINDINGS` (absolute path to `bindings.rs`, read by the
   safe crate's generator) and `DEP_LVGL_RUST_SYS_LINKED` (`1` with
   `build-lvgl`, `0` otherwise).

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

   Verify with `diff lvgl/lv_conf_template.h conf/lv_conf.h`. Then derive
   `lvgl-rust/conf/lv_conf.h` from it; the two differ only by
   `LV_USE_OBJ_ID 1` and `LV_USE_OBJ_ID_BUILTIN 0` (check with
   `diff lvgl-rust-sys/conf/lv_conf.h lvgl-rust/conf/lv_conf.h`).
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
cargo fetch --manifest-path lvgl-rust-sys/examples/external-fonts/Cargo.toml
cargo fetch --manifest-path lvgl-rust-sys/tests/fixtures/font_incremental/Cargo.toml
cargo test -p lvgl_rust_sys -p lvgl-rust-gen               # full suite, a few minutes
cargo test -p lvgl_rust_sys --lib --test build_config      # fast loop, seconds
cargo test -p lvgl_rust_sys --lib --features shadcn-theme,color_depth_32
```

With [mise](https://mise.jdx.dev) installed, `mise install` sets up the
toolchain and `mise run fetch`, `mise run test`, `mise run test-fast`,
`mise run clippy`, `mise run fmt` and `mise run doc` wrap the same commands.
Cargo then goes through mbx ([mr boxington](https://github.com/jdx/mr-boxington)),
which caches the LVGL C objects and build script output; `mbx explain --last`
says why something was rebuilt. Keep `mise.toml` tasks and CI in sync.

`DEP_LV_CONFIG_PATH` defaults to `lvgl-rust-sys/conf/` through the root
`.cargo/config.toml` (only for
cargo commands run in this checkout; an explicit value wins).

`lvgl-rust` needs a configuration with the OBJ_ID hooks: it fails to build
against `lvgl-rust-sys/conf` (a `compile_error!` that says what to add). Test
it in a separate cargo invocation with its own configuration and target
directory, as its mise tasks do: `mise run clippy-safe`, `test-safe`,
`test-views-only` (no handles, `-sys` configuration), `test-asan` (LVGL under
AddressSanitizer, with C library malloc so freed objects are visible),
`miri`, `loom` and `size-gate`. Do
not use `--workspace` for cargo commands that build `lvgl-rust`.

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

- `lvgl-rust/src/tests.rs`: the core (sessions, handles, ownership, deferred
  requests, structural checks). The same tests run against LVGL and, under
  Miri, against `ffi_stub.rs`; keep the stub's behaviour in line with LVGL's
  for every function `ffi.rs` exposes. Tests hold the `LOCK` guard.
- `lvgl-rust/src/tests.rs` `protocol` module: loom models of `Owned::drop`
  racing the drain (`--cfg loom`).
- `lvgl-rust/tests/lifecycle.rs`: `init`, ticks, `deinit` (terminal, so in
  its own process).
- `lvgl-rust/size-gate/gate.sh`: links firmware for thumbv6m and riscv32imc
  (16 slots and views-only) and checks the crate's symbols: registry size and
  `.bss` placement, no `.data`, no `core::fmt` or panic code, the OBJ_ID
  hooks, and a code budget per target (set in the `size-gate` task). Raise a
  budget only in the change that needs it, and say why.

Rules for integration tests:

- Always start nested builds with `common::cargo()`. It sets `--offline`,
  `CARGO_TERM_COLOR=never` (CI forces colors, which breaks matching on
  `Fresh lvgl_rust_sys`), the vendored config, and clears `LVGL_FONTS_DIR` and
  `LVGL_CFLAGS` inherited from the outer environment.
- Use `common::scratch_dir()` for fixtures and separate target directories, and
  remove them at the end of the test.
- A fixture with its own `Cargo.lock` needs an empty `[workspace]` table (so
  Cargo does not attach it to the root workspace) and a matching `cargo fetch` in the
  `fetch` task of `mise.toml` (which CI runs), otherwise CI fails with "no
  matching package".
- Restore any tracked file a test modifies, even on failure (see `RestoreFile`
  in `tests/incremental_font_build.rs`).

## CI

`.github/workflows/ci.yml` runs with `RUSTFLAGS=-D warnings`. Every job
installs the toolchain from `mise.toml` (`jdx/mise-action`); jobs that compile
use `jdx/mr-boxington-action` to persist the mbx cache and run Cargo
through `mise exec` (mise routes it through mbx, so builds never call `mbx` directly).
Caches are saved only on pushes to `main`; pull requests restore them. It
checks:

- `cargo fmt --check` for the workspace, the example and the fixture;
- clippy on `lvgl_rust_sys` (default, `--all-features`,
  `--no-default-features`) and `lvgl-rust-gen`;
- `lvgl-rust`: clippy, tests, views-only mode, ASan, Miri and loom, plus a
  `wasm32-unknown-unknown` build and the size gate;
- the full test suite with default features, plus library and build-config
  tests for `shadcn-theme`, `color_depth_32`, `color_depth_8` and without string
  functions;
- cross builds for `wasm32-unknown-unknown` and `riscv32imc-unknown-none-elf`;
- `cargo check` with the minimum Rust version (`mise run msrv`) on the host
  and both cross targets; raise the version in the task and in the root
  `rust-version` together;
- `cargo doc` in docs.rs mode (`DOCS_RS=1`, empty `DEP_LV_CONFIG_PATH`).

Before pushing, run at least:

```sh
cargo fmt --all --check
RUSTFLAGS="-D warnings" cargo clippy -p lvgl_rust_sys -p lvgl-rust-gen --all-targets
RUSTFLAGS="-D warnings" cargo clippy -p lvgl_rust_sys --all-targets --all-features
RUSTFLAGS="-D warnings" cargo test -p lvgl_rust_sys --lib --test build_config
```

or `mise run check`, which runs the same checks plus clippy for every feature
set and the `lvgl-rust` clippy and tests.

## Generated wrappers

`lvgl-rust/build.rs` runs `lvgl-rust-gen` on the bindings of the current
build (`DEP_LVGL_RUST_SYS_BINDINGS`) and writes `$OUT_DIR/gen.rs`, included
as the `widgets`, `enums`, `functions` and `props` modules, plus `$OUT_DIR/coverage.txt`
listing what happened to every bound function. Disabled widgets are simply
absent from the bindings, so nothing is generated for them; `lv_use_<x>`
cfgs mirror the `LV_USE_*` values for hand-written code.

- **A function is wrapped only if `access.toml` lists it**, as `shared`
  (view method), `exclusive` (`ObjMut` method, or `&mut Lvgl`), `pure`/`isr`
  (free function) or `raw` (stays in `sys`). `access.toml` is generated by
  the audit (below): never edit it by hand.
- Pointer parameters are retained (raw) until `overrides/` classifies them
  (`copied_str`, `static_str`, `copied`, ...). `@nullable` in the header gives
  `Option`. Object returns are untyped (`&Obj`) unless an override certifies
  the class.
- Styles and fonts are typed by the generator, without overrides:
  `lv_style_<x>(lv_style_t *, ...)` becomes a `StyleMut` method (exclusive
  whatever the audit says, since objects may be reading the style),
  `const lv_style_t *` parameters take a `'static` `Style` or `ConstStyle`
  (`StyleRef`), `const lv_font_t *` ones a `&'static Font`, and
  `lv_style_selector_t` a `Selector`. `lv_font_t *` (writable) stays raw:
  fonts may sit in flash. Each `lv_style_set_<p>` whose value fits a
  constant also gets a `props::<p>` constructor for `const_style!`, and
  each `lv_font_t` static of the bindings a `const` `Font` constructor.
- Data LVGL keeps and reads later (image sources, button maps, gradients,
  transitions, color filters, color keys) is mapped with
  `kind = "retained", rust = "<Type>"` to a type of `lvgl-rust` that makes
  it `'static` and checks its layout (terminators, sizes, first bytes):
  `RETAINED_TYPES` in `tables.rs` lists them, each with a `const fn
  as_ptr`. Data LVGL writes stays raw. Grid templates and scale labels are
  raw on purpose: LVGL reads past them for some cell positions or tick
  counts (`lv_grid.c:321-325`, `lv_scale.c:1849, 1988`).
- A string getter that points into the object stays raw until an override
  says `returns = "borrowed_internal"` (after checking that the pointer is
  the object's own text, a literal or static text it was given, never
  something else such as an image source). It becomes a copy-out
  `x_into(&self, buf)` on the view, since a shared setter could free the
  string, and a borrowing `x(&mut self)` on `ObjMut`.
- `classes.toml` `crate_owned` functions can never be shared or exclusive;
  `manual` ones are wrapped by hand in `lvgl-rust/src/base/`.
- `classes.toml` `[composite.<widget>]` tables drive the structural checks:
  `parts` are the objects LVGL holds pointers to (getters, `child:N`,
  `shim:`), `indexed` how many levels below a part LVGL finds objects by
  index, and `parts = []` declares a widget whose children are all the
  application's. Inside a widget class with no entry, nothing can be
  deleted, owned, moved, reordered or cleaned, and no child can be created
  (widgets cast or count their children: a tileview reads every child as
  a tile). Subclasses inherit their base's entry (`lv_obj_has_class`). Add an entry only after reading the
  widget's sources (constructor, cached pointers, `lv_obj_get_child` uses).
- Pointer parameters stay raw, and index parameters (`idx`, `*_id`, `row`,
  `sel_opt`, ...) too, until an override gives their kind with a `why`
  citing the lines that copy, write or bounds-check them (a check under
  `LV_USE_CHECK_ARG` or `LV_ASSERT` does not count: configurations turn
  them off). Functions in `relations.toml` stay raw until relations are
  supported.
- Table errors always fail the build. Entries for functions missing from a
  build's bindings warn (they may be disabled by that `lv_conf.h`) unless
  `universe.txt` lists them; `LVGL_RUST_GEN_STRICT=1`, set for every mise
  task, turns those warnings into errors.
- After changing the generator, rerun `UPDATE_GOLDEN=1 cargo test -p
  lvgl-rust-gen` and review the diff of `tests/golden/`.
- On an LVGL bump, add `tables/lvgl-<major>.<minor>/`: the generator refuses
  bindings without matching tables. Then run `mise run bump`: it rewrites
  `universe.txt` from the bindings built with `lvgl-rust/conf` and lists
  `[hierarchy]` lines that disagree with the `.base_class` of the LVGL
  sources, and stale table entries (functions or parameters that no
  longer exist). Fix those by hand, then run `mise run audit`.
  `audit-check` runs `bump --check`.

## Access audit

`mise run audit` (`lvgl-rust-gen audit`, feature `cli`) compiles the vendored
LVGL with `lvgl-rust/conf` to LLVM IR (pinned clang from mise, fixed
`x86_64-unknown-none-elf` target, so the result does not depend on the
host), builds a call graph and rewrites `access.toml`; CI runs
`mise run audit-check`. A function that can reach a sink (`sinks` in
`audit.toml`: deleting objects or displays, shrinking children arrays,
freeing group nodes) is exclusive, with the witness path as `why`; one that
schedules a deletion for later is raw; anything else is shared.

The graph is conservative; `audit.toml` makes it precise without losing
soundness:

- callbacks are tracked per struct field (`_lv_obj_class_t.3`, ...), through
  setters' parameters; an indirect call reaches only what LVGL stores in
  that field, and an unresolved one every escaping function of its
  signature;
- event sends with constant codes reach only the handlers of that code, and
  `if (code == ...)` guards in handlers are followed;
- widget code reaches only handlers registered by core code, by itself, by
  widgets it creates or that create it, and by its `peers`;
- animation callbacks are attributed to whoever installs them
  (`caller_slots`), and `lv_anim_delete` with a constant `exec_cb` reaches
  only the `deleted_cb`s installed with it (`anim_delete`);
- `exceptions` holds hand decisions (pure functions), with a reason.

Change `audit.toml` only with a source-backed reason in a comment; run
`mise run audit -- --verbose` to list handlers and class callbacks that
reach a sink, and review the `access.toml` diff.

## Pitfalls

- **Shared versus exclusive in `lvgl-rust`.** A view (`&Obj`) borrows
  `&Lvgl`; any call that may free an object, start deleting one, shrink a
  children array or free memory LVGL objects point to must take `&mut Lvgl`
  or an `ObjMut`, never `&self` on a view. Deletions requested from shared
  code go through the registry (`delete_later`, `Owned::drop`) and run in the
  drain.
- **Registry atomics.** No read-modify-write anywhere (thumbv6m and
  riscv32imc have no compare-and-swap): code without a token only stores
  `req`, `any_req` and `ticks`. `Registry::request_drop` and `drain` pair
  SeqCst fences; keep that pairing if either side changes, and rerun
  `mise run loom`.
- **New LVGL calls in the core** go through `base/ffi.rs`, with a matching
  model in `base/ffi_stub.rs`, or Miri cannot run the tests.
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
- **Cross-compilation.** The build script never passes `--target` to clang:
  for non-host targets bindgen derives the clang triple from `TARGET` itself
  (`riscv32imc-unknown-none-elf` becomes `riscv32-unknown-none-elf`, `espidf`
  becomes `elf`, `aarch64-apple-*` becomes `arm64-apple-*`). The C compiler
  comes from `cc`, which honours `CC_<target>`/`CFLAGS_<target>`; give bindgen
  the same headers with `BINDGEN_EXTRA_CLANG_ARGS_<target>` (see the
  `cross-riscv32` task). Targets without a C library (`wasm*-unknown-unknown`)
  get `shims/freestanding/` on every include path, for `inttypes.h`. The
  root `[env]` of `mise.toml` points `cc` and bindgen at the Debian/Ubuntu
  riscv32 and arm-none-eabi toolchains. CI builds
  `wasm32-unknown-unknown` and `riscv32imc-unknown-none-elf` (the riscv C
  toolchain is declared in `[bootstrap.packages]` of `mise.toml`, installed
  with `mise bootstrap packages apply`); host builds do
  not prove that a change works on other targets (xtensa included), so say
  what you actually checked.
- **Submodules.** Never edit files under `lvgl/` or `lvgl_theme_shadcn/`. Fix
  problems in `build/`, `shims/` or `conf/`, or bump the pinned commit.
- **Generated files.** `bindings.rs` and `liblvgl.a` live in `target/`; never
  commit build output. The `*.generated.c` font fixtures are small hand-written
  stand-ins for `lv_font_conv` output.

## Conventions

- Rust 2024 edition, MSRV 1.89 (`rust-version` in the root `Cargo.toml`).
  Format with `cargo fmt`; match the surrounding style and comment density.
- Commit messages follow `type: summary` (`build:`, `conf:`, `fix:`, `test:`,
  `docs:`, `ci:`, `feat:`), with a body explaining the why for anything
  non-trivial.
- Keep pull requests focused, one concern each. For larger work, prefer a
  stack of small PRs, each targeting the previous branch.
- Do not add AI attribution (no `Co-Authored-By` trailers for assistants and
  no "generated with" lines in commits or pull requests).
- Document user-visible changes (features, environment variables, metadata) in
  `README.md`, and keep this file up to date when the workflow changes.
