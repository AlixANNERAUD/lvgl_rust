# Cargo features

Every Cargo feature of the three crates. Memory costs come from
[Overhead measurements](overhead.md); environment variables are in
[Build configuration](build-configuration.md).

## lvgl-rust

| Feature | Default | Enables | Cost | LVGL configuration |
| --- | :---: | --- | --- | --- |
| `use-string-functions` | yes | `lvgl-rust-sys/use-string-functions`. | See [lvgl-rust-sys](#lvgl-rust-sys). | — |
| `async` | no | `StaticQueue`, `StaticRx`, `Receiver`, `Recv`, `Obj::subscribe`, `SubscribeOptions`, `next_event`; with `heap` too and handles enabled, `Obj::subscribe_owned` and `OwnedQueue`. No executor dependency, no atomics. | A queue lives in the application's `static`; an owned queue is an LVGL heap block (4 B header, then the queue) and its `OwnedQueue` receiver is 8 B. | — |
| `heap` | no | Closures and owned values on LVGL's own heap: `Obj::on_boxed`, `Lvgl::timer_boxed`, `ObjMut<Image>::set_src_owned`, `set_src_buffer`, `ImageBuffer`, `OwnedImageSource`, `Display::create_boxed`, `Indev::create_boxed`, `Indev::create_button_boxed`. | 4 B of `.bss` for the deferred-free list; per block, a header of 8 B (`on_boxed`) or 4 B (`timer_boxed`, owned images, owned queues) on 32-bit targets, plus LVGL's allocator header. | — |
| `allocator` | no | `LvglAllocator`, `SizedHeader`, `lvgl_allocator!`: LVGL allocates from a Rust `static`. | — | `#define LV_USE_STDLIB_MALLOC LV_STDLIB_CUSTOM`, checked by the build script (`compile_error!` otherwise). |
| `fmt` | no | `Label::set_text_fmt` (32-byte chunks), `impl Display for Error`, `impl core::error::Error for Error`. | Links `core::fmt`. Without it, `Label::set_text_i32` and `set_text_u32` format numbers without `core::fmt`. | — |
| `defmt` | no | `defmt::Format` for `Error`, `Handle`, `Owned`, the event codes and the other public enums and plain types. Adds the `defmt` 1.0 dependency. | Not measured on its own. The size gate's thumbv6m firmware with `defmt` and `debug-counters` has 80 B more crate code than without them (4578 B against 4498 B) and stays within the same 4608 B budget (see [Overhead measurements](overhead.md#static-memory-in-firmware)). | — |
| `debug-counters` | no | `DebugCounters::read` and `DebugCounters::reset`: four 16-bit counters (`drained`, `refused`, `skipped`, `leaked`). | 8 B of `.bss`, plus code: +28 B on thumbv6m in views-only mode (2274 B against 2246 B), +122 B on riscv32imc with 16 slots (5780 B against 5658 B) ([Overhead measurements](overhead.md#static-memory-in-firmware)). | — |
| `growable-registry` | no | Handle slots beyond the `LVGL_RUST_HANDLE_SLOTS` static ones: when every slot is in use, the registry allocates a chunk from the global allocator (the `alloc` crate) as large as the registry so far (N, 2N, 4N, ... slots), up to the 65,535 slots raw handles address. `handle()`, `own()`, `create_owned`, `delete_later` and the other calls that take a slot then fail with `Error::OutOfMemory` when the allocator returns null, and with `Error::RegistryFull` only at 65,535 slots. No effect in views-only mode (`LVGL_RUST_HANDLE_SLOTS=0`). | The registry static grows by a pointer per possible chunk (16 − log2 N) and an 8-byte sentinel slot: 52 B of `.bss` on 32-bit targets with the default 32 slots, 56 B with 16. Each chunk is 8 B per slot on 32-bit targets (16 B on 64-bit ones), plus the allocator's header; chunks are never freed, not even by `Lvgl::deinit`, so at most (65,535 − N) × 8 B (524,024 B with N = 32) stay allocated. The application must provide a `#[global_allocator]` that does not take its memory from LVGL's heap (`Lvgl::deinit` releases that heap, the chunks outlive it). Code: the size gate's firmware with 16 slots has 5146 B of crate code on `thumbv6m-none-eabi` (4518 B without the feature) and 6868 B on `riscv32imc-unknown-none-elf` (5674 B without), measured by `mise run size-gate` with a bump allocator in the firmware. | — |
| `shadcn-theme` | no | `lvgl-rust-sys/shadcn-theme`. | See [lvgl-rust-sys](#lvgl-rust-sys). | — |
| `color_depth_1`, `color_depth_8`, `color_depth_16`, `color_depth_24`, `color_depth_32` | no | The matching `lvgl-rust-sys` feature. | — | `LV_COLOR_FORMAT_DEFAULT` guarded by `#ifndef`. |

`lvgl-rust` always enables `lvgl-rust-sys/build-lvgl` (the safe layer
calls into the linked C library) and turns off the other default features
of `lvgl-rust-sys`. It does not forward `use-vendored-config`: set
`DEP_LV_CONFIG_PATH` to a configuration with the OBJ_ID settings
([Build configuration](build-configuration.md#lv_confh-requirements-of-lvgl-rust)).

`async` and `heap` are checked to compile for `thumbv6m-none-eabi` and
`riscv32imc-unknown-none-elf` by `mise run size-gate`; the size gate
firmware itself builds without them (default features, plus `defmt`,
`debug-counters` or `growable-registry` in the runs that measure them).

## lvgl-rust-sys

| Feature | Default | Enables | Cost | LVGL configuration |
| --- | :---: | --- | --- | --- |
| `build-lvgl` | yes | Compiles and links `liblvgl.a`: LVGL's sources, the `.c` files of the configuration directory, the shims and, when enabled, the shadcn theme and the `LVGL_FONTS_DIR` sources. Without it only the bindings are generated (`DEP_LVGL_RUST_SYS_LINKED=0`), for builds that link LVGL elsewhere. | LVGL itself. | — |
| `use-string-functions` | yes | `strchr`, `strcmp`, `strcoll`, `strcpy`, `strlen`, `strnlen`, `strnlen_s`, `strcat`, `strncat`, `strncmp`, `strncpy`, `strrchr` as `#[no_mangle] extern "C"` Rust functions, for targets without a C library. | Only what the linker keeps. | — |
| `use-vendored-config` | no | Falls back to the bundled `lvgl-rust-sys/conf/lv_conf.h` when `DEP_LV_CONFIG_PATH` is unset or empty. | — | The bundled configuration has OBJ_ID off: `lvgl-rust` with handles refuses it. |
| `shadcn-theme` | no | Compiles the `lvgl_theme_shadcn` submodule with the same headers and definitions as LVGL, and binds `lv_theme_shadcn.h` (`lv_theme_shadcn_init`, `lv_theme_shadcn_set_palette`, `lv_theme_shadcn_deinit`, ...). No second LVGL and no Rust runtime dependency. | Not measured. | — |
| `color_depth_1` | no | `LV_COLOR_FORMAT_DEFAULT=LV_COLOR_FORMAT_I1`, `LV_COLOR_DEPTH=1`. | — | `LV_COLOR_FORMAT_DEFAULT` and `LV_COLOR_DEPTH` not hard-coded (guard them with `#ifndef`). |
| `color_depth_8` | no | `LV_COLOR_FORMAT_L8`, `LV_COLOR_DEPTH=8`. | — | as above |
| `color_depth_16` | no | `LV_COLOR_FORMAT_RGB565`, `LV_COLOR_DEPTH=16`. | — | as above |
| `color_depth_24` | no | `LV_COLOR_FORMAT_RGB888`, `LV_COLOR_DEPTH=24`. | — | as above |
| `color_depth_32` | no | `LV_COLOR_FORMAT_XRGB8888`, `LV_COLOR_DEPTH=32`. | — | as above |

The `color_depth_*` definitions go to the C compiler, to bindgen and to
`DEP_LVGL_RUST_SYS_DEFINES`. Without any of them, `lv_conf.h` decides.
With several (for example under `--all-features`), the smallest depth wins
and the build script prints a warning.

## lvgl-rust-gen

| Feature | Default | Enables | Needs |
| --- | :---: | --- | --- |
| `cli` | no | The `lvgl-rust-gen` binary: `lvgl-rust-gen audit [--check] [--verbose]` (rewrites or checks `access.toml` and `events.toml`) and `lvgl-rust-gen bump [--check] [--bindings <bindings.rs>]` (rewrites or checks `universe.txt`, reports hierarchy and stale table entries). Maintainer tool; `lvgl-rust` does not use it. | clang (`CLANG`, default `clang`; `mise run audit` pins clang 22.1.8). |

The library part (used by `lvgl-rust`'s build script) has no feature.
