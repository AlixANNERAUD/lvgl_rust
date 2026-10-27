# Build for embedded targets

This guide cross-compiles LVGL and lvgl-rust for microcontrollers and
WebAssembly. Two tools need to know about the target: `cc`, which
compiles LVGL's C sources, and bindgen (libclang), which reads the same
headers to generate the bindings.

## What you need

- The Rust target, installed with `rustup target add <target>`.
- A C compiler for the target and the headers of a C library (newlib,
  picolibc, wasi-libc, ...), except for `wasm32-unknown-unknown`.
- `libclang` on the build machine, for bindgen.
- `lv_conf.h` configured as in [Configure LVGL](configure-lvgl.md).

## Steps

### 1. Tell `cc` and bindgen about the target

The build script never passes `--target` to clang: bindgen derives the
clang triple from Cargo's `TARGET` (`riscv32imc-unknown-none-elf` becomes
`riscv32-unknown-none-elf`). Three families of variables, all suffixed
with the target triple (dashes as underscores), do the rest:

| Variable | Read by | Purpose |
| --- | --- | --- |
| `CC_<target>`, `AR_<target>` | `cc` | The C compiler and archiver. |
| `CFLAGS_<target>` | `cc` | Extra C flags, usually the C library's include directory. |
| `BINDGEN_EXTRA_CLANG_ARGS_<target>` | bindgen | The same headers for libclang, which does not know the GCC toolchain's paths. |

Put them in `.cargo/config.toml` so every build sees them:

```toml
[env]
DEP_LV_CONFIG_PATH = { value = "lvgl-configuration", relative = true }
CC_thumbv7em_none_eabihf = "arm-none-eabi-gcc"
AR_thumbv7em_none_eabihf = "arm-none-eabi-ar"
CFLAGS_thumbv7em_none_eabihf = "-isystem /usr/include/newlib"
BINDGEN_EXTRA_CLANG_ARGS_thumbv7em_none_eabihf = "-isystem /usr/include/newlib"
```

The C definitions of the build (`lv_conf.h`, `LVGL_CFLAGS`, the color
depth) reach both tools from the same list, so the library and the
bindings always agree.

### 2. Use the recipe for your target family

The paths below are those of the Debian and Ubuntu packages, as in the
repository's `mise.toml`; adjust them for other systems.

**Arm Cortex-M (STM32, nRF52, RP2040, ...): `thumbv6m-none-eabi`,
`thumbv7em-none-eabihf`, `thumbv8m.main-none-eabihf`.** Install
`gcc-arm-none-eabi` and `libnewlib-arm-none-eabi`, then for each target:

```toml
[env]
CC_thumbv6m_none_eabi = "arm-none-eabi-gcc"
AR_thumbv6m_none_eabi = "arm-none-eabi-ar"
CFLAGS_thumbv6m_none_eabi = "-isystem /usr/include/newlib"
BINDGEN_EXTRA_CLANG_ARGS_thumbv6m_none_eabi = "-isystem /usr/include/newlib"
```

`thumbv8m.main-none-eabihf` has a dot in its name, which a POSIX shell
cannot export. Set its variables in `.cargo/config.toml`, with quoted
keys that keep the dot (`"CC_thumbv8m.main_none_eabihf"`), or use the
generic forms for a one-target build, as the `cross-arm` task does:
`TARGET_CC=arm-none-eabi-gcc`, `TARGET_AR=arm-none-eabi-ar`,
`TARGET_CFLAGS="-isystem /usr/include/newlib"` and
`BINDGEN_EXTRA_CLANG_ARGS="-isystem /usr/include/newlib"`.

**RISC-V microcontrollers (ESP32-C3, GD32V, ...):
`riscv32imc-unknown-none-elf`.** Install `gcc-riscv64-unknown-elf` and
`picolibc-riscv64-unknown-elf`:

```toml
[env]
CC_riscv32imc_unknown_none_elf = "riscv64-unknown-elf-gcc"
AR_riscv32imc_unknown_none_elf = "riscv64-unknown-elf-ar"
CFLAGS_riscv32imc_unknown_none_elf = "-isystem /usr/lib/picolibc/riscv64-unknown-elf/include"
BINDGEN_EXTRA_CLANG_ARGS_riscv32imc_unknown_none_elf = "-isystem /usr/lib/picolibc/riscv64-unknown-elf/include"
```

The 64-bit GCC builds 32-bit code from `TARGET`'s architecture flags,
which `cc` passes.

**ESP32, ESP32-S2, ESP32-S3 (Xtensa): `xtensa-*-none-elf` or
`xtensa-*-espidf`.** These need Espressif's Rust and LLVM toolchains
(installed with `espup`), whose `libclang` knows Xtensa, and the matching
GCC (`xtensa-esp32s3-elf-gcc`, ...) as `CC_<target>`. Give bindgen that
toolchain's C library headers with `BINDGEN_EXTRA_CLANG_ARGS_<target>`.
For `espidf` targets, bindgen turns the triple into an `elf` one. Neither
is built by the repository's CI: see step 5.

**WebAssembly without WASI: `wasm32-unknown-unknown`.** There is no C
library. lvgl-rust-sys puts its own `inttypes.h` (in
`lvgl-rust-sys/shims/freestanding/`) on the include path, so only a clang
that targets WebAssembly is needed, and no variable. Keep
`use-string-functions` on (step 3).

**WASI: `wasm32-wasip1`.** Use [wasi-sdk](https://github.com/WebAssembly/wasi-sdk);
its clang finds the SDK's sysroot, but bindgen's libclang needs it
explicitly:

```sh
export CC_wasm32_wasip1=$WASI_SDK/bin/wasm32-wasip1-clang
export AR_wasm32_wasip1=$WASI_SDK/bin/llvm-ar
export BINDGEN_EXTRA_CLANG_ARGS_wasm32_wasip1="--sysroot=$WASI_SDK/share/wasi-sysroot"
```

lvgl-rust needs no threads, so the whole crate works there.

**Emscripten: `wasm32-unknown-emscripten`.** Activate the Emscripten SDK
so that `EMSDK` is set (the build fails without it). The build script
adds Emscripten's libc and SDL include directories to bindgen's
arguments; `cc` uses `emcc`.

### 3. Decide on `use-string-functions`

The feature (on by default, in both crates) links Rust implementations of
`strlen`, `strcmp`, `strcpy` and the other C string functions LVGL may
call, for targets without a C library. Keep it on for bare-metal targets
linked without libc and for `wasm32-unknown-unknown`. If your firmware
links a full C library that already defines them, turn it off:

```toml
lvgl-rust = { git = "https://github.com/AlixANNERAUD/lvgl_rust", default-features = false }
```

Cargo unifies features: a crate of the graph that depends on
`lvgl-rust-sys` with its default features (a font crate, for example)
turns it back on, so give such dependencies `default-features = false`
too.

### 4. Build

```sh
cargo build --release --target thumbv7em-none-eabihf
```

Bare-metal firmware also needs what any `no_std` binary needs: a runtime
crate and a linker script for your chip, a panic handler, and a lock as
in [Share LVGL between threads](share-lvgl-between-threads.md). The
[`size-gate` firmware](../../lvgl-rust/size-gate/src/main.rs) is a minimal
`no_main` program that links for `thumbv6m` and `riscv32imc`.

### 5. Know what is verified

The repository's CI (`.github/workflows/ci.yml`) runs on Linux and checks:

| Target | What CI does |
| --- | --- |
| `wasm32-wasip1` | Builds both crates with wasi-sdk and runs the `lvgl-rust-sys` library tests, the `lvgl-rust` tests and the desktop example under wasmtime (`mise run cross-wasip1`). |
| `wasm32-unknown-unknown` | Builds `lvgl-rust-sys` and `lvgl-rust` (`mise run cross-wasm32`). |
| `riscv32imc-unknown-none-elf` | Builds `lvgl-rust-sys` (`cross-riscv32`); links the size-gate firmware with 16 handle slots and in views-only mode and checks its sizes (`size-gate`); checks `lvgl-rust` with `async` and `heap`. |
| `thumbv6m-none-eabi` | Links the size-gate firmware with 16 slots and views only and checks its sizes; checks `lvgl-rust` with `async` and `heap` (`size-gate`). |
| `thumbv7em-none-eabihf` | Builds the size-gate firmware with handles and in views-only mode (`cross-arm`). |
| `thumbv8m.main-none-eabihf` | Builds the size-gate firmware with handles (`cross-arm`). |

The minimum Rust version (1.89) is checked on the host,
`wasm32-unknown-unknown` and `riscv32imc-unknown-none-elf`. Only the
wasip1 build runs code; the embedded targets are built and measured, never
run on hardware. Xtensa, `espidf` and Emscripten targets are not built by
CI at all.

## Pitfalls

- **Link the compiler's support library.** LVGL compiled by GCC can call
  helpers such as `__gnu_thumb1_case_uqi` that live in `libgcc.a`. The
  size-gate firmware adds the path printed by `arm-none-eabi-gcc
  -print-libgcc-file-name` to the link arguments
  ([`size-gate/build.rs`](../../lvgl-rust/size-gate/build.rs)).
- **Same headers for both tools.** A `CFLAGS_<target>` without the
  matching `BINDGEN_EXTRA_CLANG_ARGS_<target>` gives bindings that fail
  on a missing `stdint.h` or, worse, disagree with the compiled library.
- **Ticks on targets without compare-and-swap.** On `thumbv6m` and
  `riscv32imc`, call `tick_inc` from one context only.
- **`DEP_LV_CONFIG_PATH` is needed for cross builds too**, and
  `color_depth_*` features apply to every target alike.
- **A host build proves nothing about another target.** Build for the
  target you ship, and link the final firmware: a library that compiles
  can still miss symbols at link time.

## See also

- [Supported targets](../reference/supported-targets.md)
- [Reduce the footprint](reduce-the-footprint.md)
- [Development tasks](../reference/development-tasks.md)
