# Reduce the footprint

This guide trims what lvgl-rust and LVGL take in flash and RAM. lvgl-rust
itself is small; most of a firmware is LVGL, so its configuration matters
most. All numbers come from the generated
[overhead reference](../reference/overhead.md) (size profile, no LTO, LVGL
9.6 with the vendored `lvgl-rust/conf/lv_conf.h`); rerun `mise run
overhead` to measure your own changes.

## What you need

- A firmware that builds for your target
  ([Build for embedded targets](build-for-embedded-targets.md)).
- A way to read sizes: `size` and `nm` from your toolchain
  (`arm-none-eabi-size`, `riscv64-unknown-elf-nm`, ...).

## Where the bytes are

The size-gate firmware (it initialises LVGL, creates, owns, cleans and
deletes objects, and runs the timer handler) measured:

| Target | Configuration | Crate code, B | Crate .bss, B | Firmware text, B | Firmware bss, B | Crate share of text |
| --- | --- | ---: | ---: | ---: | ---: | ---: |
| `thumbv6m-none-eabi` | 16 handle slots | 4498 | 188 | 233252 | 66264 | 1.9 % |
| `thumbv6m-none-eabi` | views only | 2246 | 44 | 230368 | 66120 | 1.0 % |
| `riscv32imc-unknown-none-elf` | 16 handle slots | 5658 | 188 | 256368 | 66264 | 2.2 % |
| `riscv32imc-unknown-none-elf` | views only | 2820 | 44 | 252948 | 66120 | 1.1 % |

The crate has no `.data`. Almost all of the firmware's `.bss` is LVGL's
builtin heap (`LV_MEM_SIZE`, 65536 bytes in that configuration), and
almost all of its text is LVGL.

## Steps

### 1. Shrink LVGL first

In `lv_conf.h`:

- **`LV_MEM_SIZE`.** With the builtin allocator, this static array is the
  largest item of `.bss`. Size it from `lv_mem_monitor` on your real
  interface, or switch to your own allocator (`LV_STDLIB_CUSTOM` with the
  `allocator` feature, see [Use the heap](use-the-heap.md)).
- **Widgets.** Set `LV_USE_<WIDGET> 0` for every widget you do not use.
  lvgl-rust generates wrappers only for the widgets present in the
  bindings, so a disabled widget costs nothing on either side.
- **Fonts.** Each `LV_FONT_MONTSERRAT_<size> 1` adds its glyph tables to
  flash. Keep the sizes you draw with, or build subset fonts in their own
  crate ([Use external fonts](use-external-fonts.md)).
- **Features.** Logging (`LV_USE_LOG`), asserts, the performance and
  memory monitors, file systems, image decoders, layouts and draw formats
  you do not need (`LV_DRAW_SW_SUPPORT_*`) all cost flash.
- **Color depth.** A smaller `LV_COLOR_FORMAT_DEFAULT` (or
  `color_depth_16` instead of `color_depth_32`) halves the draw buffers.

### 2. Size the draw buffers

`DrawBuf`s are `static`s in `.bss`. In partial mode, a buffer needs at
least one row; a tenth of the screen is a common trade-off. One buffer
instead of two halves the memory and serialises rendering and flushing.
See [Write a display driver](write-a-display-driver.md).

### 3. Size the handle registry

The registry is a static array: 8 bytes per slot plus a 24-byte header on
32-bit targets.

| `LVGL_RUST_HANDLE_SLOTS` | Registry `.bss` |
| --- | ---: |
| 32 (default) | 280 B |
| 16 | 152 B |
| 0 (views only) | 8 B |

A slot is taken by each object or timer that has a handle (`handle()`,
`own()`, `create_owned`, `delete_later` until the drain, `on_removable`,
`subscribe_owned`) and by each group. `handle()` returns
`Error::RegistryFull` when none is left. Set the smallest power of two
your interface needs, in `.cargo/config.toml`:

```toml
[env]
LVGL_RUST_HANDLE_SLOTS = "16"
```

### 4. Drop handles altogether (views-only mode)

`LVGL_RUST_HANDLE_SLOTS=0` keeps sessions, views and `ObjMut` and removes
handles, ownership, deferred deletion, timers and groups. On the measured
firmware it halves the crate's code (4498 B to 2246 B on `thumbv6m`, 5658
B to 2820 B on `riscv32imc`) and its registry takes 8 B. It also lets you
turn OBJ_ID off (`LV_USE_OBJ_ID 0`), saving 4 bytes in every LVGL object.
See step 6 of [Configure LVGL](configure-lvgl.md) for the configuration it
needs.

### 5. Size the `post` queue

`LVGL_RUST_POST_SLOTS` (default 4) costs 8 bytes per entry plus 4 bytes
(36 B by default). Set it to 0 if no callback posts work; `post` then
always returns `Error::Busy`.

### 6. Leave optional features off

`async`, `heap`, `allocator`, `fmt`, `defmt` and `debug-counters` cost
nothing when off. When on:

| Feature | Cost |
| --- | --- |
| `debug-counters` | 8 B of `.bss`; the measured firmware stays within the same code budgets (on `thumbv6m`, 2274 B instead of 2246 B in views-only mode) |
| `defmt` | not measured alone; with `debug-counters` too, the 16-slot `thumbv6m` firmware has 4578 B of crate code instead of 4498 B |
| `heap` | 4 B of `.bss`, plus heap blocks for what you put there |
| `fmt` | links `core::fmt` for `set_text_fmt` and `Display for Error`; `set_text_i32` and `set_text_u32` avoid it |

### 7. Build with the size profile and LTO

The measurements above use `opt-level = "s"` without LTO, so that the
crate's symbols stay separate. In your own firmware, enable LTO as well:
code nobody calls is dropped.

```toml
[profile.release]
opt-level = "s"
lto = true
codegen-units = 1
```

## Pitfalls

- **Per-object cost of handles.** Every LVGL object carries the 4-byte
  OBJ_ID field when handles are on, whether or not it has a handle.
- **Too few slots fail at run time, not at build time.** Check the
  `Result` of `handle()`, `own()` and `delete_later()`.
- **Views-only mode removes API.** Code using `Handle`, `Owned` or timers
  stops compiling; plan the mode before writing the interface.
- **Firmware numbers are one configuration.** Rerun `mise run overhead`
  (or measure your own image with `size`) after changing `lv_conf.h`.

## See also

- [Overhead explanation](../explanation/overhead.md)
- [Overhead reference](../reference/overhead.md)
- [Build configuration reference](../reference/build-configuration.md)
