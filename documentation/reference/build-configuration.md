# Build configuration

Every input the build scripts read, the metadata they export, the files
they generate and the cfgs they set. Cargo features are in
[Cargo features](features.md).

The build scripts of `lvgl-rust-sys` and `lvgl-rust` read their variables
with `cargo:rerun-if-env-changed`, so changing one reruns the build
script; for them, an empty value means unset. The development variables
at the end are read by scripts and tests, not by the build scripts.

## lvgl-rust-sys inputs

| Variable | Default | Effect |
| --- | --- | --- |
| `DEP_LV_CONFIG_PATH` | none (the repository's `.cargo/config.toml` sets `lvgl-rust-sys/conf` for commands run in the checkout) | Directory containing `lv_conf.h`. Required, except with the `use-vendored-config` feature or `DOCS_RS`, which fall back to `lvgl-rust-sys/conf`. The build fails if the directory does not exist or has no `lv_conf.h`. The directory is on the include path, and its `.c` files (searched recursively) are compiled into `liblvgl.a`. |
| `LVGL_CFLAGS` | none | Extra preprocessor definitions, comma-separated, each `NAME` or `NAME=VALUE` (blank entries ignored, spaces trimmed). Passed to the C compiler and to bindgen, and exported as `DEP_LVGL_RUST_SYS_CFLAGS`. |
| `LVGL_FONTS_DIR` | none | Legacy fonts: the directory's `.c` files are compiled into `liblvgl.a`, the directory is added to the include path, and its top-level `.h` files are passed to bindgen. Any change rebuilds LVGL and the bindings. A `fonts/` directory is never picked up implicitly. |
| `EMSDK` | none | Emscripten SDK root, required for `*-emscripten` targets: bindgen gets `-I` for `upstream/emscripten/system/include/libc`, `upstream/emscripten/system/lib/libc/musl/arch/emscripten` and `upstream/emscripten/system/include/SDL`. The build panics if it is unset. |
| `DOCS_RS` | none | Any non-empty value: the vendored `lvgl-rust-sys/conf` is used when `DEP_LV_CONFIG_PATH` is unset. |

Read by `cc` and bindgen rather than the build script (see their
documentation):

| Variable | Effect |
| --- | --- |
| `CC_<target>`, `AR_<target>`, `CFLAGS_<target>` (and `TARGET_CC`, `TARGET_AR`, `TARGET_CFLAGS`) | C compiler, archiver and flags for LVGL. |
| `BINDGEN_EXTRA_CLANG_ARGS_<target>` (and `BINDGEN_EXTRA_CLANG_ARGS`) | Extra clang arguments for bindgen, typically the C library headers given to the C compiler. |

The build script never passes `--target` to clang: for cross builds,
bindgen derives the clang triple from Cargo's `TARGET`.

### Rebuild triggers

Besides the variables above, the build script reruns when one of these
changes: `<config dir>/lv_conf.h`, `lvgl/src`, `lvgl/include`,
`lvgl/lvgl.h`, `shims/`, the `LVGL_FONTS_DIR` directory, and, with
`shadcn-theme`, `lvgl_theme_shadcn/src` and `lvgl_theme_shadcn/include`.

### Native configuration

The same include paths and definitions feed the C compiler (`liblvgl.a`,
with `build-lvgl`), bindgen and the exported metadata.

Include paths, in order:

1. `lvgl/src`
2. `lvgl/include`
3. `lvgl` (the LVGL root)
4. the configuration directory
5. `shims`
6. the `LVGL_FONTS_DIR` directory, if set
7. `shims/freestanding`, for targets starting with `wasm` and ending with
   `-unknown-unknown` (no C library; provides `inttypes.h`)
8. `lvgl_theme_shadcn/include`, with `shadcn-theme`

Definitions:

| Definition | When |
| --- | --- |
| `LV_CONF_INCLUDE_SIMPLE=1` | Always. |
| `LV_COLOR_FORMAT_DEFAULT=<format>`, `LV_COLOR_DEPTH=<depth>` | With a `color_depth_*` feature. |
| `LVGL_CFLAGS` entries | When set. |

Sources compiled into `liblvgl.a` (`build-lvgl` only), in this order of
directories: the `LVGL_FONTS_DIR` directory if set, `lvgl/src`, the
configuration directory, `shims`, and `lvgl_theme_shadcn/src` with
`shadcn-theme`. Every `.c` file of a directory is found recursively and
compiled in sorted order.

bindgen: headers `shims/lvgl_sys.h` (which includes `lvgl.h`), the
top-level headers of `LVGL_FONTS_DIR` and `lv_theme_shadcn.h` with
`shadcn-theme`; no comments, `Default` derived, no layout tests,
`core` types with `core::ffi` C types, `-fvisibility=default`.

## Exported metadata (`links = "lvgl_rust_sys"`)

Dependent build scripts read these variables. Their names and separators
are a public contract: changing one is a breaking change.

| Variable | Content | Separator |
| --- | --- | --- |
| `DEP_LVGL_RUST_SYS_INCLUDE_PATHS` | The include paths above, absolute. | `;` |
| `DEP_LVGL_RUST_SYS_DEFINES` | The crate's definitions: `LV_CONF_INCLUDE_SIMPLE=1` and the color depth definitions, as `NAME` or `NAME=VALUE`. | `;` |
| `DEP_LVGL_RUST_SYS_CFLAGS` | The `LVGL_CFLAGS` entries. | `,` |
| `DEP_LVGL_RUST_SYS_TARGET` | Cargo target triple of the build. | — |
| `DEP_LVGL_RUST_SYS_BINDINGS` | Absolute path to the generated `bindings.rs`. | — |
| `DEP_LVGL_RUST_SYS_LINKED` | `1` when `build-lvgl` compiled the C library, `0` otherwise. | — |

C code compiled against LVGL (fonts, drivers, custom widgets) must use
the include paths and both the `DEFINES` and `CFLAGS` lists, with the
target-aware compiler from `cc`.

`lvgl-rust` exports, through `links = "lvgl_rust"`:

| Variable | Content |
| --- | --- |
| `DEP_LVGL_RUST_HANDLE_SLOTS` | The number of handle slots after rounding (`0` in views-only mode); the static slots with `growable-registry`. |

`links = "lvgl_rust"` also means only one copy of `lvgl-rust` can be in a
build: it defines LVGL's OBJ_ID hook symbols.

## lvgl-rust inputs

| Variable | Default | Values | Effect |
| --- | --- | --- | --- |
| `LVGL_RUST_HANDLE_SLOTS` | 32 | 0 to 32,768 | Size of the handle registry, rounded up to a power of two: `8·N + 24` B of `.bss` on 32-bit targets. With feature `growable-registry`, the static slots and the size of the first chunk: the registry then grows to 2N, 4N, ... slots, up to 65,535, from the global allocator, and its static takes `8·N + 24 + 4·(16 − log2 N) + 8` B (cfg `lvgl_rust_growable`). `0` selects views-only mode (cfg `lvgl_rust_views_only`): no handles, `Owned`, timers, groups, components, deferred deletion or OBJ_ID hooks; 8 B of state. |
| `LVGL_RUST_POST_SLOTS` | 4 | 0 to 255 | Entries of the `Lvgl::post` queue: `8·P + 4` B of `.bss`, none for 0. |
| `LVGL_RUST_MAX_CALLBACK_DEPTH` | 16 | 1 to 254 | Nested callbacks (event handlers, timers, animations, observers) that run Rust code; deeper ones are skipped (`skipped` with `debug-counters`). |
| `LVGL_RUST_GEN_STRICT` | unset | any non-empty value | Stale table entries (functions or parameters missing from these bindings and not in `universe.txt`) fail the build instead of printing a warning. Every mise task sets it to `1`. |
| `DOCS_RS` | unset | any non-empty value | Documentation mode (cfg `lvgl_rust_docs`): the OBJ_ID, allocator and `build-lvgl` checks are skipped and no OBJ_ID hook is defined. |
| `DEP_LVGL_RUST_SYS_BINDINGS`, `DEP_LVGL_RUST_SYS_INCLUDE_PATHS`, `DEP_LVGL_RUST_SYS_LINKED` | set by `lvgl-rust-sys` | — | The bindings the generator reads, the header directories it scans (those ending in `include` or `shims`), and the linked check. |

An out-of-range value is a `compile_error!` naming the variable and its
range.

### `lv_conf.h` requirements of lvgl-rust

| Setting | Required when | Otherwise |
| --- | --- | --- |
| `LV_USE_OBJ_ID 1`, `LV_OBJ_ID_AUTO_ASSIGN 1`, `LV_USE_OBJ_ID_BUILTIN 0` | handles (`LVGL_RUST_HANDLE_SLOTS` above 0) | `compile_error!` with the lines to add. `lvgl-rust` defines `lv_obj_assign_id`, `lv_obj_free_id`, `lv_obj_set_id`, `lv_obj_id_compare` and `lv_obj_stringify_id`; the application must not. Cost: 4 B per object. |
| OBJ_ID off, or `LV_USE_OBJ_ID_BUILTIN 1`, or the application defines the five hooks | views-only mode | With `LV_USE_OBJ_ID 1` and `LV_USE_OBJ_ID_BUILTIN 0`, a build warning: nothing defines the hooks. |
| `LV_USE_STDLIB_MALLOC LV_STDLIB_CUSTOM` | feature `allocator` | `compile_error!`. |
| `build-lvgl` feature of `lvgl-rust-sys` (`DEP_LVGL_RUST_SYS_LINKED=1`) | always (outside `DOCS_RS`) | `compile_error!`. |

`lvgl-rust/conf/lv_conf.h` is a vendored configuration with these
settings; it differs from `lvgl-rust-sys/conf/lv_conf.h` only by
`LV_USE_OBJ_ID 1` and `LV_USE_OBJ_ID_BUILTIN 0`.

## lvgl-rust-gen inputs

| Variable | Read by | Effect |
| --- | --- | --- |
| `LVGL_RUST_GEN_STRICT` | `lvgl-rust`'s build script, passed as `Config::strict` | See above. |
| `UPDATE_GOLDEN` | `lvgl-rust-gen/tests/generate.rs` | Any value: rewrites `tests/golden/` instead of comparing the generated output with it. |
| `CLANG` | `lvgl-rust-gen audit` (feature `cli`) | The clang used to compile LVGL to LLVM IR. Default `clang`. |
| `CARGO` | `lvgl-rust-gen bump` (feature `cli`) | The cargo used to build the bindings of `lvgl-rust/conf` in `target/bump`. Default `cargo`. |

The generator refuses bindings whose LVGL version has no
`tables/lvgl-<major>.<minor>/` directory. Errors in the tables always
fail the build.

## Development variables

| Variable | Read by | Effect |
| --- | --- | --- |
| `SIZE_GATE_REPORT` | `lvgl-rust/size-gate/gate.sh` | A file to which each run appends one tab-separated line of sizes (read by `mise run overhead`). |
| `LVGL_RUST_OVERHEAD_OUTPUT` | `lvgl-rust/benches/overhead.rs` | A directory where the benchmark writes its tables. |
| `TRYBUILD=overwrite` | trybuild (`mise run test-compile-fail`) | Refreshes the expected `.stderr` of the compile-fail cases. |

## Generated files

| File | Written by | Content |
| --- | --- | --- |
| `$OUT_DIR/bindings.rs` | `lvgl-rust-sys` | bindgen output, included as `lvgl_rust_sys::*` (`lvgl_rust::sys`). Path exported as `DEP_LVGL_RUST_SYS_BINDINGS`. |
| `$OUT_DIR/liblvgl.a` | `lvgl-rust-sys` (`build-lvgl`) | The compiled C library, linked as `lvgl`. |
| `$OUT_DIR/gen.rs` | `lvgl-rust` (through `lvgl-rust-gen`) | The `enums`, `widgets`, `functions` and `props` modules, the generated methods of views, `ObjMut`, `Lvgl`, `StyleMut` and `Font`, `DrawBuf`, and the part tables of the structural checks. |
| `$OUT_DIR/coverage.txt` | `lvgl-rust` (through `lvgl-rust-gen`) | One line per bound function, in the format below. |
| `$OUT_DIR/config.rs` | `lvgl-rust` | `HANDLE_SLOTS`, `MAX_CALLBACK_DEPTH`, `POST_SLOTS`, and one `compile_error!` per configuration error. |

`coverage.txt` lines:

| Line | Meaning |
| --- | --- |
| `<function>: View <method>` | Shared method of a view. |
| `<function>: ObjMut { consumes: <bool> } <method>` | Exclusive method of `ObjMut` (`self` when it consumes). |
| `<function>: LvglShared <method>` / `LvglExclusive <method>` | Method of `&Lvgl` / `&mut Lvgl`. |
| `<function>: Create { exclusive: <bool> } create` | `W::create` and `W::create_owned`. |
| `<function>: Style <method>` | Method of `StyleMut`. |
| `<function>: Free <function>` | Free function in `functions` (pure or interrupt-safe). |
| `<function>: raw (<reason>)` | Not wrapped: stays in `sys`. |
| `<function>: manual (<reason>)` | Wrapped by hand in `lvgl-rust`, or crate-managed. |
| `<function>: skip (<reason>)` | No `access.toml` entry. |

## Cfgs

| Cfg | Set by | When |
| --- | --- | --- |
| `lv_use_<x>` | `lvgl-rust` | One per `LV_USE_*` constant of the bindings, set when its value is 1 (`lv_use_label`, `lv_use_chart`, `lv_use_grid`, ...). Disabled widgets are absent from the bindings, so nothing is generated for them. |
| `lvgl_rust_views_only` | `lvgl-rust` | `LVGL_RUST_HANDLE_SLOTS=0`. |
| `lvgl_rust_docs` | `lvgl-rust` | `DOCS_RS` set. |
| `loom` | `RUSTFLAGS="--cfg loom"` | Model checking (`mise run loom`). |
