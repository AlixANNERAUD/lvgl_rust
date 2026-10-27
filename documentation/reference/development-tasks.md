# Development tasks

The [mise](https://mise.jdx.dev) tasks of `mise.toml` (mise 2026.9.2 or
newer) and the GitHub workflows. `mise install` installs the toolchain;
`mise run <task>` runs a task; `mise tasks` lists them. Cargo runs through
mbx ([mr boxington](https://github.com/jdx/mr-boxington)), a build cache
that restores LVGL's C objects and build script output. The C toolchains
of the embedded targets are installed with
`mise bootstrap packages apply`.

`mise.toml` sets `LVGL_RUST_GEN_STRICT=1` for every task, and the
compiler and flags of the riscv32 and arm-none-eabi targets
([Supported targets](supported-targets.md)). Tasks that build
`lvgl-rust` set `DEP_LV_CONFIG_PATH` to a configuration with the OBJ_ID
hooks and use their own target directory. Most tasks set
`RUSTFLAGS=-D warnings`, as CI does.

## Tasks

| Task | Runs | CI |
| --- | --- | --- |
| `submodules` | `git submodule update --init --recursive`. | CI checks out submodules itself. |
| `fetch` | `cargo fetch` for `lvgl-rust-sys/examples/external-fonts` and `lvgl-rust-sys/tests/fixtures/font_incremental`, whose nested builds run offline. | `ci.yml` Test (every matrix entry). |
| `fmt` | `cargo fmt --all --check`; `rustfmt --check` on the compile-fail and compile-pass cases; `cargo fmt --check` for the external-fonts example, the incremental-font fixture and `assets/social-banner/screen`. | `ci.yml` Format. |
| `clippy` | Clippy on `lvgl-rust-sys` and `lvgl-rust-gen` (all targets), `lvgl-rust-sys` with all features, `lvgl-rust-sys` without default features. | `ci.yml` Clippy (the same three commands, one per matrix entry). |
| `test-fast` | `cargo test -p lvgl-rust-sys --lib --test build_config`. | `release.yml` validate. |
| `test` | `fetch`, then `cargo test -p lvgl-rust-sys -p lvgl-rust-gen` (full suite, integration tests included). | `ci.yml` Test (default). |
| `package` | `cargo package` of the three crates, each built from its package (`lvgl-rust/conf`, mbx build-script cache off). | `ci.yml` Package; `release.yml` validate. |
| `coverage` | `cargo llvm-cov` over `lvgl-rust-sys` (library and build-config tests), `lvgl-rust-gen`, and `lvgl-rust` with `async`, `heap`, `fmt` (`lvgl-rust/conf`), merged into `coverage.lcov`. Integration tests excluded. | `coverage.yml`. |
| `social-banner` | Renders `assets/social-banner/social-banner.png` (`assets/social-banner/render.sh`, resvg 0.49.0; downloads its fonts). | — |
| `cross-wasm32` | Builds `lvgl-rust-sys` and `lvgl-rust` for `wasm32-unknown-unknown`. | `ci.yml` Cross (wasm32-unknown-unknown). |
| `cross-wasip1` | With wasi-sdk-33 and wasmtime 50.0.0: `lvgl-rust-sys` library tests, `lvgl-rust` tests (`async`, `heap`, `fmt`) and the desktop example for `wasm32-wasip1`, run under wasmtime. | `ci.yml` Cross (wasm32-wasip1). |
| `cross-riscv32` | Builds `lvgl-rust-sys` for `riscv32imc-unknown-none-elf`. | `ci.yml` Cross (riscv32imc-unknown-none-elf). |
| `cross-arm` | Builds the size-gate firmware for `thumbv7em-none-eabihf` (handles and views only) and `thumbv8m.main-none-eabihf` (handles), size profile. | `ci.yml` Cross (thumbv7em and thumbv8m.main firmware). |
| `size-gate` | `lvgl-rust/size-gate/gate.sh` for `thumbv6m-none-eabi` and `riscv32imc-unknown-none-elf`: 16 slots and views only (code budgets 4608 B and 2304 B on thumbv6m, 5888 B and 2944 B on riscv32imc), then with `defmt` and `debug-counters` (thumbv6m, 16 slots, 4608 B), `debug-counters` (thumbv6m views only, 2368 B; riscv32imc 16 slots, 5888 B); then `cargo check` of `lvgl-rust` with `async` and `heap` for both targets. | `ci.yml` Cross (thumbv6m and riscv32imc firmware). |
| `overhead` | `lvgl-rust/benches/overhead-report.sh`: the `overhead` benchmark (release, feature `heap`), then `size-gate`, then writes [`documentation/reference/overhead.md`](overhead.md). Without the embedded C toolchains, it takes the crate's sizes from the size-gate job of CI. | — (host times vary between machines) |
| `msrv` | With Rust 1.89: `cargo check --workspace --all-targets`, and `cargo check` of `lvgl-rust-sys` and `lvgl-rust` for `wasm32-unknown-unknown` and `riscv32imc-unknown-none-elf` (`lvgl-rust/conf`). | `ci.yml` MSRV. |
| `clippy-safe` | Clippy on `lvgl-rust` and the size-gate firmware, all targets, features `async`, `heap`, `fmt`, `defmt`, `debug-counters`. | `ci.yml` lvgl-rust (clippy-safe). |
| `test-safe` | `cargo test -p lvgl-rust` with `async`, `heap`, `fmt`, `defmt`, `debug-counters`. | `ci.yml` lvgl-rust (test-safe); `release.yml` validate. |
| `test-views-only` | With `LVGL_RUST_HANDLE_SLOTS=0` and `lvgl-rust-sys/conf`: clippy and tests without features, with `defmt` and `debug-counters`, and with `async` and `heap`. | `ci.yml` lvgl-rust (test-views-only). |
| `test-asan` | With nightly Rust, clang and `-fsanitize=address`, on a copy of `lvgl-rust/conf` using the C library's `malloc`: `lvgl-rust` tests (`async`, `heap`, `fmt`) and the desktop example. | `ci.yml` lvgl-rust (test-asan). |
| `example` | `cargo run -p lvgl-rust --example first_application` (the [tutorial](../tutorials/first-application.md)'s program), then `--example desktop` (a scripted desktop shell). | `ci.yml` lvgl-rust (example). |
| `test-matrix` | `lvgl-rust` tests with `color_depth_16`, and with `color_depth_32` and `shadcn-theme` (plus `async`, `heap`, `fmt`). | `ci.yml` lvgl-rust (test-matrix). |
| `test-compile-fail` | With Rust 1.99.1: the trybuild cases of `lvgl-rust/tests/compile-fail` (features `async`, `heap`). | `ci.yml` lvgl-rust (test-compile-fail). |
| `test-allocator` | On a copy of `lvgl-rust/conf` with `LV_STDLIB_CUSTOM`: clippy and the `allocator` test with `allocator` and `heap`. | `ci.yml` lvgl-rust (test-allocator). |
| `miri` | With nightly Rust: the `lvgl-rust` library tests under Miri against the Rust model of LVGL, without features and with `heap`. | `ci.yml` lvgl-rust (miri). |
| `loom` | `--cfg loom`, `LVGL_RUST_HANDLE_SLOTS=4`: the `protocol` tests of `lvgl-rust` in release. | `ci.yml` lvgl-rust (loom). |
| `audit` | With clang 22.1.8: `lvgl-rust-gen audit`, rewriting `access.toml` and `events.toml`. | — (`audit-check` checks it) |
| `bump` | `lvgl-rust-gen bump`: rewrites `universe.txt`, reports hierarchy and stale table entries. | — (`audit-check` checks it) |
| `audit-check` | Clippy and tests of `lvgl-rust-gen` with `cli`, then `audit --check` and `bump --check`. | `ci.yml` Access audit. |
| `doc` | `cargo doc` of `lvgl-rust-sys` (no default features) and `lvgl-rust` in docs.rs mode (`DOCS_RS=1`, empty `DEP_LV_CONFIG_PATH`, `RUSTDOCFLAGS=-D warnings`). | `ci.yml` Docs (docs.rs mode). |
| `check` | `fmt`, `clippy`, `test-fast`, `clippy-safe`, `test-safe`, one after the other. Run before pushing. | — |

## Workflows

| Workflow | Trigger | Jobs |
| --- | --- | --- |
| `ci.yml` | Pushes to `main`, pull requests, manual dispatch. Concurrent runs of a ref are cancelled. `RUSTFLAGS=-D warnings`. | Format (`fmt`); Clippy (default, all features, bindgen only); Test (default, shadcn theme, color depth 32, color depth 8, no string functions); Access audit (`audit-check`); lvgl-rust (`clippy-safe`, `test-safe`, `test-views-only`, `test-asan`, `test-allocator`, `test-matrix`, `test-compile-fail`, `example`, `miri`, `loom`); Cross (`cross-wasm32`, `cross-wasip1`, `cross-riscv32`, `size-gate`, `cross-arm`, after `mise bootstrap packages apply`); MSRV (`msrv`); Docs (`doc`); Package (`package`). |
| `coverage.yml` | Pushes to `main`, pull requests. | `coverage`, then uploads `coverage.lcov` to Codecov (`CODECOV_TOKEN`). |
| `release.yml` | Pushes to `main` (repository owner only). | validate (`test-fast`, `test-safe`, `package`); then release-plz `release` (publishes the crates, `CARGO_REGISTRY_TOKEN`) and `release-pr` (keeps the release pull request up to date), both with `DEP_LV_CONFIG_PATH` set to `lvgl-rust/conf`. |
| `pr-title.yml` | Pull requests opened, edited, reopened or synchronised. | Checks that the title follows Conventional Commits: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `ci`, `build`, `conf`, `chore`, `style` or `revert`, an optional `(scope)`, an optional `!`, then `: ` and a summary. release-plz builds the changelogs from these titles. |

Every compiling job installs the toolchain with `jdx/mise-action` and
persists the mbx cache with `jdx/mr-boxington-action`.
