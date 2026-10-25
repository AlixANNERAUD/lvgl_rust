# lvgl-rust-sys — Rust FFI bindings for LVGL 9

[![CI](https://github.com/AlixANNERAUD/lvgl_rust/actions/workflows/ci.yml/badge.svg)](https://github.com/AlixANNERAUD/lvgl_rust/actions/workflows/ci.yml)
[![LVGL](https://img.shields.io/badge/LVGL-9.6.0-blue)](https://github.com/lvgl/lvgl)
[![License: MIT](https://img.shields.io/badge/license-MIT-green.svg)](../LICENSE)
[![no_std](https://img.shields.io/badge/no__std-yes-orange)](#)
[![MSRV](https://img.shields.io/badge/MSRV-1.89-informational)](#)

**Low-level, `no_std` Rust bindings to [LVGL](https://lvgl.io)** (Light and
Versatile Graphics Library), the most popular open-source embedded GUI library.
Build user interfaces for microcontrollers, embedded Linux, WebAssembly and
desktop simulators from Rust. LVGL is compiled from source by Cargo and the
bindings are generated with bindgen for your exact configuration.

- 🦀 **`no_std` and allocator-agnostic**: runs on bare-metal MCUs (ESP32, STM32,
  RP2040, ...), Linux, WebAssembly/Emscripten and desktop.
- ⚙️ **Your `lv_conf.h`, your bindings**: the C library, the Rust bindings and
  dependent crates are always built from the same configuration.
- 🎨 **Color depth as a Cargo feature**: 1, 8, 16 (RGB565), 24 or 32 bits.
- 🔤 **Incremental font builds**: compile `lv_font_conv` fonts in their own
  crate. Changing a font never recompiles LVGL.
- 🌗 **Optional shadcn/ui theme** for a modern look out of the box.
- 📦 **LVGL 9.6.0** vendored as a Git submodule.

> This is a `-sys` crate: it exposes the raw C API (`lv_obj_create`,
> `lv_label_set_text`, ...). For a safe, idiomatic API on top of it, use
> [`lvgl-rust`](../lvgl-rust).

## Quick start

Add the crate (it is consumed from Git):

```toml
[dependencies]
lvgl-rust-sys = { git = "https://github.com/AlixANNERAUD/lvgl_rust" }
```

Point the build at the directory that contains your `lv_conf.h`, for example in
`.cargo/config.toml`:

```toml
[env]
DEP_LV_CONFIG_PATH = { value = "lvgl-config", relative = true }
```

Then call LVGL like you would in C:

```rust
use lvgl_rust_sys::*;

unsafe {
    lv_init();
    let display = lv_display_create(320, 240);
    // lv_display_set_flush_cb(display, Some(flush)); lv_display_set_buffers(...)
    let label = lv_label_create(lv_display_get_screen_active(display));
    lv_label_set_text(label, c"Hello from Rust!".as_ptr());
    lv_obj_center(label);

    loop {
        lv_tick_inc(5);
        lv_timer_handler();
        // sleep 5 ms
    }
}
```

**Requirements:** a C compiler for your target, `libclang` (for bindgen), and the
Git submodules (`git submodule update --init --recursive`) when building from a
checkout.

## Cargo features

| Feature | Default | Description |
| --- | :---: | --- |
| `build-lvgl` | ✅ | Compile and link the LVGL C library. Disable it to only generate bindings (e.g. when LVGL is linked elsewhere). |
| `use-string-functions` | ✅ | Provide `strlen`, `strcmp`, `strcpy`, ... in Rust for targets without a libc. |
| `color_depth_1` … `color_depth_32` | | Select the color depth (`1`, `8`, `16`, `24`, `32`). See [Color depth](#color-depth). |
| `shadcn-theme` | | Compile the [lvgl_theme_shadcn](https://github.com/AlixANNERAUD/lvgl_theme_shadcn) theme and its bindings. |
| `use-vendored-config` | | Fall back to the bundled `conf/lv_conf.h` when `DEP_LV_CONFIG_PATH` is unset. |

## Configuration

| Environment variable | Description |
| --- | --- |
| `DEP_LV_CONFIG_PATH` | Directory containing `lv_conf.h`. Required unless `use-vendored-config` is enabled or building on docs.rs. |
| `LVGL_CFLAGS` | Extra comma-separated `NAME` or `NAME=VALUE` definitions passed to the C compiler **and** bindgen. |
| `LVGL_FONTS_DIR` | Legacy in-crate fonts, see [Fonts](#fonts). |
| `EMSDK` | Emscripten SDK root, required for `*-emscripten` targets. |

The build reruns whenever one of these variables, `lv_conf.h`, the LVGL sources
or the shims change. The bundled `conf/lv_conf.h` is LVGL's template with
content enabled, NULL/malloc asserts on and an overridable color format. It is a
good starting point for your own configuration.

### Cross-compiling

The C compiler comes from [`cc`](https://docs.rs/cc) (`CC_<target>`,
`CFLAGS_<target>`) and bindgen picks the clang triple from Cargo's target. If
the C library headers are not found by clang on its own, pass them with
`BINDGEN_EXTRA_CLANG_ARGS_<target>`, e.g. for `riscv32imc-unknown-none-elf`
with Debian's picolibc:

```sh
export CC_riscv32imc_unknown_none_elf=riscv64-unknown-elf-gcc
export CFLAGS_riscv32imc_unknown_none_elf="-isystem /usr/lib/picolibc/riscv64-unknown-elf/include"
export BINDGEN_EXTRA_CLANG_ARGS_riscv32imc_unknown_none_elf="$CFLAGS_riscv32imc_unknown_none_elf"
```

`wasm32-unknown-unknown` has no C library: the crate provides the one header
LVGL needs (`inttypes.h`), so only clang is required.

### Color depth

The `color_depth_*` features set both `LV_COLOR_FORMAT_DEFAULT` and
`LV_COLOR_DEPTH` for the native build, bindgen and dependent crates. Your
`lv_conf.h` must not hard-code them (guard them with `#ifndef`, like the bundled
config does). Without any of these features, `lv_conf.h` decides. If several are
enabled (e.g. `--all-features`), the smallest wins and a build warning is
emitted.

### shadcn theme

Enable `shadcn-theme` to compile the pinned
[lvgl_theme_shadcn](https://github.com/AlixANNERAUD/lvgl_theme_shadcn) C theme
with the same headers, color depth, allocator and `lv_conf.h` as LVGL. It does
not build a second LVGL or add a Rust runtime dependency.

Initialize it with `lv_theme_shadcn_init`, switch palettes with
`lv_theme_shadcn_set_palette` and use its surface/variant helpers instead of
local color overrides. Theme state is released when its display is deleted, or
explicitly with `lv_theme_shadcn_deinit`. All calls follow LVGL's
synchronization requirements.

## Using LVGL's configuration from another crate

Thanks to `links = "lvgl_rust_sys"`, dependent build scripts can compile
ABI-compatible C code (fonts, drivers, custom widgets) against the same headers
and definitions:

| Variable | Content |
| --- | --- |
| `DEP_LVGL_RUST_SYS_INCLUDE_PATHS` | `;`-separated include directories |
| `DEP_LVGL_RUST_SYS_DEFINES` | `;`-separated `NAME` / `NAME=VALUE` definitions (config, color depth) |
| `DEP_LVGL_RUST_SYS_CFLAGS` | `LVGL_CFLAGS` entries, `,`-separated |
| `DEP_LVGL_RUST_SYS_TARGET` | Cargo target triple of the native build |
| `DEP_LVGL_RUST_SYS_BINDINGS` | Absolute path to the generated `bindings.rs` |
| `DEP_LVGL_RUST_SYS_LINKED` | `1` when the `build-lvgl` feature compiled the C library, `0` otherwise |

Use the target-aware compiler from `cc` in the dependent build script. Do not
substitute the host target or guess ABI-affecting defines yourself.
[`examples/external-fonts/build.rs`](examples/external-fonts/build.rs) shows a
complete consumer.

## Helper functions

Besides the LVGL API, the bindings include a few C helpers from `shims/` for
state that LVGL keeps in private structures:

| Function | Purpose |
| --- | --- |
| `lvgl_sys_obj_is_deleting` | Whether an object is being deleted. |
| `lvgl_sys_obj_set_id_raw` | Store an object's id without running the OBJ_ID hooks (with `LV_USE_OBJ_ID`). |
| `lvgl_sys_image_cache_drop` | Drop cached decodes of an image source (`lv_image_cache_drop`). |
| `lvgl_sys_menu_part` | Cached parts of a menu, without the deprecation log of the menu getters; sidebar parts are NULL once the sidebar is removed (with `LV_USE_MENU`). |

## Fonts

**Recommended: a separate font crate.** Build the C sources generated by
`lv_font_conv --format lvgl` in their own crate, using the metadata above.
Editing a font then only rebuilds that small archive and relinks; LVGL and
bindgen are not rerun.

```sh
cargo run --manifest-path lvgl-rust-sys/examples/external-fonts/Cargo.toml -- --cjk
```

Give each font profile a unique `--lv-font-name` (for example
`font_montserrat_14_latin` and `font_montserrat_14_cjk`) so profiles can coexist
in one firmware. The Rust side exposes each descriptor as a `*const lv_font_t`
that the application can pass to LVGL, e.g. when switching languages.

Generated descriptors and glyph/map/kerning tables are `const` data and stay in
Flash/ROM: no file loading during redraw and no copy of glyph data into RAM.
Position-independent host executables may place pointer-bearing descriptors in
`.data.rel.ro` (read-only after relocation). On embedded targets, check the
final placement in your ELF/map file.

**Legacy: `LVGL_FONTS_DIR`.** When set, the `.c` files of that directory are
compiled into the LVGL archive and its top-level headers are passed to bindgen.
Any font change rebuilds LVGL and regenerates the bindings. A `fonts/` directory
in the working directory is no longer picked up implicitly.

## Development

The crate lives in the `lvgl-rust-sys/` directory of a Cargo workspace. From
the repository root:

```sh
git submodule update --init --recursive
# Nested builds run offline.
cargo fetch --manifest-path lvgl-rust-sys/examples/external-fonts/Cargo.toml
cargo fetch --manifest-path lvgl-rust-sys/tests/fixtures/font_incremental/Cargo.toml
cargo test -p lvgl-rust-sys            # uses conf/ via the root .cargo/config.toml
cargo test -p lvgl-rust-sys --features shadcn-theme,color_depth_32
cargo clippy -p lvgl-rust-sys --all-targets --all-features
```

The repository also ships a [mise](https://mise.jdx.dev) configuration that
installs the Rust toolchain with
[mr boxington](https://github.com/jdx/mr-boxington) (`mbx`), a build cache that
avoids recompiling LVGL, and defines the CI checks as tasks (needs mise
2026.9.2 or newer):

```sh
mise install
mise tasks             # list the tasks
mise run check         # fmt, clippy and the fast tests, as before a push
mise run test          # full suite
mise run coverage      # lcov report of the three crates (coverage.lcov)
mise run package       # package and build the crates as crates.io would
```

- `build/main.rs` drives `cc`, bindgen and the exported metadata from a single
  `NativeConfig`, so all of them share the same include paths and definitions.
- `build/config.rs` holds the pure logic, unit tested by `tests/build_config.rs`.
- The other integration tests build fixture crates with a nested `cargo`. They
  need `nm` and `objdump` and take a few minutes, because each one compiles LVGL.

## License

MIT, see [LICENSE](../LICENSE). LVGL is distributed under its own
[MIT license](https://github.com/lvgl/lvgl/blob/master/LICENCE.txt).
