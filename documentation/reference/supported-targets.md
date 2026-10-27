# Supported targets

What each target needs to build `lvgl-rust-sys` and `lvgl-rust`, and what
continuous integration checks for it. CI runs on GitHub's `ubuntu-latest`
Linux runners only (`.github/workflows/ci.yml`); a target without a CI
entry below is not built by CI, so nothing proves it keeps working.

## Requirements common to every target

- A C compiler for the target, found by [`cc`](https://docs.rs/cc)
  (`CC_<target>`, `CFLAGS_<target>`, `AR_<target>`), when `build-lvgl` is on
  (always for `lvgl-rust`).
- libclang for bindgen, which must find the target's C headers (pass them
  with `BINDGEN_EXTRA_CLANG_ARGS_<target>` when it does not). The build
  script never passes `--target`: bindgen derives the clang triple from
  Cargo's `TARGET`.
- Rust 1.89 or newer (`rust-version` of the workspace).
- Atomics: the crates need atomic loads and stores of 8, 16 and 32 bits,
  and no compare-and-swap. The registry, the debug counters and
  `HandleCell` use loads and stores only. `tick_inc` uses `fetch_add`
  where the target has 32-bit atomic read-modify-write
  (`target_has_atomic = "32"`), and a load and a store otherwise, where
  concurrent callers may lose ticks: call it from one context there.
- Little-endian targets for `ImageDsc::new`, `ImageBuffer::new` and
  `set_src_buffer` (they are `cfg(target_endian = "little")`).

## Targets

| Target | C toolchain | Compare-and-swap | CI |
| --- | --- | --- | --- |
| Linux host (`x86_64-unknown-linux-gnu` on the runners) | The host C compiler and libclang. | yes | Everything: `lvgl-rust-sys` tests (default features, plus library and build-config tests with `shadcn-theme`, `color_depth_32`, `color_depth_8`, and without string functions), clippy on three feature sets, `lvgl-rust` clippy and tests (`test-safe`), views-only mode, AddressSanitizer (nightly, LVGL built with clang), the `allocator` feature, 16-bit color, and 32-bit color with the shadcn theme, compile-fail cases (rustc 1.99.1), the first-application and desktop examples, Miri, loom, the access audit, MSRV check, docs in docs.rs mode, packaging, coverage. |
| macOS hosts | The host C compiler and libclang. For `aarch64-apple-*`, bindgen uses the `arm64-apple-*` clang triple. | yes | None. |
| Windows hosts | The host C compiler and libclang. The build script strips the `\\?\` prefix from canonical paths for C compilers. | yes | None. |
| `wasm32-unknown-unknown` | clang with a WebAssembly backend. No C library: `lvgl-rust-sys/shims/freestanding/` (for `inttypes.h`) is added to every include path, and `use-string-functions` provides the C string functions. | yes | `cross-wasm32`: builds `lvgl-rust-sys` (`lvgl-rust-sys/conf`) and `lvgl-rust` (`lvgl-rust/conf`). `msrv`: `cargo check` of both with Rust 1.89. |
| `wasm32-wasip1` | [wasi-sdk](https://github.com/WebAssembly/wasi-sdk): `CC_wasm32_wasip1=wasm32-wasip1-clang`, `AR_wasm32_wasip1=llvm-ar`, and `BINDGEN_EXTRA_CLANG_ARGS_wasm32_wasip1="--sysroot=<sdk>/share/wasi-sysroot"`. | yes | `cross-wasip1` (wasi-sdk-33 and wasmtime 50.0.0 as task tools): `lvgl-rust-sys` library tests and `lvgl-rust` tests (features `async`, `heap`, `fmt`) run under wasmtime, then the desktop example. Tests that spawn threads or catch panics are ignored there (WASI has no threads; wasm aborts on panic). |
| `*-emscripten` | The Emscripten SDK; `EMSDK` must point to it (the build script adds its libc, musl and SDL headers for bindgen). | — | None. |
| `thumbv6m-none-eabi` (Cortex-M0, M0+) | `gcc-arm-none-eabi` and `libnewlib-arm-none-eabi` (Debian/Ubuntu packages in `[bootstrap.packages]` of `mise.toml`); `CFLAGS` and bindgen get `-isystem /usr/include/newlib`. | no | `size-gate`: firmware with 16 handle slots and in views-only mode, plus 16 slots with `defmt` and `debug-counters` and views-only with `debug-counters`; checks the registry size and its `.bss` placement, no `.data`, no `core::fmt` or panic code, the OBJ_ID hooks, and a code budget. Also `cargo check` of `lvgl-rust` with `async` and `heap`. |
| `thumbv7em-none-eabihf` (Cortex-M4, M7) | As for thumbv6m. | yes | `cross-arm`: builds the size-gate firmware with handles (`lvgl-rust/conf`) and in views-only mode (`lvgl-rust-sys/conf`), size profile. No size checks. |
| `thumbv8m.main-none-eabihf` (Cortex-M33) | As for thumbv6m. The target name has a dot, so its compiler and flags go through `TARGET_CC`, `TARGET_AR`, `TARGET_CFLAGS` and `BINDGEN_EXTRA_CLANG_ARGS` (`ARM_NEWLIB_FLAGS` in `mise.toml`). | yes | `cross-arm`: builds the size-gate firmware with handles. No size checks. |
| `riscv32imc-unknown-none-elf` (ESP32-C3 class cores, other RV32IMC) | `gcc-riscv64-unknown-elf` and `picolibc-riscv64-unknown-elf`; `CFLAGS` and bindgen get `-isystem /usr/lib/picolibc/riscv64-unknown-elf/include`. bindgen uses the `riscv32-unknown-none-elf` clang triple. | no | `cross-riscv32`: builds `lvgl-rust-sys`. `size-gate`: firmware with 16 handle slots, in views-only mode and with 16 slots and `debug-counters`, with the checks above; `cargo check` of `lvgl-rust` with `async` and `heap`. `msrv`: `cargo check` of both crates with Rust 1.89. |
| Xtensa (`xtensa-esp32-*`, ESP32, ESP32-S2, ESP32-S3) | A Rust toolchain and C compiler with Xtensa support (not provided by `mise.toml`). For `*-espidf` targets, bindgen uses an `elf` clang triple. | depends on the chip; not needed | None. |

The size gate's measured sizes are in
[Overhead measurements](overhead.md#static-memory-in-firmware).

## Toolchains used by CI

| Tool | Version | Used by |
| --- | --- | --- |
| Rust | stable, with the cross targets `wasm32-unknown-unknown`, `wasm32-wasip1`, `riscv32imc-unknown-none-elf`, `thumbv6m-none-eabi`, `thumbv7em-none-eabihf`, `thumbv8m.main-none-eabihf` | every job |
| Rust | 1.89 (`wasm32-unknown-unknown`, `riscv32imc-unknown-none-elf`) | `msrv` |
| Rust | nightly (`rust-src`; `miri` for Miri) | `test-asan`, `miri` |
| Rust | 1.99.1 | `test-compile-fail` |
| clang | 22.1.8 | `audit`, `audit-check` |
| wasi-sdk, wasmtime | wasi-sdk-33, 50.0.0 | `cross-wasip1` |
| Debian/Ubuntu packages | `gcc-riscv64-unknown-elf`, `picolibc-riscv64-unknown-elf`, `gcc-arm-none-eabi`, `libnewlib-arm-none-eabi` (`mise bootstrap packages apply`) | cross and MSRV jobs |

See [Development tasks](development-tasks.md) for the tasks and the jobs
that run them.
