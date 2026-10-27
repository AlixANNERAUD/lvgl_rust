# Configure LVGL for lvgl-rust

This guide sets up the `lv_conf.h` and the build variables that lvgl-rust
reads. LVGL itself is configured as in C; lvgl-rust adds three OBJ_ID
settings and three optional build variables.

## What you need

- A crate that depends on `lvgl-rust` (see the
  [first application tutorial](../tutorials/first-application.md)).
- A directory of your own for `lv_conf.h`, for example `lvgl-configuration/` at
  the root of your crate.

## Steps

### 1. Start from a working configuration

Copy [`lvgl-rust/conf/lv_conf.h`](../../lvgl-rust/conf/lv_conf.h) into
your directory. It is LVGL 9.6's `lv_conf_template.h` with:

- its content enabled (`#if 1`);
- `LV_COLOR_FORMAT_DEFAULT` wrapped in `#ifndef`, so the `color_depth_*`
  features can override it;
- `LV_USE_ASSERT_NULL` and `LV_USE_ASSERT_MALLOC` set to 1;
- the OBJ_ID settings of the next step.

It uses LVGL's builtin heap (`LV_USE_STDLIB_MALLOC LV_STDLIB_BUILTIN`) with
`LV_MEM_SIZE` of 65536 bytes, and no operating system (`LV_USE_OS
LV_OS_NONE`). Change anything else as you would for a C project.

### 2. Enable the OBJ_ID hooks

Handles find objects through LVGL's OBJ_ID hooks, which lvgl-rust defines.
Your `lv_conf.h` must contain:

```c
#define LV_USE_OBJ_ID 1
#define LV_OBJ_ID_AUTO_ASSIGN 1
#define LV_USE_OBJ_ID_BUILTIN 0
```

lvgl-rust then defines `lv_obj_assign_id`, `lv_obj_free_id`,
`lv_obj_set_id`, `lv_obj_id_compare` and `lv_obj_stringify_id`, so neither
LVGL nor your application may define them. Each LVGL object grows by 4
bytes on 32-bit targets (the `id` field).

If one of the three values differs, the build of lvgl-rust stops with a
`compile_error!` that says:

```text
lvgl-rust tracks objects through LVGL's OBJ_ID hooks. Add to lv_conf.h:

    #define LV_USE_OBJ_ID 1
    #define LV_OBJ_ID_AUTO_ASSIGN 1
    #define LV_USE_OBJ_ID_BUILTIN 0

lvgl-rust then defines lv_obj_assign_id, lv_obj_free_id, lv_obj_set_id,
lv_obj_id_compare and lv_obj_stringify_id. To keep LVGL's own OBJ_ID
support instead, build with LVGL_RUST_HANDLE_SLOTS=0 (views only, no
handles).
```

The build reads these values from the generated bindings, so they are the
values LVGL was really compiled with, `LVGL_CFLAGS` included.

### 3. Point the build at the directory

Set `DEP_LV_CONFIG_PATH` to the directory that contains `lv_conf.h`. The
simplest place is `.cargo/config.toml` in your crate or workspace:

```toml
[env]
DEP_LV_CONFIG_PATH = { value = "lvgl-configuration", relative = true }
```

The variable is required: an empty value counts as unset, and the build of
`lvgl-rust-sys` fails without it. The build reruns when the variable or
any file of the directory changes.

### 4. Choose the color depth

Either set `LV_COLOR_FORMAT_DEFAULT` in `lv_conf.h`, or enable one feature
of lvgl-rust:

```toml
[dependencies]
lvgl-rust = { git = "https://github.com/AlixANNERAUD/lvgl_rust", features = ["color_depth_16"] }
```

| Feature | `LV_COLOR_FORMAT_DEFAULT` | `LV_COLOR_DEPTH` |
| --- | --- | --- |
| `color_depth_1` | `LV_COLOR_FORMAT_I1` | 1 |
| `color_depth_8` | `LV_COLOR_FORMAT_L8` | 8 |
| `color_depth_16` | `LV_COLOR_FORMAT_RGB565` | 16 |
| `color_depth_24` | `LV_COLOR_FORMAT_RGB888` | 24 |
| `color_depth_32` | `LV_COLOR_FORMAT_XRGB8888` | 32 |

The feature defines both values for the C library, bindgen and the
metadata exported to other build scripts. Keep `LV_COLOR_FORMAT_DEFAULT`
inside `#ifndef LV_COLOR_FORMAT_DEFAULT` and do not define
`LV_COLOR_DEPTH` yourself, or the feature and the file disagree. If
several features are enabled (with `--all-features`, for example), the
smallest depth wins and the build prints a warning.

### 5. Size the registry and the queues (optional)

lvgl-rust reads three variables at build time. Set them in the same
`[env]` table:

```toml
[env]
DEP_LV_CONFIG_PATH = { value = "lvgl-configuration", relative = true }
LVGL_RUST_HANDLE_SLOTS = "16"
LVGL_RUST_POST_SLOTS = "8"
LVGL_RUST_MAX_CALLBACK_DEPTH = "16"
```

| Variable | Default | Accepted values | Effect |
| --- | --- | --- | --- |
| `LVGL_RUST_HANDLE_SLOTS` | 32 | 0 to 32768, rounded up to a power of two | Registry slots: how many objects, timers and groups can have a handle at once. 0 selects views-only mode. |
| `LVGL_RUST_POST_SLOTS` | 4 | 0 to 255 | Entries of the `Lvgl::post` queue. `post` returns `Error::Busy` when it is full. |
| `LVGL_RUST_MAX_CALLBACK_DEPTH` | 16 | 1 to 254 | Nested callbacks (an event handler that triggers another event, ...). A callback past the limit is skipped. |

A value out of range stops the build with a `compile_error!` naming the
variable and its range. The build reruns when one of them changes.

Each registry slot costs 8 bytes of `.bss` on 32-bit targets, and each
`post` entry 8 bytes: [Reduce the footprint](reduce-the-footprint.md)
and the [overhead reference](../reference/overhead.md) give the totals.

### 6. Build without handles (optional)

With `LVGL_RUST_HANDLE_SLOTS=0`, lvgl-rust keeps only sessions, views and
`ObjMut`: no `Handle`, `Owned`, `WidgetExt`, `delete_later`, `clean_later`,
`load_screen_later`, timers, groups, components, `on_removable` or
`OwnedQueue`. `Lvgl::create_screen` then returns an `ObjMut` instead of an
`Owned<Obj>`.

This mode defines no OBJ_ID hook, so `lv_conf.h` must do one of:

- keep LVGL's own hooks: `LV_USE_OBJ_ID 1` with `LV_USE_OBJ_ID_BUILTIN 1`;
- turn OBJ_ID off: `LV_USE_OBJ_ID 0` (objects stay 4 bytes smaller);
- leave both at the lvgl-rust values and define the five functions in
  your application.

With `LV_USE_OBJ_ID 1` and `LV_USE_OBJ_ID_BUILTIN 0`, the build prints a
warning that reminds you of this. The vendored
[`lvgl-rust-sys/conf/lv_conf.h`](../../lvgl-rust-sys/conf/lv_conf.h)
turns OBJ_ID off (`LV_USE_OBJ_ID 0`) and works in this mode.

Build scripts of crates that depend on lvgl-rust read the slot count as
`DEP_LVGL_RUST_HANDLE_SLOTS` (0 in views-only mode).

### 7. Add definitions without editing the file (optional)

`LVGL_CFLAGS` adds comma-separated `NAME` or `NAME=VALUE` definitions to
the C compiler and to bindgen, for example to turn a widget off for one
build:

```sh
LVGL_CFLAGS="LV_USE_CHART=0,LV_USE_CALENDAR=0" cargo build
```

## Pitfalls

- **Disabled widgets have no wrappers.** lvgl-rust generates its wrappers
  from the bindings of your configuration: with `LV_USE_CHART 0`, there is
  no `widgets::Chart`. Code that uses a widget fails to compile until you
  enable it in `lv_conf.h`.
- **The `use-vendored-config` feature of `lvgl-rust-sys` does not help.**
  Its vendored configuration turns OBJ_ID off, so lvgl-rust refuses it
  unless you build in views-only mode.
- **The `allocator` feature needs `LV_USE_STDLIB_MALLOC LV_STDLIB_CUSTOM`.**
  The build stops with a `compile_error!` otherwise. See
  [Use the heap](use-the-heap.md).
- **lvgl-rust needs the C library.** It enables the `build-lvgl` feature
  of `lvgl-rust-sys`; a build where LVGL is not compiled stops with
  "lvgl-rust needs the C library".
- **One copy only.** lvgl-rust defines the OBJ_ID hook symbols, so its
  manifest declares `links = "lvgl_rust"`: two versions of the crate in one
  dependency graph fail to resolve.

## See also

- [Build configuration reference](../reference/build-configuration.md)
- [Features reference](../reference/features.md)
- [Reduce the footprint](reduce-the-footprint.md)
