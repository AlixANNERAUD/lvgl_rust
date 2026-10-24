# Safe, generated LVGL bindings (`lvgl-rust`): design

- Status: draft, under review
- Date: 2026-10-24
- LVGL version: 9.6.0 (submodule `80ca777e3`)
- Scope: a new safe crate and generator on top of `lvgl_rust_sys`, a
  workspace restructure, and the final repository rename

## 1. Goal

Provide a safe, idiomatic Rust API for LVGL 9 that:

1. is **generated at build time** from the per-build bindgen output, so it
   follows each user's `lv_conf.h` (disabled widgets and features are not
   generated), with a checked-in overrides table for what signatures cannot
   express;
2. keeps the **memory footprint** (static RAM, heap including LVGL's own
   `lv_malloc` heap, and flash) as low as possible: this is the top
   priority, and every runtime structure below is budgeted in bytes;
3. is **portable** across wasm32 (`unknown-unknown`, `wasip1`,
   `emscripten`), ESP32 (xtensa and `riscv32imc`), STM32/nRF Cortex-M
   (`thumbv6m` without atomic CAS, `thumbv7m`/`thumbv7em`, `thumbv8m`) and
   native hosts (x86_64/aarch64 on Linux, macOS, Windows);
4. is **sound in multi-threaded programs** without locking itself: the
   application proves serialized access with a token (the proposal of
   emobotics-dev/oxivgl#130);
5. fixes every soundness gap known from prior art (section 2.2).

Xila (the main consumer) is not modified from this repository; section 13
only lists the migration steps it can take.

### 1.1 Non-goals for v1

- Exclusive access inside input callbacks (synchronous delete, reparent or
  screen load from a click handler): deferred actions cover it in v1.
- `CustomClass<T>` (Rust state inside an LVGL object): later milestone,
  requires `LV_USE_PRIVATE_API=1`. Xila's `Window` stays out of scope.
- A DELETE-event hook backend for configs that cannot enable `LV_USE_OBJ_ID`.
- Re-initialising LVGL after `lv_deinit` in the same process.
- Safe handlers for refresh-time event codes (`COVER_CHECK`, `DRAW_*`,
  `DRAW_TASK_ADDED`): custom drawing stays in `sys` (9.2).
- Safe wrappers for the draw, image decoder and file-system driver
  registries, varargs functions, and anything the audit marks `Raw`: these
  stay reachable through the re-exported `sys` module.
- Bindings-only consumers (Xila's wasm guest): they keep using
  `lvgl_rust_sys` directly.

## 2. Context

### 2.1 Inputs

- `lvgl_rust_sys` already compiles LVGL with `cc`, generates `no_std`
  bindings with bindgen (1,961 `lv_*` functions with the vendored config)
  and exports `DEP_LVGL_RUST_SYS_{INCLUDE_PATHS,DEFINES,CFLAGS,TARGET}`.
  That contract is unchanged; only additive keys are introduced.
- API inventory (classification of every bound function):
  - about 66% of functions can be wrapped from signatures and naming rules
    alone, 72% when automatic skips are counted;
  - the remainder is covered by about 15 type- or pattern-level rules and
    about 150 to 250 per-function overrides;
  - pointer const-ness is not a reliable mutation signal (40 to 66% of
    getters take a `*mut` receiver); the verb is;
  - about 140 setters retain the pointer they receive, many undocumented.
- Measured struct sizes (clang, `conf/lv_conf.h`, 32-bit targets / aarch64):
  `lv_obj_t` 56 B (60 B with `LV_USE_OBJ_ID`) / 80 B (88 B),
  `lv_obj_spec_attr_t` 56 B / 72 B, `lv_event_dsc_t` 12 B / 24 B,
  `lv_style_t` 12 B / 16 B (+4 B / +8 B with `LV_USE_ASSERT_STYLE`),
  `lv_style_const_prop_t` 8 B / 16 B, `lv_timer_t` 28 B / 40 B. The
  vendored config has `LV_USE_EXT_DATA 0`; enabling it grows objects,
  event descriptors and timers. LVGL's TLSF allocator turns an n-byte
  request into a `block(n) = max(12, round4(n)) + 4` byte block on 32-bit
  targets and `max(24, round8(n)) + 8` on 64-bit targets
  (`lv_tlsf.c:235-242, 349-361`).

### 2.2 Soundness gaps to fix (prior art)

| Gap | Seen in | Fixed by |
|---|---|---|
| Handle used after its parent, `lv_obj_clean` or a widget deleted the object | lvgl-rs #111, oxivgl spec 8.1 | Registry + OBJ_ID hook (6, 8) |
| Several Rust handles for one object, double free on Drop | lvgl-rs #111, #140 | Views are borrowed, one `Owned` per object (7) |
| Style, font, text or image freed while LVGL still points to it | lvgl-rs #110 | `'static` retained data, owned setters freed after destructors (9.1) |
| Boxed closures in `user_data` never freed | lvgl-rs | Closures freed at their object's DELETE (9.2) |
| `Drop` running without the lock | oxivgl #130 open question | Store-only deferred requests (8.2) |
| Pointer-validity test used as a use-after-free check | oxivgl (`lv_obj_is_valid`) | Never used; generation + class check (6.3) |
| Calls before `lv_init` | lvgl-rs #69 | Every call needs a `Session`, only `init` creates the first one (5) |
| Unchecked enum values used as array indices | LVGL with `LV_USE_CHECK_ARG=0` | Newtype enums + bounds checks (11) |
| Callback deleting the object an outer call is still using | LVGL source analysis | Callbacks get shared access only (5.3, 9.2) |

## 3. Workspace and crates

```
lvgl-rust/                     workspace; repository renamed to xila-project/lvgl-rust last
├─ lvgl-rust-sys/              today's crate: lib name lvgl_rust_sys, links = "lvgl_rust_sys"
│   └─ lvgl/, lvgl_theme_shadcn/, build/, src/, shims/, conf/, tests/, examples/   (moved in M0, 3.3)
├─ lvgl-rust-gen/              host-only generator library, used by lvgl-rust's build.rs (4)
│   └─ tables/lvgl-9.6/{access,events,classes,relations}.toml, overrides/**/*.toml, universe.txt
└─ lvgl-rust/                  safe crate: no_std, no alloc, no runtime dependencies,
    │                          links = "lvgl_rust" (prevents two copies of the hook symbols)
    └─ conf/lv_conf.h          vendored config with OBJ_ID enabled, for tests and examples (3.2)
```

### 3.1 `lvgl-rust-sys` changes (additive, plus one cross-target fix)

- New metadata keys, documented in the README:
  - `DEP_LVGL_RUST_SYS_BINDINGS`: absolute path to the generated
    `bindings.rs`;
  - `DEP_LVGL_RUST_SYS_LINKED`: `1` when the `build-lvgl` feature compiled
    the C library, `0` otherwise.
- New C shims in `shims/lvgl_sys.{c,h}` (they need the private object
  layout, which bindgen only sees as opaque under the vendored config):
  - `bool lvgl_sys_obj_is_deleting(const lv_obj_t *)`;
  - `void lvgl_sys_obj_set_id_raw(lv_obj_t *, void *)`, under
    `#if LV_USE_OBJ_ID`;
  - `void lvgl_sys_image_cache_drop(const void *)`, unconditional: it only
    forwards to `lv_image_cache_drop`, which LVGL always compiles and
    exports but declares in a private header
    (`lvgl/src/misc/cache/instance/lv_image_cache.h:52`, reached through
    `lvgl_private.h`), so unlike the other shims it needs no private struct
    layout. The image cache can be resized at runtime, so no config macro
    decides whether it is needed;
  - accessors for composite parts with no usable public getter, used by
    the part tables (8.3): every menu part (`storage`, `main`,
    `main_header`, `main_header_back_btn`, `main_header_title`, `sidebar`,
    `sidebar_header`, `sidebar_header_back_btn`, `sidebar_header_title`,
    `lvgl/src/widgets/menu/lv_menu_private.h:41-54`, under
    `#if LV_USE_MENU`), because the public menu getters are deprecated
    and log on every call. The sidebar
    accessors return NULL unless `sidebar_generated` is set:
    `lv_menu_set_sidebar_page(NULL)` deletes the sidebar but clears only
    that bit (`lv_menu.c:362-365`), so the cached pointers dangle.
- Cross targets:
  - `clang_args()` stops pushing `--target={TARGET}` (`build/main.rs:166-168`).
    clang rejects Rust triples such as `riscv32imc-unknown-none-elf`, and
    bindgen 0.72.1 adds `--target=rust_to_clang_target(TARGET)` itself when
    none is given and the target is not the host (`bindgen/lib.rs:684-724,
    785-799`), so host builds still get no `--target`. The mapping changes
    more than RISC-V:
    - `riscv32*` and `riscv64*` architectures become `riscv32` and
      `riscv64` (`riscv32imc-esp-espidf` becomes `riscv32-esp-elf`);
    - an `espidf` OS becomes `elf`, so Xila's `xtensa-esp32-espidf` and
      `xtensa-esp32s3-espidf` reach clang as `xtensa-esp32-elf` and
      `xtensa-esp32s3-elf`;
    - `aarch64-apple-*` becomes `arm64-apple-*`, and an Apple `sim` suffix
      becomes `simulator`;
    - other triples are passed unchanged.

    M0 checks that the bindings generated for `xtensa-esp32s3-espidf` (with
    the esp-rs clang) and `aarch64-apple-darwin` are identical before and
    after the change, and records any difference in the PR. AGENTS.md's
    cross-compilation pitfall is updated in the same milestone: bindgen
    chooses the clang triple from `TARGET`, and only for non-host targets.
  - `shims/freestanding/inttypes.h` defines the `PRI*` macros LVGL uses
    (`lvgl/include/lvgl/lv_types.h:22`,
    `lvgl/include/lvgl/stdlib/lv_sprintf.h:12-23`). Its directory joins
    `NativeConfig.include_paths` only for targets without a C library
    (`wasm32-unknown-unknown`), so `cc`, bindgen and
    `DEP_LVGL_RUST_SYS_INCLUDE_PATHS` agree, and it gets a
    `rerun-if-changed`. `LV_INTTYPES_INCLUDE` is a quoted include, so the
    `-I` directory wins over clang's builtin header, whose `#include_next`
    finds no C library. The confs define `LV_INTTYPES_INCLUDE` without
    `#ifndef`, so it is not redefined with `-D`.
- The crate keeps its package name until the rename milestone, then becomes
  `lvgl-rust-sys` (crates.io treats `-` and `_` as the same name; the crate
  was never published, Xila uses a git dependency).

### 3.2 `lvgl-rust`

- Depends on `lvgl-rust-sys` with `default-features = false`, requires
  `build-lvgl`, and passes through `use-string-functions`, `color_depth_*`
  and `shadcn-theme`.
- `build.rs` reads the `-sys` metadata, runs the generator and writes
  `$OUT_DIR/gen.rs`. If `LINKED=0` it emits `compile_error!`: the safe layer
  must share an address space with a linked LVGL.
- `lvgl-rust/conf/lv_conf.h` is the `-sys` vendored file with three
  changes: `LV_USE_OBJ_ID 1`, `LV_OBJ_ID_AUTO_ASSIGN 1`,
  `LV_USE_OBJ_ID_BUILTIN 0`. AGENTS.md documents it as a second vendored
  config with its own allowed-deviation list, checked by `diff` against the
  template.
  - `lvgl-rust`'s tests, examples and the "vendored with OBJ_ID" matrix
    entry (14.6) build `-sys` against it through an explicit
    `DEP_LV_CONFIG_PATH` (a mise task and CI job; `tests/common::cargo()`
    for nested builds).
  - The workspace value in `.cargo/config.toml` stays the `-sys` config,
    which keeps the `-sys` tests linking: with `LV_USE_OBJ_ID_BUILTIN 0`
    their binaries would lack the hook symbols. One cargo invocation builds
    `-sys` once, so the two packages are tested in separate invocations.
  - `LVGL_CFLAGS` cannot enable OBJ_ID on top of the `-sys` config:
    `lv_conf.h` defines `LV_USE_OBJ_ID` without `#ifndef`, so the `-D` is
    overridden.
  - docs.rs sets `DOCS_RS`, which makes `-sys` use its own vendored config
    (OBJ_ID off). Under `DOCS_RS`, `build.rs` skips the OBJ_ID and `LINKED`
    checks and compiles the hook module out: the API is documented, nothing
    is linked.
- Module layout: `src/core/{session,registry,view,objmut,hook,drain,structural,event,trampoline,lvbox}.rs`
  and `src/{display,indev,group,timer,anim,style,queue,allocator,component,error}.rs`,
  plus the generated modules.

### 3.3 Workspace move (M0)

- `git mv` the `lvgl/` and `lvgl_theme_shadcn/` submodules into
  `lvgl-rust-sys/` (this updates `.gitmodules`), then move `build/`, `src/`,
  `shims/`, `conf/`, `tests/` (with `tests/fixtures/`),
  `examples/external-fonts/` and `README.md` there. `build/main.rs` is
  unchanged: its paths are relative to its manifest, and `tests/common`
  finds its fixtures through `CARGO_MANIFEST_DIR`.
- The root keeps the workspace `Cargo.toml`, one `Cargo.lock`, `AGENTS.md`
  and its symlinks (repository map and commands updated), `mise.toml`,
  `.github/` and `.cargo/config.toml`, which becomes
  `DEP_LV_CONFIG_PATH = { value = "lvgl-rust-sys/conf", relative = true }`.
- The mise tasks and CI update their `--manifest-path`, `cargo fetch` and
  `cargo fmt` paths for the example and the fixture.
- The build-time generator finds the headers through
  `DEP_LVGL_RUST_SYS_INCLUDE_PATHS`; the audit CLI finds the LVGL sources
  through `cargo metadata` (the `-sys` manifest directory plus `lvgl/`),
  never through a hard-coded repository path.
- Xila's git dependency resolves the package by name inside the workspace,
  so it is unaffected.
- Release (M9) means a git tag. A crates.io publish fits too: `lvgl-rust-sys`
  would ship an `include` list (`lvgl/src`, `lvgl/include`, LVGL's top-level
  headers and licence, the theme sources, `build/`, `shims/`, `conf/`),
  about 4.2 MB compressed for LVGL 9.6, under the 10 MB limit.

### 3.4 Toolchain

The minimum Rust version is **1.89**, the highest of the releases the
design rests on:

| Assumption | Provided by |
|---|---|
| Standard C ABI for `extern "C"` on `wasm32-unknown-unknown` (LVGL passes `lv_color_t` by value, e.g. `lv_obj_set_style_bg_color`; older compilers corrupt such arguments) | 1.89 |
| `&raw mut` / `&raw const` (the `UserData` encoding, 9.2) | 1.82 |
| An unwinding panic out of `extern "C"` aborts (trampolines, 9.2) | 1.81 |
| Inline `const { }` blocks (post-monomorphization asserts, `[EMPTY; N]` statics) | 1.79 |
| `c""` literals | 1.77 |
| 32-bit atomic load and store on `riscv32imc-unknown-none-elf` (`forced-atomics`, no CAS) | older than 1.89; checked by the MSRV job |

- It is above the MSRV of bindgen 0.72 and `cc`, which `-sys` already
  uses.
- Every workspace crate declares `rust-version = "1.89"`.
- The esp-rs xtensa toolchain tracks stable releases; its CI job pins one
  at 1.89 or later.
- An MSRV CI job (14.7) checks the host, `wasm32-unknown-unknown` and
  `riscv32imc-unknown-none-elf`.

## 4. Generator

A host-only, build-time crate with no C, Python or Doxygen dependency.
Crate dependencies: `syn` (full), `quote` and `prettyplease`, already in
bindgen 0.72's graph, plus `serde` (derive) and `toml` for the tables.
libclang, already required by `-sys`, is used only by the `cli` audit
feature (4.7).

1. **Locate.** Read `DEP_LVGL_RUST_SYS_BINDINGS`, `_INCLUDE_PATHS`,
   `_LINKED`, `CARGO_CFG_TARGET_POINTER_WIDTH`, `CARGO_CFG_TARGET_ENDIAN`,
   and the build variables `LVGL_RUST_HANDLE_SLOTS` (default 32, rounded up
   to a power of two, `0` allowed) and `LVGL_RUST_MAX_CALLBACK_DEPTH`
   (default 16). Values out of range are build errors, never clamped:
   slots above 32,768, and a depth outside 1..=254 (so `depth` and the
   `busy` counters fit in a `u8`, 9.2). All of them, and every table, get
   `rerun-if-env-changed` / `rerun-if-changed`. The tables are read from
   `tables/lvgl-<major>.<minor>/`, selected by the `LVGL_VERSION_MAJOR` and
   `LVGL_VERSION_MINOR` constants of the bindings; no matching directory
   is a hard error.
2. **Parse.** Parse `bindings.rs` with `syn`. Collect foreign functions,
   constants (`LVGL_VERSION_*`, `LV_USE_*`, `LV_USE_OBJ_ID*`,
   `LV_USE_ASSERT_STYLE`, `LV_DRAW_BUF_ALIGN`, `LV_USE_STDLIB_MALLOC`,
   `LV_USE_PRIVATE_API`, ...), structs (opaque or plain data) and the
   `lv_*_class` statics. Function items are parsed with balanced brackets,
   never regexes.
3. **Header scan.** A plain-text scan of the include paths recovers what
   bindgen drops: `@nullable` and "or NULL", `LV_DEPRECATED(` on the line
   before a declaration, `[]` array parameters, and `static inline`
   functions. Defaults are conservative: an undocumented parameter is
   non-nullable, every pointer return is `Option`. The `-sys` bindings are
   not changed (no `generate_comments`), so every consumer sees identical
   bindings.
4. **Typed IR**, serialisable for golden tests. Receivers are matched on
   the C type name of parameter 0, longest class prefix first
   (`imagebutton` before `image`, `buttonmatrix` before `button`), never on
   the resolved Rust alias (every enum is a `c_uint` alias, `c_int` on
   MSVC targets). The generator accepts either as an enum's resolved type
   and rejects anything else; the golden IR records the C type name and a
   normalised `enum` kind, so the same golden files pass on Windows.
5. **Classify** with the inventory's ordered rules (first match wins) into
   a plan:

   ```rust
   pub enum Access { Pure, Isr, Shared, Exclusive, Raw }
   pub enum Plan {
       Skip(Reason), Raw(Hazard), Manual,
       Method { recv: ClassId, access: Access, params: Vec<ParamPlan>, ret: RetPlan },
   }
   pub enum RetPlan { Value, Enum(NewtypeId), View, CopyOut, Static, SubHandle(ChildOf) }
   ```

   Key rules, in order:
   - **Crate-owned functions** (`crate_owned` in `classes.toml`) are Skip
     or Raw, whatever any other table says. Skip: `lv_obj_assign_id`,
     `lv_obj_free_id`, `lv_obj_set_id`, `lv_obj_id_compare`,
     `lv_obj_stringify_id` (the crate defines them, 8.1). Raw: every
     `lv_obj_remove_event*` function (they would remove descriptors the
     crate installed); the `user_data`, `driver_data`, callback and
     `auto_delete` setters of crate-wrapped resources
     (`lv_timer_set_user_data`, `_set_cb`, `_set_auto_delete`,
     `lv_display_set_driver_data`, `_set_flush_cb`,
     `_set_flush_wait_cb`, `lv_indev_set_driver_data`, `_set_read_cb`, the
     `lv_anim_set_*` callback and `user_data` setters); every
     `*_set_external_data`; `lv_tick_set_cb` (it would replace the crate's
     tick source, 5.1) and `lv_tick_inc` (it writes LVGL's non-atomic
     `sys_time`). `lv_obj_set_user_data` stays an ordinary Shared setter:
     the crate stores nothing in object `user_data`.
     `lv_timer_set_repeat_count` is not on the list: timers are created
     with `auto_delete = false` (9.3), so an exhausted count pauses the
     timer, and the hand-written `Handle<Timer>` setter is safe.
     `crate_owned` blocks generated wrappers and table entries only;
     hand-written Manual code (`timer.rs`, `display.rs`, ...) calls these
     functions while keeping the invariant they protect.
   - **Crate-managed resources.** Functions whose receiver is `lv_timer_t`,
     `lv_display_t`, `lv_indev_t`, `lv_anim_t` or `lv_group_t`, and every
     `lv_event_dsc_*` function, are Manual (hand-written in `timer.rs`,
     `display.rs`, `indev.rs`, `anim.rs`, `group.rs`) unless the reviewed
     `generate` allowlist names a plain-value getter or setter. Their
     deletes exist only on the owner types. Getters returning LVGL-owned
     timers or groups (`lv_display_get_refr_timer`,
     `lv_indev_get_read_timer`, `lv_timer_get_next`, `lv_group_get_default`,
     `lv_group_by_index`) are Raw.
   - Skip libc shims, internals, deprecated functions and varargs functions.
   - `lv_<w>_create` is Shared and returns a view (6.2).
   - **Object returns are untyped.** A getter returning `lv_obj_t *`
     returns `&Obj`, whatever its name suggests (`lv_keyboard_get_textarea`
     returns `&Obj`, not `&Textarea`). A typed view is emitted only when an
     override certifies the class (`returns = { view = "label" }` for a part
     the widget creates itself, such as `lv_textarea_get_label`); otherwise
     the caller uses `Obj::downcast`. With `LV_USE_CHECK_ARG 0` LVGL's own
     class checks are compiled out, so a wrong typed view would be type
     confusion in C.
   - Mutability comes from the verb (`get_`, `is_`, `has_`, `calc_`,
     `check_` are shared reads); scalar and plain-data parameters map
     directly; `@nullable` becomes `Option`.
   - **Retained by default.** Any pointer parameter of a non-getter, `const`
     or not and `[]` included, is retained and Raw until the overrides
     table classifies it. The only exemptions are receiver and object
     pointers mapped to views, out-parameters, and parameters marked copied
     (area and point structs, strings LVGL duplicates). `_static` string
     variants take `&'static CStr`.
6. **Overrides.** TOML files under `overrides/**/*.toml`, with two key
   namespaces: `[type.<lv_type>]` entries apply first, then `[fn.<lv_fn>]`
   entries. Unknown keys are rejected (`deny_unknown_fields`); a
   `[type.*]` key must name a type in the bindings and a `[fn.*]` key a
   function in the bindings or in `universe.txt` (the union of the all-on
   config and the OS and draw-backend variants); every `params.<name>`
   must exist; conflicting `access` values are errors.
   `crate_owned` wins over `access.toml` and every override: an entry that
   makes one of its functions Shared or Exclusive is an error. Stale entries
   are hard errors in maintainer CI and warnings plus `$OUT_DIR/coverage.txt`
   downstream, so a user's `lv_conf.h` can never break their build.
   Retained parameters are listed per function by kind:
   - read-only: `&'static T` in core, or an owned setter with `heap` (9.1);
   - written by LVGL: an owned `&'static mut [T]` or a buffer type wrapping
     `UnsafeCell` (the chart ext x/y arrays, `lv_canvas_set_buffer`, both
     buffers of `lv_subject_init_string`);
   - length-bound: the chart ext arrays. The wrapper stores each array's
     length, and `lv_chart_set_point_count` and the ext setters reject a
     count larger than an assigned array; otherwise the group stays Raw;
   - `lv_obj_null_on_delete` is Raw.

   Other override families: the style mutators are mutable-style-only
   (9.1), the draw-buffer setters are Manual or Raw (9.5), and child
   reordering (`lv_obj_move_to_index`, `lv_obj_swap`) is Manual to run the
   structural check (8.3).

   **Rodata receivers.** `ConstStyle`, `Font` and `ImageDsc` are reached
   through `&'static` references to data that may sit in flash. Emitting a
   method that is not a `get_`/`is_` read on one of them is a hard
   generator error unless a `[type.*]` or `[fn.*]` entry allows it.
   `lv_font_set_kerning` writes into the font (`lv_font.c:203-207`), so it
   is Raw for `&'static Font`; kerning is chosen in the font's const
   initialiser.
7. **Access table.** `access.toml` gives every emitted function an
   `access` value, `pure`, `isr`, `shared`, `exclusive` or `raw` (there are
   no separate pure or ISR lists). It is produced offline by
   `lvgl-rust-gen audit` (feature `cli`, maintainers only, uses libclang)
   and reviewed on each LVGL bump:
   - it builds a call graph of `lvgl/src` under the union config;
   - edges through function pointers go to every LVGL implementation of
     that slot (class `event_cb`/`destructor_cb`, internal event
     descriptors, animation, timer and async callbacks); event sends with
     a constant code only connect to handlers registered for that code or
     for `LV_EVENT_ALL`;
   - registering an LVGL-internal callback adds an edge from the
     registering function to that callback, so functions that schedule a
     delete (`lv_async_call` or `lv_timer_create` with an internal deleting
     callback, `lv_anim_start` with `lv_obj_delete_anim_completed_cb`,
     `lv_obj_delete_async`, `lv_obj_delete_delayed`,
     `lv_msgbox_close_async`) are never Shared. These three are Raw: the
     deletion they schedule runs later from `lv_timer_handler` without the
     structural checks of 8.3 (a layer, a composite part or a loading
     screen would be deleted), and `delete_later` covers the need;
   - Rust callbacks are cut (they are shared-only); raw C callbacks are an
     unsafe obligation of the code that installs them;
   - sinks: `obj_delete_core`, `lv_display_delete`, and every `lv_free`,
     `lv_realloc`, `lv_ll_remove` or `lv_ll_clear` of memory reachable from
     an object, group, display or draw task, whether or not safe code can
     see it. For example `lv_obj_remove_child` shrinks `children[]`
     (`lv_obj.c:1225-1248`), and `lv_group_remove_obj` frees a list node
     that `focus_next_core` keeps across the DEFOCUSED send
     (`lv_group.c:535-543`). A sink is cleared only by showing that no
     frame dispatching a Rust callback holds a pointer into that memory
     across the dispatch;
   - for each event code, it scans every `lv_event_get_param` dereference
     and the getters that return the param unchecked
     (`lv_event_get_self_size_info`, `lv_event_get_hit_test_info`,
     `lv_event_get_cover_area`, `lv_event_set_ext_draw_size`) along all
     class `event_cb` paths, and derives the `sendable` list of
     `events.toml` (codes whose handlers tolerate a NULL param);
   - every `Exclusive` entry is stored with its witness path, and CI runs
     `audit --check` against the pinned LVGL. The check also fails if a
     `crate_owned` function reaches Shared or Exclusive, or if a
     mutable-style-only function accepts a `ConstStyle`.

   **A function without an `access.toml` entry is not emitted.** It stays
   reachable through `sys`.
8. **Emit** a single `gen.rs` with inline modules, one per widget, each
   behind `cfg` on the matching `LV_USE_*` (and
   `cargo::rustc-check-cfg` for every such cfg). Pure pass-throughs are
   `#[inline]` (unused ones cost 0 B of flash); wrappers containing checks
   are out of line and non-generic; generic APIs (`Handle<W>::get`,
   `WidgetExt::own`, `Handle::downcast`) are thin shells over non-generic
   `resolve`/`mint`. Exclusive wrappers follow one rule:
   - (a) a call whose first parameter is the object becomes a method on
     `ObjMut<'_, W>`, taking `self` by value when the table says `consumes`
     (`lv_msgbox_close` becomes `ObjMut<'_, Msgbox>::close(self)`);
   - (b) an exclusive call with no object receiver becomes a `&mut self`
     method on `Lvgl`;
   - (c) a `borrowed_internal` getter becomes `Handle<W>::x(self, &mut Lvgl)`
     and `ObjMut<'_, W>::x(&mut self)`, both borrowing exclusive access for
     the result's lifetime, plus a copy-out `x_into(&self, buf)` on the view.

   `const` size asserts are emitted so they are evaluated for the target.
   Items that need a literal from the bindings are emitted into `gen.rs`
   too: `DrawBuf` (9.5) gets `#[repr(C, align(N))]` with
   `LV_DRAW_BUF_ALIGN` filled in, because `repr(align)` accepts only a
   literal. Rustdoc comes from the LVGL header comments.
9. **Bump workflow.** `lvgl-rust-gen bump` regenerates `universe.txt`, the
   class hierarchy (from `.base_class = &lv_bar_class` in the sources) and
   the audit diff, and lists stale overrides and parameter-name drift.

Table excerpts (illustrative; the audit produces the real content; each
`# file` comment starts a separate file):

```toml
# access.toml: access = "pure" | "isr" | "shared" | "exclusive" | "raw"
[fn.lv_obj_set_parent]
access = "exclusive"
why = "lv_obj_remove_child shrinks children[] (lv_obj.c:1225-1248)"

[fn.lv_dropdown_open]
access = "exclusive"
why = "reparents the list (lv_dropdown.c:602)"

[fn.lv_screen_load_anim]
access = "exclusive"
why = "del_prev (lv_display.c:913-915), auto_del (940-942)"

[fn.lv_msgbox_close]
access = "exclusive"
consumes = true

[fn.lv_group_remove_obj]
access = "exclusive"
why = "frees a list node held across DEFOCUSED (lv_group.c:535-543)"

[fn.lv_obj_delete_async]
access = "raw"
why = "deletes later from lv_timer_handler without the structural checks (lv_obj_tree.c:142-147)"

[fn.lv_obj_add_screen_load_event]
access = "raw"
why = "user trigger code becomes a deleting code"

[fn.lv_color_mix]
access = "pure"

# events.toml: sendable = handlers tolerate a NULL param (derived by the audit);
# deleting = exclusive send; refresh = never delivered to safe handlers (9.2)
sendable = ["CLICKED", "SHORT_CLICKED", "VALUE_CHANGED", "READY", "CANCEL"]
deleting = ["CLICKED"]          # msgbox close button: lv_msgbox.c:259, 311-322
typed = { KEY = "send_key" }    # *param read unconditionally: lv_textarea.c:904, lv_spinbox.c:537, lv_table.c:615
refresh = ["COVER_CHECK", "DRAW_*", "DRAW_TASK_ADDED"]

# classes.toml
hierarchy = { slider = "bar", spinbox = "textarea", keyboard = "buttonmatrix", spinner = "arc", canvas = "image" }
generate = ["lv_display_get_horizontal_resolution", "lv_timer_get_paused"]

[crate_owned]
skip = ["lv_obj_set_id", "lv_obj_assign_id", "lv_obj_free_id", "lv_obj_id_compare", "lv_obj_stringify_id"]
raw = [
    "lv_obj_remove_event*", "lv_timer_set_user_data", "*_set_external_data",
    "lv_tick_set_cb", "lv_tick_inc",
]

[composite.msgbox]
parts = ["lv_msgbox_get_header", "lv_msgbox_get_title", "lv_msgbox_get_footer", "lv_msgbox_get_content"]
max_depth = 2

[composite.textarea]            # spinbox inherits it
parts = ["lv_textarea_get_label"]
max_depth = 1

[composite.roller]
parts = ["child:0"]
max_depth = 1

[composite.menu]                # deprecated in 9.6: protects objects reached through views
parts = [
    "shim:menu.storage", "shim:menu.main", "shim:menu.main_header",
    "shim:menu.main_header_back_btn", "shim:menu.main_header_title",
    "shim:menu.sidebar", "shim:menu.sidebar_header",          # NULL unless sidebar_generated
    "shim:menu.sidebar_header_back_btn", "shim:menu.sidebar_header_title",
]
max_depth = 3

[composite.tabview]
parts = ["lv_tabview_get_tab_bar", "lv_tabview_get_content"]
max_depth = 3                   # bar button labels (8.3)
indexed = true

# relations.toml
[fn.lv_keyboard_set_textarea]
param = "ta"
clear = "lv_keyboard_set_textarea(holder, NULL)"

[fn.lv_indev_set_cursor]        # reparents and can never be cleared
raw = true

# overrides/style.toml
[type.lv_style_t]
rodata = "ConstStyle"           # only get_/is_ methods on the const form

[fn.lv_style_reset]
style = "mutable"
why = "writes unconditionally (lv_style.c:199-207)"

# overrides/widgets/buttonmatrix.toml
[fn.lv_buttonmatrix_set_ctrl_map]
params.ctrl_map = { kind = "slice", len = "expr:self.button_count()" }

# overrides/widgets/chart.toml
[fn.lv_chart_set_series_ext_y_array]
params.array = { kind = "retained_mut", len = "point_count" }

# overrides/widgets/label.toml
[fn.lv_label_get_text]          # text_into(buf), Handle::text, ObjMut::text
returns = "borrowed_internal"

# overrides/widgets/textarea.toml
[fn.lv_textarea_get_label]
returns = { view = "label" }    # the textarea creates its label itself
```

## 5. Access model

```rust
/// Implement only on a type whose existence proves exclusive, non-recursive
/// access to LVGL (for Xila: a dedicated guard newtype, never a guard type
/// shared with unrelated mutexes). The end of one guard's lifetime must
/// synchronize-with the start of the next one, on any thread (Release/Acquire
/// or stronger, as `Mutex` and RTOS mutexes provide): this orders the
/// registry and LVGL's own C globals between successive holders.
pub unsafe trait LvglAccess {}

pub struct Lvgl { _p: PhantomData<*mut ()> }                        // zero-sized, !Send, !Sync
pub struct Session<'g> { lv: Lvgl, _g: PhantomData<&'g mut ()> }  // zero-sized, Deref/DerefMut<Target = Lvgl>

pub fn init<'g, A: LvglAccess>(guard: &'g mut A) -> Result<Session<'g>, Error>; // lv_init once
impl Lvgl {
    pub fn enter<'g, A: LvglAccess>(guard: &'g mut A) -> Result<Session<'g>, Error>;
    pub fn timer_handler(&mut self) -> u32;                 // lv_timer_handler, then drain()
    pub fn deinit(session: Session<'_>) -> Result<(), Error>; // terminal (8.5)
    pub fn create_screen(&self) -> Result<Owned<Obj>, Error>; // no parent owns a screen; &Obj with N = 0
    pub fn screen_active(&self) -> Option<&Obj>;
    pub fn edit<W: Widget>(&mut self, select: impl for<'a> FnOnce(&'a Lvgl) -> Option<&'a W>)
        -> Option<ObjMut<'_, W>>;                           // exclusive access to a selected view (6.2)
    pub fn load_screen(&mut self, screen: Handle<Obj>, anim: ScreenAnim) -> Result<(), Error>;
    pub fn load_screen_later(&self, screen: Handle<Obj>, anim: ScreenAnim) -> Result<(), Error>; // 8.2
    pub fn set_tick_cb(&mut self, f: extern "C" fn() -> u32); // platform clock instead of tick_inc
    pub unsafe fn assume<'a>() -> &'a mut Lvgl;             // raw interop
    pub unsafe fn enter_callback() -> Option<DepthGuard>;   // raw C callbacks calling safe code
}
/// Zero-sized, !Send. Holds one level of `depth`; Drop decrements it.
pub struct DepthGuard { _p: PhantomData<*mut ()> }
impl DepthGuard { pub fn lv(&self) -> &Lvgl; }             // shared access, as in a trampoline
pub fn tick_inc(ms: u32);                                   // no token (5.1)
```

### 5.1 Rules

- The crate never locks. `enter` is generic over `A` but `#[inline]`
  around a non-generic `enter_raw`, so nothing is duplicated per guard type.
- `enter` fails with `Reentrant` if a session is already active
  (`entered` flag) or a callback is running (`depth > 0`). This catches
  recursive locks and minting inside callbacks. `entered` is a same-thread
  re-entry check, not the source of cross-thread ordering: that comes from
  the `LvglAccess` contract, and the SeqCst flag does not make a weaker
  guard sound.
- `init` fails with `Reentrant` when called twice (`init == 1`), and
  `init` or `enter` fail with `NotInitialized` after `deinit`
  (`init == 2`); no other safe entry point creates a first session, so no
  LVGL call can happen before `lv_init`. `enter` before `init` returns
  `NotInitialized`.
- `enter_callback` is for raw C callbacks that call safe code: it checks
  `depth < LVGL_RUST_MAX_CALLBACK_DEPTH`, increments it and returns a
  `DepthGuard`, or returns `None` at the limit. `guard.lv()` is the only
  way such code gets a `&Lvgl`; it never combines the guard with
  `Lvgl::assume`.
- Pure functions (colour, palette, area/point, trigonometry; about 60) need
  no token. ISR-safe functions (`tick_inc`, `FlushDone::ready`) need none
  either.
- `tick_inc` never calls `lv_tick_inc`, which writes LVGL's non-atomic
  `sys_time` (`lv_tick.c:37-43`): from a second core, a host thread or a
  wasm thread that is a C data race, including with the `lv_memzero` of
  `lv_init` (`lv_init.c:114`). Instead `init` calls
  `lv_tick_set_cb(crate_tick_cb)` after `lv_init` returns (the memzero
  would clear an earlier one), and `crate_tick_cb` returns
  `REG.hdr.ticks.load(Relaxed)`. `tick_inc` does `fetch_add` where
  `cfg(target_has_atomic = "32")` provides it, and a Relaxed load plus
  store otherwise (`thumbv6m`, `riscv32imc`): concurrent writers can then
  lose ticks but never cause undefined behaviour, so it should be called
  from one context. `Lvgl::set_tick_cb` replaces the source on platforms
  with a clock.

### 5.2 Shared and exclusive calls

- `&Lvgl` allows every `Shared` call: views, getters, value setters,
  registering handlers, sending non-deleting events.
- `&mut Lvgl` is required by every `Exclusive` call: anything that may free
  an object, start deleting one, shrink a children array or free memory
  reachable from LVGL objects (the sinks of 4.7), including everything LVGL
  reaches internally from that call (invariant C, section 6.4).
- A view borrows the `Lvgl` it came from, so a view and `&mut Lvgl` are
  never held at the same time. Exclusive APIs take a `Handle`, an `ObjMut`
  or a selector (`Lvgl::edit`), never a view next to `&mut Lvgl`.

### 5.3 Callbacks

LVGL calls the crate's trampolines only from inside calls made by the lock
holder. A trampoline conjures a shared `&Lvgl` (a zero-sized type: no
memory access), so callbacks get shared access only. A delete requested
from a callback is deferred (8.2). Refresh-time codes are never delivered
to safe handlers (9.2).

## 6. Registry, handles and views

### 6.1 Layout

One zeroed static `REG` in `.bss`; slots at or above the high-water mark
are implicitly free, so there is no `.data` initializer.

```rust
#[repr(C)]
struct Slot {                 // 8 B on 32-bit, 16 B on 64-bit (asserted: 2 * usize)
    ptr:  Cell<*mut c_void>,  // object / timer / group pointer, or the next free index
    gen:  Cell<u16>,
    meta: Cell<u8>,           // state FREE|LIVE|DEAD, OWNED, TEMP, KIND OBJ|TIMER|GROUP, P_DELETE, P_CLEAN
    req:  AtomicU8,           // written only by Owned::drop (store 1), cleared by the drain
}
#[repr(C)]
struct Header {               // 24 B (align 4) on every target; the slots start at offset 24
    ticks: AtomicU32,         // tick counter read by crate_tick_cb (5.1)
    hwm: Cell<u16>, head: Cell<u16>, tail: Cell<u16>,
    load: Cell<u16>,          // pending load_screen_later: slot index + 1, 0 = none (8.2)
    load_anim: Cell<ScreenAnim>, // 6 B: type, auto_del, time and delay in u16 ms (up to 65.5 s)
    depth: Cell<u8>, any_req: AtomicBool, entered: AtomicBool, init: AtomicU8,
}
#[repr(C)]
struct Registry { hdr: Header, slots: [Slot; N] } // static REG: 24 + 8·N B on 32-bit, 24 + 16·N on 64-bit (asserted)
#[repr(u8)] enum Kind { Obj, Timer, Group }        // the KIND bits of meta

#[repr(transparent)] pub struct Handle<W>(NonZeroU32, PhantomData<fn() -> W>); // gen << 16 | (index + 1)
#[repr(transparent)] pub struct Owned<W>(Handle<W>);                           // !Copy, Send
```

- `Handle<W>` is 4 B, `Copy + Send + Sync`; `Option<Handle<W>>` is 4 B.
- Each field has a single writer: `ptr`, `gen`, `meta` and the header's
  `Cell` fields are written only by the token holder; `req` once by the
  single `Owned::drop`, then cleared by the drain; `any_req` and `entered`
  with no read-modify-write: SeqCst stores, and SeqCst loads in the drain
  (8.2); `ticks` by `tick_inc` alone.
- `unsafe impl Sync for Registry` (the same argument covers `StaticQueue`,
  10): the `Cell` fields are read and written only while a `Session`
  exists, or inside trampolines called by its holder; sessions are totally
  ordered by happens-before through the `LvglAccess` contract (5); the only
  accesses without a token (`Owned::drop`, `tick_inc`, ISR and
  other-thread paths) touch only the atomics `req`, `any_req` and `ticks`,
  (SeqCst stores for the first two, 5.1 for `ticks`), and never
  read `gen` or `ptr`, so `Owned::drop` cannot check the generation first.
- No read-modify-write atomics are required anywhere (`tick_inc` uses
  `fetch_add` only where the target has it), so the registry works on
  `thumbv6m` and `riscv32imc` without `critical-section` or
  `portable-atomic`.
- Allocation takes fresh slots (`hwm`) first, then the FIFO free list, so
  reuse spreads over the currently free slots (at most N; just one when
  the registry is nearly full).
- `LVGL_RUST_HANDLE_SLOTS=0` selects the views-only mode (6.5): 8 B of
  static header and 0 B per object.

### 6.2 Views

```rust
#[repr(C)]
pub struct Obj { _o: UnsafeCell<[u8; 0]>, _m: PhantomData<(*mut u8, PhantomPinned)> } // at the lv_obj_t
#[repr(transparent)] pub struct Bar(Obj);    impl Deref for Bar    { type Target = Obj; }
#[repr(transparent)] pub struct Slider(Bar); impl Deref for Slider { type Target = Bar; }

pub unsafe trait Widget: 'static { type Base: Widget; fn class() -> *const lv_obj_class_t; }
pub unsafe trait IsA<B: Widget>: Widget {}

impl Slider {                                                 // generated for every widget
    pub fn create<'a>(parent: &'a Obj) -> Result<&'a Slider, Error>;
    pub fn create_owned(parent: &Obj) -> Result<Owned<Slider>, Error>; // create + own
}
pub trait WidgetExt: Widget + Sealed {                         // blanket impl for every W: Widget
    fn handle(&self) -> Result<Handle<Self>, Error>;          // O(1) dedup through obj->id
    fn own(&self) -> Result<Owned<Self>, Error>;              // AlreadyOwned / Internal for composite parts
}
impl<W: Widget> Handle<W> {
    pub fn get<'a>(self, lv: &'a Lvgl) -> Result<&'a W, Error>;
    pub fn get_mut<'s>(self, lv: &'s mut Lvgl) -> Result<ObjMut<'s, W>, Error>;
    pub fn upcast<B: Widget>(self) -> Handle<B> where W: IsA<B>;
    pub fn downcast<D: Widget + IsA<W>>(self, lv: &Lvgl) -> Option<Handle<D>>; // class check, same slot
}
impl Obj {
    pub fn as_ptr(&self) -> *mut lv_obj_t;                    // valid until the next exclusive call
    pub unsafe fn from_raw<'a>(lv: &'a Lvgl, p: NonNull<lv_obj_t>) -> &'a Obj;
    pub fn downcast<W: Widget>(&self) -> Option<&W>;          // lv_obj_has_class
    pub fn child(&self, index: i32) -> Option<&Obj>;
    pub fn parent(&self) -> Option<&Obj>;
}

pub struct ObjMut<'s, W> { p: NonNull<lv_obj_t>, _l: PhantomData<(&'s mut Lvgl, W)> }
impl<W: Widget> ObjMut<'_, W> {
    pub fn view(&self) -> &W;
    pub fn delete(self) -> Result<(), Error>;
    pub fn clean(&mut self) -> Result<(), Error>;
    pub fn set_parent(&mut self, parent: Handle<Obj>) -> Result<(), Error>;
    pub fn set_parent_to(&mut self, select: impl for<'a> FnOnce(&'a Lvgl) -> Option<&'a Obj>)
        -> Result<(), Error>;
    pub fn load_screen(self, anim: ScreenAnim) -> Result<(), Error>;
    pub fn send_event(self, code: SendableEventCode) -> Result<(), Error>;
    pub fn send_key(self, key: u32) -> Result<(), Error>;    // KEY with a live param (9.2)
}
```

- A view `&'a W` is a zero-sized type at the object's address: 0 B, and a
  call through it is the raw FFI call.
- Upcasts are free through `Deref`; the hierarchy comes from `classes.toml`.
- Creation is Shared: `lv_<w>_create` allocates and sends creation events
  (the audit confirms it per widget), and returns a view with the parent's
  lifetime, covered by invariant V. NULL maps to `OutOfMemory`. `.own()` or
  `.handle()` keeps the object; `create_screen` returns `Owned` because no
  parent owns a screen.
- `WidgetExt` replaces inherent `Obj::handle`/`own`, so `slider.handle()`
  returns `Handle<Slider>` instead of resolving to `Obj`'s method through
  auto-deref. Each method is a one-line shell over a non-generic
  `mint(ptr, Kind::Obj)`. `Handle::downcast` checks `D::class()` through
  `resolve` and reuses the slot.
- Object getters (`child`, `parent`, `screen_active`, event targets) return
  views, never handles, so traversing the tree never takes a slot. They
  return `&Obj` unless an override certifies the class (4.5); a typed view
  otherwise comes only from `downcast`, which checks `lv_obj_has_class`.
  `handle()` mints a slot only when the application wants to store one.
- `Lvgl::edit` turns a view chosen by a selector into an `ObjMut`: the
  selector's borrow ends before `&mut self` is re-borrowed and no LVGL call
  runs in between, so the pointer is still valid. `set_parent_to` passes
  the new parent the same way. Generated exclusive methods follow the
  emission rule of 4.8.
- Getters that point into object memory (176 functions) are never exposed
  as view-lifetime borrows, because a shared setter could free that memory:
  views get copy-out variants (`text_into(&self, buf: &mut [u8]) -> Result<&CStr, Error>`),
  and zero-copy borrows exist only under exclusive access
  (`Handle<Label>::text(self, lv: &mut Lvgl)` and
  `ObjMut<'_, Label>::text(&mut self)`, both `-> Result<&CStr, Error>`).
  No getter returns `&mut [T]`. Style getters return values tied to the
  view lifetime, never `&'static`.

### 6.3 Resolution

One non-inlined function of about 60 B:

```rust
fn resolve(raw: u32, kind: Kind, class: *const lv_obj_class_t) -> Result<NonNull<c_void>, Error> {
    let i = (raw as usize & 0xFFFF).wrapping_sub(1) & (N - 1);   // mask: no panic path
    let s = &REG.slots[i];
    if s.gen.get() != (raw >> 16) as u16 || s.state() != LIVE || s.kind() != kind
        || s.req.load(Relaxed) != 0 || s.meta.get() & P_DELETE != 0 { return Err(Error::Dead) }
    let p = s.ptr.get();
    if kind == Kind::Obj {
        if REG.hdr.depth.get() != 0 && unsafe { lvgl_sys_obj_is_deleting(p.cast()) } { return Err(Error::Dead) }
        if !class.is_null() && unsafe { lv_obj_get_class(p.cast()) != class && !lv_obj_has_class(p.cast(), class) } {
            return Err(Error::Dead)
        }
    }
    Ok(unsafe { NonNull::new_unchecked(p) })
}
```

A 16-bit generation wraps. A stale handle aliases only after its slot is
reused 65,536 times; with FIFO reuse that takes about 65,536 × K
allocations, where K is the number of free slots (only 65,536 when one
slot is free), plus an exact phase match. The class check guarantees that
an aliased view is a live object of a compatible class: a logic error,
never undefined behaviour. Values stored next to a handle do not rely on
the wrap bound: `EventSub` and `QueueRef` are validated against the
object's event list before use (9.2, 10), and a relation re-tests the
holder's getter (9.4). The registry never runs out because of retired
slots.

### 6.4 Invariants

- **T (single token).** A `Session` exists only when `entered` was false
  and `depth` was 0, and successive sessions are ordered by the
  `LvglAccess` contract. Trampolines run only inside calls made by the
  session holder and conjure only `&Lvgl`.
- **C (classification).** No `Shared` call frees or starts deleting an
  object, shrinks a children array, or frees or unlinks memory reachable
  from an object, group, display or draw task (the sinks of 4.7) while a
  frame dispatching a Rust callback may hold a pointer into it, including
  everything LVGL reaches from that call. Rust callbacks are shared-only,
  so they cannot complete such a chain, and they never run inside the
  rendering window (9.2). The audit (4.7) and the "absent means not
  emitted" rule maintain C.
- **V (views).** A view `&'a W` refers to an allocated object that is not
  being deleted, whose class is `W` or a subclass. Views come only from
  `resolve`, creation, `ObjMut::view`, or object getters (non-null, and
  not `is_deleting` when `depth > 0`). At depth 0 no deletion is in
  progress, because every deletion completes inside one exclusive call.
  During `'a` only shared calls run, so by C the object stays allocated.
- **R (registry).** A slot is LIVE if and only if its pointer refers to an
  allocated resource of its kind whose free hook has not run. Timers and
  groups are freed only by the crate (`auto_delete` forced to false;
  `deinit` is terminal). No safe API writes the fields R relies on
  (`obj->id`, timer `user_data` and `auto_delete`, display and indev
  `driver_data` and callbacks, the event descriptors the crate installs):
  the crate-owned rule (4.5) keeps their setters out of the generated API.

### 6.5 Views-only mode (N = 0)

`LVGL_RUST_HANDLE_SLOTS=0` keeps a reduced header (`ticks`, `init`,
`entered`, `depth`: 7 B, 8 B with alignment) and compiles out the slots, `hwm`,
`head`, `tail`, `any_req`, the pending load and the OBJ_ID requirement, so
objects cost 0 B more than in C.

The hook module (8.1) and the address-taking in `init` are compiled out
too: the crate defines none of the five OBJ_ID functions, so a config with
`LV_USE_OBJ_ID_BUILTIN 1`, or a user-defined OBJ_ID implementation, links
without duplicate symbols, and `obj->id` is left to it.

- Compiled out: `Handle`, `Owned`, `HandleCell`, `WidgetExt`,
  `create_owned`, `EventSub`/`on_removable`,
  `Event::target_handle`/`target_is`, timers, groups,
  `delete_later`/`clean_later`/`load_screen_later`, `subscribe_owned`,
  relation wrappers (raw-only), `Component`, and the forms that take a
  `Handle` (`Lvgl::load_screen`, `ObjMut::set_parent`, `Handle<W>::x`
  getters).
- Replacements: creation is unchanged, and
  `Lvgl::create_screen(&self) -> Result<&Obj, Error>` returns a view.
  Exclusive access comes from `Lvgl::edit`; `ObjMut::set_parent_to`,
  `ObjMut::load_screen`, `ObjMut::delete` and `ObjMut::clean` do the
  structural work; borrowed-internal getters exist on `ObjMut` and as
  `_into` copies.
- There is no deferred deletion without a slot (8.2): a callback records
  its intent in application state (an `on_with` context or a static), and
  the application makes the exclusive call after `timer_handler` returns.
- `heap` works without slots, except `subscribe_owned` and `timer_boxed`
  (both return an `Owned`, which needs a slot).

## 7. Ownership

`Owned<W>` is the single owner of an object (4 B, `!Copy`, `Send`). `own()`
refuses a second owner (`AlreadyOwned`) and composite parts (`Internal`).

```rust
impl<W: Widget> Owned<W> {
    pub fn handle(&self) -> Handle<W>;                                      // same slot
    pub fn get<'a>(&self, lv: &'a Lvgl) -> Result<&'a W, Error>;
    pub fn get_mut<'s>(&self, lv: &'s mut Lvgl) -> Result<ObjMut<'s, W>, Error>;
    pub fn delete(self, lv: &mut Lvgl) -> Result<(), Error>;               // 8.3 checks, then lv_obj_delete
    pub fn upcast<B: Widget>(self) -> Owned<B> where W: IsA<B>;
}                                                                           // Drop = deferred request (8.2)
```

- `owned.delete(&mut lv)` deletes immediately after the structural checks;
  on `Busy` (a screen being loaded) the request stays pending, as for a
  drop, and the drain retries.
- Dropping an `Owned` without a session records a deferred request (8.2).
- An owned object can still die through another path (its parent deleted,
  a consuming method such as `ObjMut<'_, Msgbox>::close`, an LVGL-internal
  deleter): the hook marks the slot DEAD (8.1), the `Owned`'s drop or
  `delete` only acknowledges it, and the drain frees the slot.
- `Handle`s never delete anything; when the object dies they resolve to
  `Err(Dead)`.
- Objects without an owner belong to their LVGL parent, as in C.

## 8. Deletion

### 8.1 OBJ_ID hook (required in v1)

- Requires `LV_USE_OBJ_ID 1`, `LV_OBJ_ID_AUTO_ASSIGN 1`,
  `LV_USE_OBJ_ID_BUILTIN 0` (N > 0 only). Otherwise `build.rs` emits
  `compile_error!` quoting the three lines to add and naming the five
  functions the crate then defines.
- With `LV_USE_OBJ_ID_BUILTIN 0`, LVGL defines none of `lv_obj_assign_id`,
  `lv_obj_set_id`, `lv_obj_free_id`, `lv_obj_stringify_id` and
  `lv_obj_id_compare` (`lv_obj_id_builtin.c:44-123`), while bindgen still
  declares them, so the crate defines all five.
- `lv_obj_free_id` is called at the end of `lv_obj_destructor`
  (`lv_obj.c:1561-1563`): after the event list (first removed in
  `obj_delete_core` at `lv_obj_tree.c:688`, before the children; the call
  at `lv_obj.c:1543` only removes descriptors added again during
  deletion), children (`lv_obj_tree.c:691-695`), styles and animations
  (1531) are removed, before `lv_free` (`lv_obj_tree.c:754`). `lv_obj_destruct` has a single caller,
  so every deletion path runs the hook and no event handler can skip it.
- Cost: +4 B per object (56 → 60 B, TLSF block 60 → 64 B), no allocation.

```rust
#[no_mangle] unsafe extern "C" fn lv_obj_assign_id(_: *const lv_obj_class_t, _: *mut lv_obj_t) {}
#[no_mangle] unsafe extern "C" fn lv_obj_free_id(o: *mut lv_obj_t) { registry::on_free(o, lv_obj_get_id(o) as usize) }
#[no_mangle] unsafe extern "C" fn lv_obj_set_id(o: *mut lv_obj_t, id: *mut c_void); // NULL: free_id, then clear obj->id
#[no_mangle] unsafe extern "C" fn lv_obj_id_compare(a: *const c_void, b: *const c_void) -> c_int { (a != b) as c_int }
#[no_mangle] unsafe extern "C" fn lv_obj_stringify_id(o: *mut lv_obj_t, buf: *mut c_char, len: u32) -> *const c_char; // "#index"
```

- `on_free`: if `id - 1 < N`, the slot's pointer equals the object and the
  slot is LIVE: an OWNED slot becomes DEAD (pending P_DELETE/P_CLEAN bits
  cleared) until the drain observes it; any other slot is freed (pending
  bits cleared, generation incremented, pushed on the FIFO). Either way the
  header's pending load is cleared if it names this slot. Foreign or
  garbage ids are ignored.
- `lv_obj_set_id(o, NULL)` mirrors the builtin, which frees the old id
  first (`lv_obj_id_builtin.c:82-87`): `on_free` runs and `obj->id` is
  cleared through the shim, so the object becomes untracked and its handles
  resolve to `Dead` (an owned object is then leaked, never freed twice). A
  non-NULL id is ignored, with a debug assertion: only the registry writes
  `obj->id`.
- `obj->id` holds `index + 1` (NULL = untracked) and is written through the
  `-sys` shim.
- With N > 0, `init` takes the addresses of the five hook functions so
  their archive member is always linked; CI checks with `nm`
  (codegen-units 16, no LTO). With N = 0 the module is compiled out (6.5).

### 8.2 Deferred requests

- `Owned::drop`: `req.store(1, SeqCst); any_req.store(true, SeqCst)`.
- `delete_later` and `clean_later` (usable from callbacks) set P_DELETE or
  P_CLEAN on the object's slot. An object without a slot gets one through
  the same path as `handle()`, marked TEMP, or the call returns
  `RegistryFull`. The structural checks (8.3) run when the request is made,
  and again when the drain executes it. At request time only permanent
  refusals are returned (`Internal` for a layer or a composite part,
  `InvalidArg`); a `Busy` result (a loading or previous screen) still sets
  P_DELETE or P_CLEAN and leaves the retry to the drain, as for
  `Owned::delete` (7).
- `load_screen_later` stores one pending load in the header (slot index and
  `ScreenAnim`); a second request before the drain replaces the first
  (last writer wins). It takes a `Handle`, like `load_screen`, so the
  target always has a slot.
- Every request sets `any_req`. `drain()` runs only at depth 0, in `enter`
  and after `lv_timer_handler`:

  ```rust
  if REG.hdr.any_req.load(SeqCst) {
      REG.hdr.any_req.store(false, SeqCst);
      for s in REG.slots.iter().take(hwm as usize) { act(s) }  // req loaded with SeqCst; no bounds check
  }
  apply_pending_load();
  #[cfg(feature = "heap")] if !deferred_head.is_null() { free_deferred_list() }
  ```

  Deletes and cleans run first; the load then applies the checks of
  `load_screen` (8.3), and a dead target discards it silently. The
  deferred-free list (9.1) is freed whenever it is non-empty, whether or
  not `any_req` was set, so a box waits for no unrelated request.
- `act` on an OWNED slot whose `req` is set, or on any slot with P_DELETE
  or P_CLEAN. On a LIVE slot it first runs the structural checks (8.3); on
  `Busy` it leaves `req` and the bits set and stores `any_req = true`
  (SeqCst), so the next drain retries at no cost. Then:
  - **(a) OWNED, `req` set** (the `Owned` was dropped or deleted):
    1. if the slot is LIVE, call `lv_obj_delete` (or `lv_timer_delete` /
       `lv_group_delete` for the other kinds) with OWNED still set: the
       hook turns an object slot DEAD and does not free it; the drain marks
       a timer or group slot DEAD itself;
    2. free the slot exactly once (generation incremented, pushed on the
       FIFO);
    3. clear `req`, OWNED and the P_* bits. The token is held throughout,
       so steps 2 and 3 may be swapped; clearing OWNED before step 1 would
       let the hook free the slot too and push it on the FIFO twice.
  - **(b) OWNED, `req` clear, P_DELETE set** (`delete_later` on an owned
    object, `TimerCtx::delete_later` on a timer whose `Owned<Timer>` is
    alive): delete as in (a) step 1, clear P_DELETE, and leave the slot
    DEAD with OWNED set. It is not freed: the `Owned` still exists and its
    drop will store `req` on this slot without reading `gen` (6.1), so the
    slot stays pinned until a later drain observes `req` and runs (a),
    which skips the delete because the slot is DEAD.
  - **(c) not OWNED, P_DELETE set** (only objects reach this case: a
    timer or group slot keeps OWNED until its `req` is observed): call
    `lv_obj_delete` without setting OWNED. The hook frees the slot exactly
    once (pending bits and TEMP cleared, generation incremented, pushed on
    the FIFO), and the drain does not touch it afterwards.
  - **P_CLEAN**: call `lv_obj_clean` on a LIVE slot and clear only P_CLEAN;
    the slot is otherwise left as it was.
- A TEMP slot is released once its request completes,
  unless a `Handle` to it was handed out in the meantime: every path that
  returns or stores one (`handle()`, `own()`, `target_handle()`, relations,
  `subscribe_owned`) clears TEMP.
- No lost request: the drain's clear of `any_req` and its `req` loads are
  SeqCst (a `fence(SeqCst)` after the clear followed by Relaxed loads is
  equivalent). Under SeqCst's total order, a drop whose flag store follows
  the clear leaves the flag set; one that precedes it has already stored
  `req`, which the scan sees. The Relaxed `req` load in `resolve` is only
  a conservative fast path; correctness does not depend on it.
- No ABA: an OWNED slot is pinned until the drain observes its request,
  even when a P_DELETE has already deleted its resource (case b); only a
  slot with no `Owned` is freed by the delete itself (case c). The hook
  clears pending bits on slots that are not owned, and the pending load
  when it names the dying slot.
- There is no slotless path. `lv_obj_delete_async` would skip the checks at
  execution time, and LVGL cancels pending async deletes only for its own
  callback (`lv_obj_tree.c:721-725`), so a crate callback that re-checks
  could not be cancelled when the object dies first.
- A pending delete makes `resolve` return `Dead` immediately.

### 8.3 Structural checks

One non-generic function, about 300 B plus a table of 4 to 8 B per part,
run by `own`, `ObjMut::delete`, `clean`, `set_parent`/`set_parent_to`,
child reordering, `delete_later`/`clean_later` (when requested and when
drained), and the drain:

- display layers (created at `lv_display.c:156-159`) cannot be deleted,
  owned or reparented. LVGL does not forbid deleting one, but
  `lv_display_delete` later deletes the layers again through stored
  pointers (`lv_display.c:220-232`), a double free, and
  `update_resolution` dereferences them (`lv_display.c:1571-1581`);
  `lv_obj_delete` clears only `act_scr` (`lv_obj_tree.c:65-85`). LVGL
  already refuses to reparent a parentless object
  (`lv_obj_tree.c:154-157`), so for `set_parent` the check is defence in
  depth;
- **composite parts.** Object X is a part of composite C when one of C's
  part expressions in `classes.toml` (a getter, `child:N`, or a `shim:`
  accessor for parts with no public getter, 3.1) returns X. The check walks
  up at most `max_depth` ancestors of X. For each ancestor whose class
  matches a composite through `lv_obj_has_class` (a `base_class` walk,
  `lv_obj.c:1279-1291`, so spinbox inherits textarea's rule), it evaluates
  C's parts at the time of the call, never cached, because msgbox parts
  appear lazily (`lv_msgbox_add_title`, `lv_msgbox_add_footer_button`).
  Cost: at most `max_depth` parent lookups and a few getter calls, no tree
  walk.
  - delete, own, set_parent and reordering are refused (`Internal`) when X
    is a part of C or a strict ancestor of one;
  - clean is refused when X is a composite with parts (a msgbox or a
    textarea, even if only user children would go) or a strict ancestor of
    a part of C;
  - user children of a msgbox `content`, and the labels of
    `lv_msgbox_add_text`, are not parts: they can be deleted and owned;
  - a part is an object LVGL reaches through a cached pointer that it does
    not reset on delete, or by child index: the dropdown's list is reset
    (`lv_dropdown.c:790, 830`), so it is not a part. The audit generates or
    checks the lists (msgbox, textarea, menu, roller, tabview, win,
    calendar, ...);
  - the menu widget is deprecated in 9.6: every `lv_menu_*` function
    carries `LV_DEPRECATED` (`lvgl/include/lvgl/widgets/lv_menu.h:83`
    onwards) and its getters call `LV_LOG_DEPRECATED` (`lv_menu.c:519`).
    The skip-deprecated rule (4.5) therefore generates no safe menu API.
    Its part table still exists, to protect menu objects that safe code
    reaches through views (a menu created through `sys`, then traversed
    with `child` or `parent`). Every part goes through a `shim:` accessor
    (3.1), never a deprecated getter, and the cached pointers LVGL never
    resets (`lv_menu.c:284-300` and the `lv_menu_refr_*_header_mode`
    functions dereference them) are all listed;
- **indexed composites** (tabview): set_parent, delete and reordering are
  refused (`Internal`) when X is the bar, the content, a page or a bar
  button; clean is refused on the bar and on a bar button; delete,
  set_parent and clean are refused on a bar button's label
  (`child_by_type(button, 0, label)`); and set_parent is refused when the
  old or new parent is the tabview, its bar or its content. LVGL finds the
  bar and the content by child index (`lv_tabview.c:306, 312`), pages by
  child count (114), buttons by typed index (128, 186, 292) and their
  labels by typed index (129, 142). `lv_tabview_set_tab_text` passes the
  button and label lookups on unchecked (126-130), so a missing button or
  label is a NULL dereference with `LV_USE_CHECK_ARG 0` and
  `LV_USE_ASSERT_OBJ 0`, and the index check does not help, because the
  tab count comes from the content's child count. A label sits three
  levels below the tabview, hence `max_depth = 3`;
- `set_parent` rejects the object itself and any of its descendants (LVGL
  has no cycle guard: `lv_obj_tree.c:149-179`);
- a screen equal to `lv_display_get_screen_loading()` or
  `lv_display_get_screen_prev()` cannot be deleted (`Busy`, the drain
  retries): LVGL does not clear them when they are deleted during an
  animated load (`lv_display.c:892-895`, `lv_refr.c:413, 1077-1100`);
- `load_screen` requires a parentless object: `lv_obj_delete` clears
  `act_scr` only for parentless objects (`lv_obj_tree.c:66-85`).

### 8.4 LVGL-internal deleters

These are reachable only from `Exclusive` entry points (`timer_handler` or
exclusive wrappers), which the audit proves. The full list, which the
audit's golden output is compared against:

- the msgbox close button on CLICKED (registered at `lv_msgbox.c:259`,
  handler `lv_msgbox.c:311-322`);
- `delete_on_screen_unloaded` (`lv_obj.c:2196-2199`);
- screen loads (`lv_display.c:913-915, 940-942, 1730-1733`);
- the menu sidebar (`lv_menu.c:362-363`);
- the dropdown destructor (`lv_dropdown.c:789`);
- the pinyin IME destructor (raw-only);
- `lv_obj_delete_anim_completed_cb` (`lv_obj_tree.c:135-140`, scheduled by
  `lv_obj_delete_delayed`), run by the animation timer;
- `lv_obj_delete_async_cb` (`lv_obj_tree.c:645-648`, scheduled by
  `lv_obj_delete_async` and by `lv_msgbox_close_async` at
  `lv_msgbox.c:299-305`), run by the async timer;
- the fragment delete (`lv_fragment.c:127`);
- the nuttx touchscreen cursor (`lv_nuttx_touchscreen.c:103`,
  driver-only, raw);
- display deletion (`lv_display.c:220-239`).

LVGL cancels both scheduled deletions when the object dies first
(`lv_obj.c:1531` deletes its animations, `lv_obj_tree.c:724` cancels the
async call), and the hook invalidates every handle when the object goes.
The functions that schedule them are still Raw (4.7), because the
deletion skips the structural checks of 8.3.

### 8.5 Terminal `deinit`

In order: drain pending requests; return `Busy` while any display's
`DisplayCore.flushing` reads true (Acquire, 9.5); delete the crate's timers
(freeing `timer_boxed` closures) and groups; delete every display (all
hooks fire, and owned queue boxes are freed by their DELETE); free the
deferred-free list; call `lv_deinit()`; set `init = 2`, after which `init`
and `enter` always fail with `NotInitialized`.

After `Busy` the session is gone; the caller re-enters with the same guard
and retries. That is cheap, because `Session` is zero-sized, and nothing
needs undoing, because `init` is still 1 and the steps before the check
leave LVGL fully usable.

## 9. Retained data, callbacks, drivers

### 9.1 Retained data

- Core, no heap:
  - `set_text_static(&'static CStr)` (text stays in flash), `&'static Font`,
    `&'static ImageDsc`;
  - `ButtonMap`, grid and scale arrays from `const` builders that
    guarantee the terminator;
  - buffers that LVGL writes take `&'static mut [T]` or an `UnsafeCell`
    buffer type, and length-bound arrays are checked (4.6).
- `ConstStyle`, from `const_style! { ... }`: entirely in `.rodata`, 0 B of
  RAM and heap. The macro builds the bindgen `lv_style_t` with a struct
  literal that names every field, with `prop_cnt = 255` and
  `has_group = !0` (`LV_STYLE_CONST_INIT` semantics,
  `lvgl/include/lvgl/core/lv_style.h:52-70`). When the bindings report
  `LV_USE_ASSERT_STYLE 1`, the generator exports a cfg and the macro also
  sets `sentinel: LV_STYLE_SENTINEL_VALUE` (0xAABBCCDD), which every API
  checking `LV_STYLE_SENTINEL_OK` requires; because the literal names
  every field, a mismatch with the config fails to compile. The macro
  always appends `LV_STYLE_CONST_PROPS_END` (`prop = LV_STYLE_PROP_INV`,
  value 0, line 70) to its property array, where LVGL's property
  walk stops, and rejects `LV_STYLE_PROP_INV` as a user property, which
  would hide the properties after it.
- LVGL refuses `lv_style_set_prop`, `lv_style_remove_prop`, and
  `lv_style_copy`/`lv_style_merge` into a const style
  (`lv_style.c:215-218, 229-232, 301-304, 354-357`), but `lv_style_init`
  and `lv_style_reset` write unconditionally (`lv_style.c:184-207`), which
  faults on a style in flash. So `ConstStyle` is exposed only as
  `&'static ConstStyle` (no interior mutability) and passed as
  `*const lv_style_t` to `lv_obj_add_style`, `lv_obj_remove_style`,
  `lv_obj_replace_style` (both style arguments) and the style getters.
  `lv_style_init`, `lv_style_reset`, the `lv_style_set_*` setters,
  `lv_style_remove_prop` and the copy/merge destination exist only on the
  mutable `Style`, never on a trait shared with `ConstStyle`.
- `Style`: a static `lv_style_t` (12 B, 16 B with `LV_USE_ASSERT_STYLE`)
  mutated under the token, followed by `report_style_change`.
- `heap` feature: setters whose retained parameter is read-only get an
  owned variant that takes the value, for example `img.set_src_owned(dsc)`
  with `T: Send + 'static`. It places `T` in an `LvBox` on LVGL's heap
  (header `next` + `T`) and installs a filter-DELETE descriptor on the same
  object. The DELETE descriptor fires before the destructors, and LVGL
  still holds the value's address in the image header cache, the image
  cache (keyed by the `src` pointer) and draw-unit resources. So it calls
  `lvgl_sys_image_cache_drop(src)` for image sources, which reads `*src`
  through `lv_image_src_get_type` and notifies the draw units
  (`lv_image_cache.c:81-96`), then moves the box onto the deferred-free
  list. Freeing with entries still cached would let a later allocation at
  the same address hit a stale entry. The list is freed at the next
  depth-0 `enter` or `timer_handler` (8.2), after the object's
  destructors. No destructor dereferences retained data:
  `lv_image_destructor` frees only LVGL-owned FILE and SYMBOL copies
  (`lv_image.c:700-710`). The value is never handed out as a plain `&T`,
  so a setter that needs `'static` cannot receive data kept alive by
  another object.
- `LvBox` layout: `#[repr(C)] struct LvBox<H, T> { hdr: H, value: T }`
  placed at the start of the LVGL block when
  `align_of::<T>() <= LV_HEAP_ALIGN`: the header (at most 4 B on 32-bit)
  is padded to `T`'s alignment and the budgets of section 12 hold.
  `LV_HEAP_ALIGN` is `size_of::<usize>()` in every allocator mode: the
  builtin TLSF aligns to exactly that on every target in scope
  (`lv_tlsf.c:50-53, 235-242`), C `malloc` to at least that, and an
  `LvglAllocator` must guarantee it (9.6). For a more aligned `T`
  (`u64`, `f64` or `Duration` on 32-bit targets, `u128` on x86_64), the
  constructor chooses at compile time to over-allocate by
  `align_of::<T>() - LV_HEAP_ALIGN` bytes, keep the header at the block
  start and compute the value's address from the block pointer, which
  stays the pointer stored and freed. Every `LvBox` follows this rule
  (owned setters, `on_boxed`, `subscribe_owned`, `create_boxed`,
  `timer_boxed`), so code that builds on a 64-bit host also builds on
  32-bit targets.

### 9.2 Event callbacks

```rust
pub struct Event<'cb> { e: NonNull<lv_event_t>, _p: PhantomData<(&'cb Lvgl, *mut ())> }
impl Event<'_> {
    pub fn lv(&self) -> &Lvgl;
    pub fn code(&self) -> EventCode;
    pub fn target(&self) -> Option<&Obj>;                // None while deleting
    pub fn target_handle(&self) -> Option<Handle<Obj>>;  // lookup only, never mints
    pub fn target_is(&self, h: Handle<Obj>) -> bool;
    pub fn param_obj(&self) -> Option<&Obj>;             // CHILD_CHANGED, CHILD_CREATED only
    pub fn key(&self) -> Option<u32>;                    // KEY only
    pub fn point(&self) -> Option<Point>;                // pointer codes only
    pub fn stop_bubbling(&self);
    pub fn stop_processing(&self);                       // no-op for DELETE
}
/// Padding-free values of at most 4 B on every target, stored by value in `user_data` (sealed).
pub unsafe trait UserData: Copy + Send + 'static {}
/// A context and its function in one static; `user_data` points at it.
pub struct Handler<C: Sync + 'static> { pub ctx: C, pub f: fn(&Event<'_>, &'static C) }
impl Obj {
    pub fn on_fn(&self, code: EventCode, f: fn(&Event<'_>)) -> Result<(), Error>;
    pub fn on<F>(&self, code: EventCode, f: F) -> Result<(), Error>;  // F: for<'cb> Fn(&Event<'cb>) -> R + Send + 'static, zero-sized
    pub fn on_with<D: UserData, F>(&self, code: EventCode, data: D, f: F)
        -> Result<(), Error>;                                         // F: Fn(&Event<'_>, D) -> R + Send + 'static, zero-sized
    pub fn on_static<C: Sync + 'static>(&self, code: EventCode, h: &'static Handler<C>) -> Result<(), Error>;
    #[cfg(feature = "heap")]
    pub fn on_boxed<F>(&self, code: EventCode, f: F) -> Result<(), Error>; // F: Fn(&Event<'_>) -> R + Send + 'static
    pub fn on_removable<D: UserData, F>(&self, code: EventCode, data: D, f: F)
        -> Result<EventSub, Error>;                                   // bounds of on_with
    pub fn send_event(&self, code: SafeEventCode) -> Result<(), Error>;
    pub fn delete_later(&self) -> Result<(), Error>;
    pub fn clean_later(&self) -> Result<(), Error>;
}
```

- Handler functions are `for<'cb> Fn(&Event<'cb>) -> R` with `R` either
  `()` or `Result<(), Error>`; closures must be `Fn` because events
  re-enter (`set_size` in a handler synchronously sends STYLE_CHANGED,
  `add_state` sends STATE_CHANGED, `update_layout` sends SIZE_CHANGED).
  Every stored type is `'static`: `F` in every `on*`, `D`, and the
  `on_boxed` closure. Every closure type is also `Send`, zero-sized or
  not: sessions move between threads (5), so the trampoline may form `&F`
  on another token holder's thread than the one that registered it, which
  is a move of `F` across threads.
- Zero-sized `F` (`on`, `on_with`, `on_removable`, timers): the bound is
  `F: for<'cb> Fn(&Event<'cb>, ..) -> R + Send + 'static`, and zero size is a
  post-monomorphization `const { assert!(size_of::<F>() == 0) }` inside the
  function. The function calls `mem::forget(f)`, so values moved into the
  closure are never dropped (leak semantics, correct for HAL singletons
  and guards), and the trampoline, monomorphised over `F`, only forms a
  shared `&F` from `NonNull::<F>::dangling().as_ref()`, valid for a
  zero-sized type. It never creates, copies or drops an `F` by value. A
  closure with a non-zero-sized, non-`'static` or non-`Send` capture (a
  `!Send` zero-sized token with thread affinity) is rejected.
- `UserData` is implemented only for types of at most 4 B on every target,
  so host builds reject the same `D` as wasm32, ESP32 and Cortex-M builds:
  `u8` to `u32`, `i8` to `i32`, `f32`, `bool`, `char`, fieldless
  `#[repr(u8|u16|u32)]` enums, `Handle<W>` and `Option<Handle<W>>`. A
  `&'static T` context goes through `on_static` instead (a reference is
  8 B on 64-bit hosts). The value is encoded as
  `let mut p = ptr::null_mut::<c_void>(); ptr::write_unaligned((&raw mut p).cast::<D>(), d);`
  and decoded with `ptr::read_unaligned((&raw const p).cast::<D>())` from
  a local copy of `user_data`. Starting from null means the slot never
  holds uninitialized bytes, and padding, which would, is ruled out by the
  bound; nothing goes through `usize`, `transmute_copy` or
  `without_provenance`. Timers and animations use the same encoding (9.3).
- `on_static` stores `&'static Handler<C>` in `user_data`; one trampoline
  per `C` calls `(h.f)(e, &h.ctx)`. A `Handler` is built in a `static`, so
  it costs no heap.
- `on_with` stores `D` by value in `user_data`: per-instance context with no
  heap and no statics.
- `on_boxed` uses one filter-ALL descriptor and an `LvBox` whose header is
  `{code: u16, busy: u8, flags: u8}` (4 B, in place of `next`: these boxes
  never go on the deferred-free list); the trampoline calls `F` only for
  its code. On DELETE (which never bubbles) it frees the box, or marks it
  DOOMED if `busy > 0` so the outermost call frees it.
- No closure is freed while running: freeing one requires deleting its
  object, which by invariant C cannot happen while a shared callback runs.
  BUSY/DOOMED is defence in depth; it is required for animations, because
  `lv_switch.c:338`, `lv_spinner.c:95` and `lv_animimage.c:123` delete all
  animations of an object from shared setters.
- Trampolines are `extern "C"` (an unwinding panic aborts). Before user
  code they check `depth < LVGL_RUST_MAX_CALLBACK_DEPTH`; only then do they
  increment `depth` (and the box or queue `busy` count), run user code and
  decrement. At the limit they skip user code without touching `depth`,
  and still run the crate's own DELETE work (frees, relation clears). With
  the limit in 1..=254 (4.1), `depth` and `busy` stay at or below 254, so
  plain `u8` increments cannot overflow (checked with `debug_assert`);
  saturating arithmetic is not used, because it would desynchronise the
  matching decrements.
- Refresh-time codes (`COVER_CHECK`, `DRAW_*`, `DRAW_TASK_ADDED`), sent
  inside the rendering window (`lv_refr.c:816-871`), are never delivered to
  safe handlers: `on*` rejects them with `InvalidArg`, and trampolines skip
  user code for them under a filter-ALL descriptor. A shared setter called
  there would free memory that a queued draw task still reads (the label
  task keeps `label->text`, `lv_label.c:936`), and the guard in
  `lv_inv_area` (`lv_refr.c:293`) is an `LV_ASSERT_MSG`, compiled out with
  `LV_USE_ASSERT 0` (`lvgl/include/lvgl/debugging/lv_assert.h:59-71`) as in the vendored config. The
  audit report also lists setters that free before they invalidate.
- Param accessors check `code()` against a fixed allow-list first, then
  null-check `param`, and only then read it. The check runs in Rust and
  never relies on `LV_USE_CHECK_ARG`, under which alone LVGL's getters test
  the code (`lv_obj_event.c:202-344`):
  - `param_obj`: CHILD_CHANGED and CHILD_CREATED only
    (`lv_obj_class.c:131-132`, `lv_obj_tree.c:179, 184`), `None` for a NULL
    param (`lv_obj_tree.c:77, 112, 185`), with the same deleting check as
    `target()`;
  - `key`: KEY only, the param being a `uint32_t *` (`lv_group.c:312`);
  - `point`: only the pointer codes that LVGL's input code sends with the
    input device as param (PRESSED, PRESSING, PRESS_LOST, RELEASED, the
    CLICKED family, LONG_PRESSED*, HOVER_*, GESTURE, LEAVE), read through
    `lv_indev_get_point`. The code list of `lv_event_get_indev`
    (`lv_obj_event.c:206-216`) is not reused: it also lists KEY and
    SCROLL_BEGIN, whose param is a key or an animation
    (`lv_obj_scroll.c:365`). Any other code, including registered ones,
    returns `None`.
- Sending: `Obj::send_event` takes `SafeEventCode` (sendable, not deleting)
  and `ObjMut::send_event` takes `SendableEventCode` (sendable, deleting
  codes included). Both are newtypes generated into `gen.rs` from the
  `sendable` and `deleting` lists of `events.toml`, with one associated
  const per code and no constructor from `EventCode`. The codes left out (DELETE, CHILD_*, SCREEN_*,
  SIZE/STYLE/LAYOUT_CHANGED, DRAW_*, COVER_CHECK, HIT_TEST,
  REFR_EXT_DRAW_SIZE, GET_SELF_SIZE, INSERT, widget codes with a param such
  as the gstreamer state event, `lv_gstreamer.c:406`) have handlers that
  read `param` unconditionally when `LV_USE_CHECK_ARG` is 0. KEY goes only
  through `ObjMut::send_key`, which passes a pointer to a live `u32`
  (`lv_textarea.c:904`, `lv_spinbox.c:537` and `lv_table.c:615` dereference
  it). Codes from `lv_event_register_id` stay sendable.
- Removal only through `EventSub::remove(self, &mut Lvgl)`. `EventSub` holds
  the handle, the descriptor pointer and its trampoline (12 B, 24 B on
  64-bit). `remove`
  resolves the handle to Y, walks Y's event list
  (`lv_obj_get_event_count`, `lv_obj_get_event_dsc`) comparing addresses,
  so a stale pointer is never dereferenced, then checks
  `lv_event_dsc_get_cb` against the stored trampoline and removes by index
  (`lv_obj_remove_event`). Removable trampolines are a family of their own
  whose code differs from every crate-internal one (relations, owned
  setters, `on_boxed`, queues, `bubble_children`), so identical-code
  folding cannot merge them, and an alias after a generation wrap can at
  most remove another removable subscription whose `D` owns nothing: a
  logic error. There is no removal by callback identity alone, which
  merged functions would break. The same non-generic list walk validates
  `QueueRef` (10).
- `HandleCell<W>`: an `AtomicU32` with load/store only (4 B) for handles in
  statics.

### 9.3 Timers, animations, observers, groups

```rust
impl Lvgl {
    pub fn timer<D: UserData, F>(&self, period_ms: u32, data: D, f: F)
        -> Result<Owned<Timer>, Error>;          // F: Fn(&TimerCtx<'_>, D) + Send + 'static, zero-sized
    #[cfg(feature = "heap")]
    pub fn timer_boxed<F>(&self, period_ms: u32, f: F) -> Result<Owned<Timer>, Error>; // F: Fn(&TimerCtx<'_>) + Send + 'static
    pub fn create_group(&self) -> Result<Owned<Group>, Error>;
}
impl TimerCtx<'_> { pub fn lv(&self) -> &Lvgl; pub fn handle(&self) -> Handle<Timer>; pub fn delete_later(&self); }
```

- A timer occupies a registry slot (`KIND = TIMER`). `D` is stored by value
  in `lv_timer_t.user_data` with the `UserData` encoding, so a core timer
  allocates nothing besides LVGL's own timer.
- Timers have no `obj->id`, and `user_data` holds `D` (or the
  `timer_boxed` box), so nothing stores the slot index. `TimerCtx` holds
  the `lv_timer_t` pointer, and `TimerCtx::handle()` and `delete_later()`
  find the slot with one non-generic scan of
  `REG.slots.iter().take(hwm)` for a LIVE slot with `KIND = TIMER` and
  `ptr == timer`, under the token. It costs no RAM and about 40 B of
  flash, and runs O(hwm) only when one of the two is called, never on
  every tick. The scan always finds the slot: a timer the crate created
  holds one until the drain deletes it (8.2).
- The timer is created with `auto_delete = false`, so a repeat count that
  runs out pauses it (`lv_timer.c:371-379`) instead of freeing it behind
  the registry. `Handle<Timer>` exposes pause, resume, period and repeat
  count, which are safe for that reason.
- `Owned<Timer>::drop` and `TimerCtx::delete_later` defer through the `req`
  byte or P_DELETE; the drain calls `lv_timer_delete`. It frees the slot
  once `req` is observed: after `delete_later` with the `Owned<Timer>`
  still alive, the slot stays DEAD and OWNED until the drop (8.2, case b).
  Only `timer_boxed` owns an allocation: an `LvBox` with the `on_boxed`
  header (busy and DOOMED), freed by the drain or by `deinit`.
- Animation state lives in `anim.user_data` with the `UserData` encoding;
  `exec` and `completed` get a shared context with a checked view of `var`;
  `deleted` gets only the data. `lv_anim_t` receivers are Manual (4.5), so
  their callback and `user_data` setters are never generated.
- Subjects are `&'static`; observer callbacks get shared access.
- Groups (M5): a group takes a slot (`KIND = GROUP`), because
  `Owned<Group>` drops through the store-only request byte and
  `lv_group_delete` stays crate-only (it also detaches the group from every
  indev, `lv_group.c:93-100`). `Handle<Group>::add_obj`,
  `Handle<Group>::remove_all` and `ObjMut::remove_from_group` are
  exclusive, because focus changes iterate the group's object list while
  sending FOCUSED and DEFOCUSED (4.7). `Indev::set_group` attaches a group
  (9.5); `lv_group_get_default` and `lv_group_by_index` are Raw.

### 9.4 Relations

`relations.toml` lists functions that store one object inside another
(`lv_keyboard_set_textarea`, ...). The safe wrapper installs a
clear-on-delete descriptor on the target (a 16 B block, a 32 B block on
64-bit, only when used) whose `user_data` holds the holder as a `Handle<Obj>`, minted or
deduped through `obj->id` when the relation is bound (`RegistryFull` is
possible). On the target's DELETE, the callback resolves the handle first
and does nothing if the holder is dead; only then does it test
`getter(holder) == target` and call the listed clear action. Rebinding the
same target does not stack a second descriptor. The first descriptor on an
object that had none also allocates the event array (a 16 B block) and,
if the object had no `spec_attr` yet, that block (60 B; 80 B on 64-bit). Relations that cannot be
cleared (`lv_indev_set_cursor`, `lv_ime_pinyin_set_keyboard`,
`lv_menu_set_load_page_event`) are raw-only, and so are all relation
wrappers with `LVGL_RUST_HANDLE_SLOTS=0`.

### 9.5 Display and input drivers

Driver callbacks get no token, so they cannot touch the object tree.

```rust
pub trait Flush: Send + 'static { fn flush(&mut self, req: FlushRequest<'_>); }
impl FlushRequest<'_> {
    pub fn area(&self) -> Area;
    pub fn pixels(&mut self) -> FlushPixels<'_>;   // render-mode and stride aware
    pub fn defer(self) -> FlushDone;               // dropping without defer() = done now
}
pub struct FlushDone(NonNull<DisplayCore>);        // Send, ISR-safe
impl FlushDone { pub fn ready(self); }             // one Release store, its last access

#[repr(C)] pub struct DisplayCore { flushing: AtomicBool }               // 1 B
#[repr(C)] pub struct DisplayCell<D> { core: DisplayCore, driver: MaybeUninit<D> } // application static
pub struct IndevCell<R>(MaybeUninit<R>);           // no crate state

#[repr(C, align(4))]                               // in gen.rs: LV_DRAW_BUF_ALIGN (4 in conf/), 4.8
pub struct DrawBuf<const BYTES: usize>([u8; BYTES]);

impl Display {                                     // 8 B owner
    pub fn create<D: Flush>(lv: &mut Lvgl, cell: &'static mut DisplayCell<D>, driver: D,
                            width: u32, height: u32, bufs: DrawBuffers<'static>, mode: RenderMode)
        -> Result<Display, Error>;
    #[cfg(feature = "heap")]
    pub fn create_boxed<D: Flush>(lv: &mut Lvgl, driver: D, width: u32, height: u32,
                                  bufs: DrawBuffers<'static>, mode: RenderMode) -> Result<Display, Error>;
    pub fn try_delete(self, lv: &mut Lvgl) -> Result<(), (Self, Error)>; // Busy while flushing
}
```

- Driver state lives in `driver_data`, so `user_data` stays free for the
  application. `DisplayCell<D>` is an application static holding the
  crate's `DisplayCore` (the `flushing` flag, 1 B, padded to `D`'s
  alignment) and `D`; `create_boxed` puts both in an `LvBox`. `flushing`
  needs only atomic load and store, which `thumbv6m` and `riscv32imc`
  have. `IndevCell<R>` holds only the reader. `Display` and `Indev` own
  the LVGL pointer and the cell pointer (8 B on 32-bit).
- Flush completion never touches `lv_display_t` from another thread or an
  ISR:
  - `DisplayCore` holds `flushing: AtomicBool`; the flush trampoline stores
    true before calling `D::flush`;
  - `FlushDone::ready` and a `FlushRequest` dropped without `defer()` do a
    single Release store of false, their last access to `DisplayCore`, and
    never call `lv_display_flush_ready`;
  - the display gets a crate `flush_wait_cb` that waits (spin, or a
    platform wait hook) until `flushing` reads false with Acquire; LVGL
    then clears `disp->flushing` itself, on its own thread
    (`lv_refr.c:1493-1497`). `disp->flushing` is a plain `volatile int`
    that LVGL spins on (`lvgl/src/display/lv_display_private.h:88`, `lv_refr.c:1500`):
    writing it from another core would race, and a store made after
    `lv_display_flush_ready` could clear the flag of the next flush;
  - `try_delete` and `deinit` read the atomic with Acquire and return
    `Busy` while it is true;
  - `D` and the atomic are reached through raw-pointer projections, so no
    `&mut DisplayCore` exists while an ISR writes the flag.
- Dropping a `Display` or `Indev` does nothing (counted as a leak with
  `debug-counters`); deletion is explicit.
- Every function that changes the shape or ownership of the draw buffers is
  Manual or Raw in the overrides, because the audit only looks for frees
  and `LV_USE_CHECK_ARG=0` removes LVGL's own checks
  (`lv_display.c:550-571, 691-692`). All are exclusive:
  - `set_render_mode`: PARTIAL is always allowed; FULL or DIRECT requires
    `stride(original width, colour format) × original height` to fit each
    stored buffer;
  - `set_tile_cnt`: 1 ≤ n < 256 (a count of 0 divides by zero,
    `lv_refr.c:937`);
  - `set_color_format`, `set_resolution` and `set_rotation` re-check the
    buffer size and stride against the new format or geometry in every
    config. LVGL checks buffer size and alignment only in
    `lv_display_set_buffers_with_stride` (`lv_display.c:550-571`), and
    only with `LV_USE_CHECK_ARG 1`; these setters never do:
    `set_color_format` rewrites `header.cf` of the existing buffers
    (`lv_display.c:658-660`), and `set_resolution`/`set_rotation` only call
    `update_resolution` (`lv_display.c:1554-1591`);
  - `set_draw_buffers`, `set_3rd_draw_buffer`, `set_buffers` and
    `set_buffers_with_stride` are Raw, or methods taking a new
    `DrawBuffers<'static>` with the same check;
  - `set_physical_resolution` and `set_offset` do not change what LVGL
    writes into the buffers and stay generated.
- `Indev` mirrors `Display` with a `Read` trait, plus
  `Indev::set_group(&mut self, lv: &mut Lvgl, group: Option<Handle<Group>>)`.

### 9.6 Custom allocator

```rust
pub unsafe trait LvglAllocator: Sync {
    fn alloc(&self, size: usize) -> *mut u8;                    // aligned to LV_HEAP_ALIGN
    unsafe fn realloc(&self, ptr: *mut u8, new_size: usize) -> *mut u8;
    unsafe fn free(&self, ptr: *mut u8);
    fn monitor(&self) -> Option<MemMonitor> { None }
}
lvgl_rust::lvgl_allocator!(MY_ALLOCATOR);
```

- Feature `allocator`. `build.rs` checks
  `LV_USE_STDLIB_MALLOC == LV_STDLIB_CUSTOM`.
- The macro exports the symbols LVGL expects in that mode:
  `lv_mem_init`, `lv_mem_deinit`, `lv_mem_add_pool`, `lv_mem_remove_pool`,
  `lv_malloc_core`, `lv_realloc_core`, `lv_free_core`,
  `lv_mem_monitor_core`, `lv_mem_test_core`, with defaults for the
  optional ones.
- Contract: `alloc` and `realloc` return pointers aligned to at least
  `LV_HEAP_ALIGN` (`size_of::<usize>()`). `realloc` is never called with
  NULL, size 0 or LVGL's `zero_mem` sentinel, and `free` never with NULL:
  `lv_malloc`, `lv_realloc` and `lv_free` handle those cases
  (`lv_mem.c:59-65, 131-160`), and the macro's `lv_realloc_core` maps the
  NULL that `lv_realloc` forwards to `alloc(new_size)`.
- `SizedHeader<A: GlobalAlloc>` adapts a Rust allocator, because LVGL's
  `realloc` does not pass the old size. It allocates
  `Layout::from_size_align(size + H, ALIGN)` from `A`, stores the size in a
  header of `H = max(size_of::<usize>(), ALIGN)` bytes and hands LVGL
  `base + H`, so LVGL's pointers keep `ALIGN` alignment. `ALIGN` defaults
  to `LV_HEAP_ALIGN` (a 4 B header on 32-bit, 8 B on 64-bit). Draw buffers
  need no more: LVGL over-allocates them by `LV_DRAW_BUF_ALIGN - 1` and
  aligns them itself (`lv_draw_buf.c:614-620`).
- LVGL's own allocations and the crate's `heap` feature then share that
  single pool.

### 9.7 Custom components

v1 provides composite components, which need no private API:

```rust
pub trait Component: Sized {
    type Root: Widget;
    fn root(&self) -> Handle<Self::Root>;
}
// A component is a plain struct holding its root Owned<...> and child Handles, built by an
// associated function in one of two forms:
//   (a) fn build(parent: &Obj) -> Result<Self, Error>                       // default
//   (b) fn build(parent: Handle<Obj>, lv: &mut Lvgl) -> Result<Self, Error>  // needs Exclusive calls
```

- Form (a) makes only Shared calls: creating children and registering
  handlers free no object, so the parent view stays valid under invariant
  V, and the parent costs no slot.
- Form (b) resolves `parent.get(lv)` or `get_mut(lv)` after each exclusive
  call, never across one, and costs one parent slot (8 B on 32-bit).
- Neither form uses `Lvgl::assume` to hold a view and `&mut Lvgl` at once.

Deleting the root deletes the children and their handles resolve to
`Dead`; handlers use `on_with` to carry a handle or index. Cost: nothing
beyond the parts. Components need slots (6.5). `CustomClass<T>` (Rust state
inside the object) is a later milestone gated on `LV_USE_PRIVATE_API=1`.

## 10. Async events (feature `async`)

Queues are accessed only under the token, so they need no atomics, no
`critical-section` and no CAS.

```rust
pub struct EventQueue<E: Copy + Send + 'static, const N: usize> {
    head: u8, len: u8,
    flags: u8,                 // CLOSED | OVERFLOW | RX_TAKEN | DOOMED
    busy: u8,                  // trampoline frames running on this queue
    waker: Option<Waker>,
    capture: fn(&Event<'_>) -> Option<E>,
    buf: [MaybeUninit<E>; N],
}
pub struct StaticQueue<E, const N: usize>(UnsafeCell<EventQueue<E, N>>);
unsafe impl<E: Copy + Send, const N: usize> Sync for StaticQueue<E, N> {} // token discipline (6.1)
impl<E: Copy + Send + 'static, const N: usize> StaticQueue<E, N> {
    pub const fn new(capture: fn(&Event<'_>) -> Option<E>) -> Self;       // const { assert!(N >= 1 && N <= 255) }
    pub fn receiver(&'static self, lv: &Lvgl) -> Option<StaticRx<E, N>>;   // takes RX_TAKEN once, !Clone
}
pub struct QueueRef<E, const N: usize> { obj: Handle<Obj>, boxp: NonNull<LvBox<(), EventQueue<E, N>>> } // 8 B, !Clone; block pointer
impl Obj {
    pub fn subscribe<E: Copy + Send + 'static, const N: usize>(&self, q: &'static StaticQueue<E, N>, opts: SubOpts)
        -> Result<(), Error>;
    #[cfg(feature = "heap")]
    pub fn subscribe_owned<E: Copy + Send + 'static, const N: usize>(&self, opts: SubOpts,
        capture: fn(&Event<'_>) -> Option<E>) -> Result<QueueRef<E, N>, Error>; // same N assert
}
#[cfg(feature = "heap")]
impl<E, const N: usize> QueueRef<E, N> { pub fn close(self, lv: &mut Lvgl); }
pub enum Recv<E> { Event(E), Empty, Closed }
pub trait Receiver { type Item; fn try_recv(&self, lv: &Lvgl) -> Recv<Self::Item>; fn register(&self, lv: &Lvgl, w: &Waker); }
pub async fn next_event<A: LvglAccess, R: Receiver, F: Future<Output = A>>(lock: impl FnMut() -> F, rx: &R)
    -> (A, Option<R::Item>);
```

- `capture` runs inside the callback, so the application chooses its record
  type (Xila could use a 16 B record: code, key, lookup-only target, point).
  `E` is `Send` because records cross threads with the token, as for
  `Mutex<T>: Sync`.
- `SubOpts::bubble_children` adds `EVENT_BUBBLE` to children on
  CHILD_CREATED.
- `StaticQueue` is never freed: DELETE sets the sticky CLOSED flag and
  wakes the waker. `receiver()` tests and sets RX_TAKEN in the plain
  `flags` byte under the token, so a single consumer needs no CAS
  (`thumbv6m` has none).
- Owned queues (`heap`): `subscribe_owned` mints or dedups the subscribed
  object's slot (`RegistryFull`, absent with N = 0) and takes no extra slot
  when the object already has one. `QueueRef` is `{ Handle<Obj>, box }`,
  8 B (16 B on 64-bit), `Send` when `E` is.
  - The box is an `LvBox<(), EventQueue<E, N>>` (9.1). The descriptor's
    `user_data` and `QueueRef.boxp` both hold the block pointer returned
    by the allocator; the queue's address is derived from it with the
    alignment rule of 9.1 (they differ when `E` is aligned above
    `LV_HEAP_ALIGN`, for example a record holding a `u64` on a 32-bit
    target). The same block pointer is compared by the validation walk and
    passed to `lv_free` by `close` and DELETE.
  - `try_recv` and `register` first resolve the handle under the token,
    then confirm that the object's event list holds a descriptor with the
    owned-queue trampoline for `(E, N)` and `user_data` equal to
    `boxp` (the walk of 9.2). Otherwise they return `Closed` (`register` does
    nothing) without touching the box.
  - DELETE wakes the waker and frees the box; events still queued are
    dropped (a `capture` that sees DELETE can record a final event).
  - Dropping a `QueueRef` touches nothing, so it needs no token, deferral
    or CAS: the box stays attached until the object's DELETE.
    `close(self, &mut Lvgl)` removes the descriptor and frees the box at
    once.
- Overflow drops the newest event and sets OVERFLOW.
- Access discipline: the queue is reached only through raw pointers or
  short-lived `&mut` borrows, and no reference into it is live while user
  code or `Waker` vtable code runs:
  - the trampoline increments `busy`, copies the `capture` fn pointer out
    and calls it with no borrow held; it then takes a fresh short borrow,
    re-reads `len` and `flags` (a nested frame may have pushed, overflowed
    or closed the queue), pushes or sets OVERFLOW, moves the waker out with
    `Option::take` (never `clone`), releases the borrow, wakes, and
    decrements `busy`;
  - `register` returns early when `will_wake` holds; otherwise it clones
    the new waker first, swaps it in under a short borrow, releases the
    borrow, then drops the old waker;
  - `try_recv` pops under a short borrow and runs no vtable code;
  - DELETE sets CLOSED and takes the waker under a short borrow, releases
    it, then wakes; an owned box is freed only when `busy` is 0, otherwise
    DOOMED hands the free to the outermost frame, as for `on_boxed`.
- Built only on `core::task::Waker`: no executor dependency.

## 11. Errors and features

```rust
#[repr(u8)] #[non_exhaustive]
pub enum Error { Dead = 1, RegistryFull, OutOfMemory, InvalidArg, Busy, Internal,
                 Reentrant, NotInitialized, AlreadyOwned, TooSmall }
```

- 1 B. Wrappers have no panic path (masks, no `unwrap`); `Display for
  Error` only with `fmt`.
- Variant mapping for the cases that are not obvious: `init` twice is
  `Reentrant`; `init` or `enter` after `deinit` is `NotInitialized`;
  permanent structural refusals of 8.3 (a layer, a composite part, `clean`
  on a composite, an indexed tabview child) are `Internal`; a `set_parent`
  cycle and `load_screen` on an object that has a parent are
  `InvalidArg`; a loading or previous screen is `Busy`. Above
  `LVGL_RUST_MAX_CALLBACK_DEPTH` user code is skipped silently (9.2).
- Enums and flags have a fixed public representation,
  `#[repr(transparent)] struct X(u32)` with associated consts, whatever
  the bindgen alias resolves to (`c_uint`, or `c_int` on MSVC). Generated
  wrappers convert at the FFI boundary with `as` to and from the `-sys`
  alias written by name (`as lv_part_t`), with a local
  `#[allow(clippy::unnecessary_cast)]` as for `c_char`. LVGL 9.6 values
  (`LV_PART_ANY` = 0xF0000, state and flag masks, `PREPROCESS`) fit in 31
  bits, so the casts are lossless and nothing truncates. Flag types
  implement `BitOr`. Index parameters are bounds-checked against the
  matching count getter.
- Bounds that traits cannot express (`F` zero-sized, `EventQueue`'s
  1 ≤ N ≤ 255) are inline `const { assert!(..) }` blocks, which fail only
  at monomorphization. Their trybuild cases therefore build to codegen
  (trybuild's compile-fail runs `cargo build`, and each case instantiates
  the function in a `fn main`).
- `set_text_str(&str)`: an empty string calls `lv_label_set_text(obj, c"")`;
  an interior NUL is `InvalidArg`. Otherwise it calls
  `lv_label_set_text(obj, c"")` first (`lv_label_ins_text` does nothing on
  static text, `lv_label.c:727-728`), then appends 32 B stack chunks (31
  data bytes and a NUL) through `lv_label_ins_text`, cutting only where
  `s.is_char_boundary(i)` holds (backing off up to 3 bytes), because LVGL
  inserts at a UTF-8 character index. Each chunk costs one `lv_realloc`
  and a text refresh, so the cost is O(n²/32).
- Out of memory leaves `label->text` NULL in two places: the initial
  `lv_label_set_text(obj, c"")` frees the old text and then gets NULL from
  `lv_malloc` (`set_text_internal`, `lv_label.c:1071-1078`), and a failed
  chunk realloc stores NULL (`lv_label.c:734-736`). The next
  `lv_label_ins_text` would then run `lv_strlen(NULL)` (`lv_label.c:731`),
  guarded only by an `LV_ASSERT` (`lv_string_builtin.c:224`) that the
  vendored `LV_USE_ASSERT 0` compiles out. So `set_text_str` (the empty
  case included) checks that `lv_label_get_text(obj)` is non-NULL after
  the clear and after every chunk. On NULL it stops, calls
  `lv_label_set_text_static(obj, c"")`, which never allocates, so the
  label always ends with valid text and later `ins_text` calls return
  early on static text, and returns `OutOfMemory`. A failed chunk realloc
  leaks the previous text: `lv_label.c:734` overwrites the only pointer to
  it, and LVGL's realloc does not free on failure. A single
  `lv_label_set_text_fmt(obj, "%.*s", len, ptr)` is not used: LVGL's
  builtin `vsnprintf` reads the byte after the precision
  (`lv_sprintf_builtin.c:824`) and, with precision 0, the whole string
  (813), so it reads past a `&str`.
- `set_text_i32`/`u32` use a roughly 100 B itoa instead of `core::fmt`.
- Style setters are generated as
  `obj.style(Selector::DEFAULT).pad_all(0).radius(0)` (0 B).

| Feature / variable | Effect | Cost when off |
|---|---|---|
| core | session, registry, views, `ObjMut`, `on_fn`/`on`/`on_with`/`on_static`/`on_removable`, timers, groups, `ConstStyle`, display, indev, components | — |
| `heap` | `LvBox` on LVGL's heap: `on_boxed`, owned setters, `subscribe_owned`, `create_boxed`, `timer_boxed`, deferred-free list (+4 B static) | 0 B |
| `allocator` | `LvglAllocator`, `lvgl_allocator!`, `SizedHeader` | 0 B |
| `async` | `StaticQueue`, `EventQueue`, `next_event` (`subscribe_owned` also needs `heap`) | 0 B |
| `fmt` | `set_text_fmt`, `Display for Error` (+6 to 12 KB if `core::fmt` is not already linked) | 0 B |
| `defmt`, `debug-counters` (8 B) | diagnostics | 0 B |
| `use-string-functions`, `color_depth_*`, `shadcn-theme` | passed through to `-sys` | — |
| `LVGL_RUST_HANDLE_SLOTS` | N, rounded up to a power of two, at most 32,768 (error above); `0` = views only (6.5) | — |
| `LVGL_RUST_MAX_CALLBACK_DEPTH` | callback recursion limit, 1..=254 (error outside), default 16 | — |

Unsafe surface: `sys` (re-exported), `LvglAccess` impls, `Lvgl::assume`,
`enter_callback`, `Obj::from_raw`, `unsafe impl Widget` for custom classes,
the draw/decoder/fs registries, varargs, and everything `Raw` in
`coverage.txt`. Raw C callbacks must not synchronously delete or reparent
objects. They defer through `enter_callback()` and `delete_later`, which
apply the structural rules of 8.3; `lv_obj_delete_async` bypasses those
rules and is only for objects they allow (not a layer, a composite part or
a screen being loaded).

## 12. Memory budget

ESP32 (32-bit) with the vendored `conf/lv_conf.h` (`LV_USE_EXT_DATA 0`,
which otherwise grows objects, event descriptors and timers;
`LV_USE_ASSERT_STYLE 0` unless a row says otherwise). LVGL heap figures
are blocks of LVGL's builtin TLSF, `block(n) = max(12, round4(n)) + 4`;
with `lvgl_allocator!` and `SizedHeader` (9.6) they follow the Rust
allocator's rounding plus the 4 B size header instead. Static RAM means
the crate's own `.bss` unless a row names application statics. Flash
figures are estimates; each milestone's size gate replaces those of the
rows it delivers.

| Structure | Static RAM | LVGL heap | Flash (estimate) |
|---|---|---|---|
| `Lvgl`, `Session`, views, `Event`, `ObjMut` | 0 B (4 to 8 B of stack) | 0 B | 0 B |
| Registry slots | 8·N (N=16: 128 B; 32: 256 B; 64: 512 B) | 0 B | 1.5 to 2.5 KB core |
| Registry header (pending load and tick counter included) | 24 B; 8 B with N = 0 | 0 B | in core |
| Deferred deletes | 0 B (byte in the slot) | 0 B | in core |
| `delete_later` / `clean_later` on an object without a slot | 1 slot until the drain | 0 B | in core |
| OBJ_ID hook | 0 B | +4 B per object | ≈180 B |
| Structural checks | 0 B | 0 B | ≈300 B + 4 to 8 B per part |
| Stored `Handle` / `Owned` / `Option<Handle>` | 4 B each | — | — |
| Stored `QueueRef` / `Display` / `Indev` | 8 B each | — | — |
| Stored `EventSub` | 12 B | — | — |
| Event handler (`on*`) | 0 B | same as raw LVGL (16 B descriptor + array growth) | 60 B once; 30 to 60 B per handler type |
| `on_boxed` closure | 0 B | handler + block(4 + size_of F) (≥ 16 B) | 60 to 90 B per closure type |
| Owned setter (`*_owned`) | 0 B | 16 B descriptor¹ + block(4 + size_of T) | ≈50 B per type |
| `LvBox` value aligned above `LV_HEAP_ALIGN` | 0 B | + (align_of T − 4) B per box | — |
| Relation (keyboard → textarea) | 1 slot per distinct holder | 16 B per bound target¹ | ≈80 B |
| Timer | 1 slot | 40 B block, LVGL's own (28 B `lv_timer_t` + 8 B `lv_ll` node: `lv_timer_create` allocates through `lv_ll_ins_head`) | ≈140 B (≈40 B slot lookup for `TimerCtx` included) |
| `timer_boxed` closure | 1 slot | timer + block(4 + size_of F) | 60 to 90 B per closure type |
| Group | 1 slot | LVGL's own (`lv_group_t`, one list node per member) | ≈100 B |
| `DisplayCell<D>` / `IndevCell<R>` | size_of D + 1 B flag padded to D's alignment / size_of R (application statics) | 0 B (`create_boxed`: one block) | in display / indev |
| `StaticQueue<E = 8 B, N = 8>` | 80 B | handler | 300 to 500 B for queues |
| `subscribe_owned` queue | 0 B (the object's slot) | 16 B descriptor¹ + block(16 + size_of E · N) | in queues |
| `ConstStyle` with p properties | 0 B | 0 B | 12 B (+4 B with `LV_USE_ASSERT_STYLE`) + 8·(p+1) B rodata |
| Mutable `Style` | 12 B (16 B with `LV_USE_ASSERT_STYLE`) | block for 5·p B | 0 B |
| Generated pass-through wrapper | 0 B | 0 B | 0 B unless called; call site = raw call |
| Checked wrapper / `Handle::get` site | 0 B | 0 B | 20 to 40 B once / 8 to 12 B per site |

¹ The first descriptor on an object that has none also allocates the
event array (a 16 B block, growing by 4 entries) and, if the object has
no `spec_attr` yet, that block (60 B). Plain `on*` handlers pay the same
in raw LVGL; for owned setters, relations and owned queues it is a cost
of safety.

64-bit hosts (Rust sizes are covered by the `size_of` asserts; heap
figures are 64-bit TLSF blocks, `max(24, round8(n)) + 8`): slots are 16 B
and the header 24 B; the hook adds 8 B per object (block 88 → 96 B); an
event descriptor is a 32 B block (24 B struct); a timer a 64 B block
(40 B `lv_timer_t` + 16 B node + 8 B); `spec_attr` an 80 B block; a
stored `QueueRef`, `Display` or `Indev` is 16 B and an `EventSub` 24 B;
`StaticQueue<E = 8 B, N = 8>` is 96 B; a `ConstStyle` takes 16 B (+8 B
with `LV_USE_ASSERT_STYLE`) + 16·(p+1) B.

Worked examples:

- Small MCU without `heap` (60 objects, 2 owned screens + 6 handles at
  N = 16, 10 `on_with` handlers, 1 display, 1 indev, 6 const styles of 6
  properties): 152 B of crate `.bss` (128 B of slots, 24 B header);
  application-held values: `Display` and `Indev` 8 B each, the 2 `Owned`
  and 6 `Handle`s 32 B, and 1 B of `DisplayCell` flag next to the driver;
  240 B of LVGL heap added by safety (the hook; handlers cost the same as
  raw LVGL); styles 0 B RAM, 408 B rodata.
- Xila-like with `heap` and `async` (300 objects, 40 handles at N = 64,
  5 windows with owned queues E = 16 / N = 16, 30 boxed closures with 8 B
  captures, a keyboard bound to 6 textareas), heap in builtin TLSF blocks:
  540 B of crate `.bss` (512 B of slots, 24 B header, 4 B deferred-free
  head; the queues use the windows' slots and the keyboard one more of
  the 64); hook 1,200 B; queues 1,460 B, up to 1,540 B if each window
  allocates a fresh 16 B event array (replacing about 1.9 KB of
  `VecDeque`s); closures 960 B, up to 1,440 B with a fresh 16 B event array
  per handler; relation 96 B, plus the event array and `spec_attr` on
  textareas with no prior handler. Xila's `STDLIB_CUSTOM` setup shifts
  the heap figures by its allocator's rounding.

## 13. Xila migration (recommendations only)

1. `-sys` milestone M0 lands: nothing changes for Xila.
2. Xila's `lv_conf.h`: `LV_USE_OBJ_ID 1`, `LV_USE_OBJ_ID_BUILTIN 0`
   (`LV_OBJ_ID_AUTO_ASSIGN` is already 1). The crate then defines the five
   OBJ_ID functions, so Xila must not. Cost: +4 B per object.
3. Return a dedicated guard newtype from `lock()`, `synchronous_lock()` and
   `try_lock()`, and implement `LvglAccess` only on it (today they share the
   `MutexGuard` type used by `ALLOCATION_MUTEX`).
4. The Graphics task runs `Lvgl::enter(&mut guard)?.timer_handler()`; raw
   `lv_timer_handler` stays valid during the migration. Deferred requests
   and boxes on the deferred-free list are released on the Graphics task's
   next `enter(..).timer_handler()`; raw `lv_timer_handler` calls only
   delay that release, they do not leak it.
5. Replace `stubs.rs` allocator exports with `lvgl_allocator!`.
6. Windows become owned containers plus `subscribe_owned`, which removes the
   synchronous lock in `OwnedWindow::drop` and the unlocked
   `lv_obj_delete_async` in `Home`'s Drop. `Window` itself stays in Xila.
7. Display and indev move to `create_boxed`, fixing the leaked `UserData`.
8. Applications migrate module by module with `as_ptr()` and `from_raw`;
   raw callbacks calling safe code use `enter_callback()`.

## 14. Testing

1. Registry logic under Miri against a stub FFI layer: mint, dedup, FIFO,
   generation wrap, OWNED pin and DEAD state, TEMP slots, the pending load
   (last writer wins, cleared by the hook), the request/drain state
   machine, including `act` on an OWNED slot whose object the hook turns
   DEAD during the delete, a `Busy` refusal retried by the next drain, and
   `delete_later` (or `TimerCtx::delete_later`) while the `Owned` is alive:
   the drain deletes, the slot stays DEAD and OWNED, other slots are
   allocated and freed, then the `Owned` is dropped and only that slot is
   freed. Also under Miri:
   - `LvBox` layouts with a mock `lv_malloc`/`lv_free` returning pointers
     that are 4 mod 8, and closures and owned values holding a `u64` and an
     `f64`;
   - a re-entrant `capture` that sends an event to the same subscription
     and calls `try_recv` on the static receiver, and a test waker whose
     `wake`, `clone` and `drop` call `try_recv` and `register`.
2. loom: `Owned::drop` against the drain (no read-modify-write; SeqCst
   stores and the drain's SeqCst loads): no lost request, never the wrong
   object deleted, no slot pushed twice on the FIFO, including a drop that
   races a drain running a P_DELETE on the same owned slot while other
   slots are reused.
3. Host scenarios with LVGL built with `-fsanitize=address` (through
   `CFLAGS`), each holding the test lock:
   - every deletion path (delete, clean, parent delete, msgbox close via
     CLICKED, screen load with `auto_del` and with a pending `del_prev`,
     `delete_on_screen_unloaded`, menu sidebar, dropdown list, display
     delete);
   - deferred drains requested from callbacks, `delete_later` on an object
     without a slot and `RegistryFull`, `load_screen_later` replaced before
     the drain; an `Owned` dropped on another thread;
   - closures, owned-setter values and owned queue boxes freed exactly once
     (counted through `lv_mem_monitor`); switch and spinner animations
     deleted during their own `exec`;
   - `QueueRef`: `try_recv` returns `Closed` after the object's deletion;
     the `QueueRef` dropped before and after the deletion, and on another
     thread; `close()`;
   - views requested during deletion return `None`; structural refusals,
     including lazily added msgbox parts, deletable msgbox content children
     and the tabview's indexed refusals (clean of the bar and of a bar
     button, delete, set_parent and clean of a bar button's label, then
     `lv_tabview_set_tab_text` still valid); the keyboard relation, also with
     the holder deleted first; `FlushDone` deferral through
     `flush_wait_cb` and `try_delete` returning `Busy`; refused
     draw-buffer setters; queue CLOSED and OVERFLOW flags; param accessors
     returning `None` for other codes with `LV_USE_CHECK_ARG 0`;
     `lv_realloc(NULL, 16)` through `SizedHeader`; `set_text_str` with
     multibyte characters across a 31-byte chunk boundary and on a label
     holding static text, and with a mock allocator (or ASan build) that
     fails the allocation of the initial clear and of a later chunk: the
     call returns `OutOfMemory` and the label holds static `""`;
     `tick_inc` reaching `lv_tick_get` through the crate tick callback
     after `init`; a typed getter override checked
     against `lv_obj_has_class`; a deferred-free box released after the
     image cache drop with no other request pending;
   - aliasing after a wrap under the class check, and an alias whose stale
     descriptor address is reused by a relation descriptor on the aliased
     object, through `EventSub` and through `QueueRef` (test-only narrow
     generation width).
4. trybuild compile-fail tests: a view used after `timer_handler` or
   another exclusive call; a view escaping a callback; `ObjMut` alongside a
   view; a view passed next to `&mut Lvgl`; a closure with a non-zero-sized
   capture passed to `on` (a zero-sized capture is accepted and leaked),
   and a zero-sized closure capturing a `!Send` zero-sized value passed to
   `on` or `timer`;
   `on_with` with a `D` borrowing a local, and with a `u64` `D` (rejected
   on 64-bit hosts too); an `EventQueue` with N = 0 or 256; `on_boxed` with a closure
   borrowing a local `String`; an owned setter given a struct holding a
   borrowed slice; non-`'static` text passed to `set_text_static`; `Lvgl` or
   a view sent across threads; cloning a `QueueRef` or a `StaticRx`; a
   borrowed text getter used across a setter. Cases whose error is a
   post-monomorphization `const` assert build to codegen (11).
5. Generator: golden IR and coverage files for the vendored and Xila-like
   configs; negative tests for override validation; a test that fails if
   `gen.rs` contains a safe wrapper for a `crate_owned` function or if
   `access.toml` or an override makes one Shared or Exclusive; negative
   tests for a mutating method on a rodata receiver, an unknown `[type.*]`
   key, an out-of-range `LVGL_RUST_MAX_CALLBACK_DEPTH` or
   `LVGL_RUST_HANDLE_SLOTS`, and a missing `tables/lvgl-<major>.<minor>/`;
   golden IR identical on the Windows job; `audit --check` freshness; `universe.txt` completeness; a hierarchy test
   creating each widget and checking `lv_obj_has_class(obj, Base::class())`.
6. `lv_conf` matrix through nested builds (`tests/common::cargo()`):
   vendored with OBJ_ID (`lvgl-rust/conf`, 3.2); Xila-like (`OS_NONE`,
   `STDLIB_CUSTOM`, private API on, colour depth 16, shadcn); minimal (few
   widgets, no flex, grid or observer); asserts and argument checks off;
   views only (N = 0). These fixtures depend on `lvgl-rust`, which
   resolves `serde`, `toml`, `syn`, `quote` and `prettyplease` through
   `lvgl-rust-gen`. They are workspace members left out of
   `default-members` (so a plain `cargo test` does not build them) and
   share the root `Cargo.lock`, so the root `cargo fetch` covers them; a
   fixture that
   needs its own lockfile gets a matching
   `cargo fetch --manifest-path ...` step in the mise `fetch` task and CI,
   otherwise the offline nested build fails with "no matching package".
7. Cross builds: `thumbv6m-none-eabi`, `thumbv7em-none-eabihf`,
   `thumbv8m.main-none-eabihf`, `riscv32imc-unknown-none-elf`,
   `wasm32-unknown-unknown`, `wasm32-wasip1`, `wasm32-unknown-emscripten`,
   hosts on Linux, macOS and Windows, and an optional xtensa job. Each job
   names the C compiler and sysroot that both `cc` and bindgen use, through
   `CC_<target>`, `CFLAGS_<target>` and `BINDGEN_EXTRA_CLANG_ARGS_<target>`:
   - `thumbv*`: arm-none-eabi newlib, through `--sysroot` or `-isystem`;
   - `riscv32imc`: a riscv32 newlib toolchain (`riscv32-esp-elf` for ESP);
   - `wasm32-wasip1`: wasi-sdk (`CC_wasm32_wasip1`,
     `--sysroot=$WASI_SYSROOT`);
   - `wasm32-unknown-unknown`: no C library; the `-sys` freestanding
     `inttypes.h` (3.1);
   - `wasm32-unknown-emscripten`: `EMSDK`, as today.

   CI states that host builds prove nothing about xtensa. An MSRV job
   (3.4) runs `cargo check` with the minimum toolchain for a host,
   `wasm32-unknown-unknown` and `riscv32imc-unknown-none-elf`.
8. Size gates on `thumbv6m` and `riscv32imc` example firmware, built with
   default features at N = 16 and at N = 0. The gate sums the `nm -S`
   sizes of the crate's own symbols (a section total cannot isolate the
   crate) and compares them with the value for the feature set: 0 B of
   `.data`; `.bss` of 8·N + 24 for N > 0 and 8 B for N = 0, +4 with
   `heap`, +8 with `debug-counters`; `.text` budgets. `nm` also shows no
   `core::fmt`, `panic_bounds_check` or `unwrap_failed` from the crate.
   With N = 16 it shows the five hook symbols defined by the crate; with
   N = 0 it shows that the crate defines none of them (6.5). `size_of` asserts: `Slot` =
   2·usize, `Handle` = 4, `Option<Handle>` = 4, `Header` = 24,
   `Registry` = 24 + 8·N on 32-bit and 24 + 16·N on 64-bit.

## 15. Milestones

One pull request per milestone. A milestone PR may be split into a stack of
smaller PRs, each targeting the previous branch, when it is too large to
review at once. Each milestone ships the tests of what it adds and
re-measures the flash rows it delivers.

| Milestone | Content | Exit criterion |
|---|---|---|
| M0 | Workspace move and crate skeletons (3.3); `-sys` additive keys (`BINDINGS`, `LINKED`), shims, README; bindgen target fix and freestanding `inttypes.h` (3.1); `-sys` CI jobs for `riscv32imc-unknown-none-elf` (with the riscv32 newlib toolchain of 14.7) and `wasm32-unknown-unknown`; bindings for `xtensa-esp32s3-espidf` and `aarch64-apple-darwin` compared before and after the target fix, and the AGENTS.md cross-compilation pitfall updated (3.1); `rust-version = "1.89"` and the MSRV job (3.4) | `-sys` CI green, cross jobs included; Xila unaffected |
| M1 | Core: session, registry and views-only mode, tick counter and `tick_inc`, `enter_callback`/`DepthGuard`, OBJ_ID hook (five functions), requests and drain, `Obj` view, `ObjMut`, `Lvgl::edit`, `WidgetExt`, `Owned`, `HandleCell`, `Error`, structural checks for layers and screens, `delete_later`/`clean_later`/`load_screen_later`; `lvgl-rust/conf` and the docs.rs mode (3.2); arm-none-eabi newlib in CI for the `thumbv6m` size gate (14.7); the first nested matrix entries, vendored with OBJ_ID and N = 0, under the lockfile rule of 14.6 | Miri registry and drain cases (14.1) and loom; ASan for Obj and screen deletion paths, deferred drains, an `Owned` dropped on another thread and wrap aliasing; size gate with measured flash for the M1 rows |
| M2 | Generator phase 1 (dependencies `syn`, `quote`, `prettyplease`, `serde`, `toml`; M2 itself adds no nested fixture): table selection by LVGL version, parse, header scan, IR, rules (crate-owned, retained default, untyped object returns, rodata receivers), overrides with `[type.*]`/`[fn.*]` validation; `classes.toml` and `relations.toml` schemas and validation; a hand-written `access.toml` seed for Obj plus 5 widgets, replaced by the audit output in M3; Obj plus 5 widgets | Golden tests; crate-owned negative tests |
| M3 | Audit tool, `access.toml` and `events.toml` for 9.6, `bump`; all mechanical widgets, composite part tables and checks, style properties, `ConstStyle`, mutable `Style`, retained data (`set_text_static`, `ButtonMap`, builders, retained buffers), `set_text_str` and the itoa setters | `audit --check` in CI; ASan for msgbox, dropdown and menu deletion paths |
| M4 | Callbacks (`on_fn`, `on`, `on_with`, `on_static`, `on_removable`, `Event`, sending), timers, animations, observers and subjects, relations, composite components | trybuild suite (queue and heap cases aside); relation and callback-depth tests |
| M5 | Display, indev, groups, `DrawBuf`, `Flush`/`Read` | `FlushDone`, draw-buffer setter and display deletion tests |
| M6 | Async: `StaticQueue`, `next_event` | Queue flag tests; `StaticRx` trybuild case; Miri cases for a re-entrant `capture` and a re-entrant test waker (14.1) |
| M7 | `heap` (`LvBox` with its alignment rule, `on_boxed`, owned setters, `subscribe_owned`/`QueueRef`, `create_boxed`, `timer_boxed`), `allocator`, `fmt`, `defmt`, counters | Leak-count tests for closures, owned values and queues; `QueueRef` tests; Miri `LvBox` layout cases with `u64` and `f64` values (14.1); trybuild cases for `on_boxed` with a closure borrowing a local `String` and an owned setter given a borrowed slice (14.4) |
| M8 | Xila-like example in this repository and migration notes for Xila (Xila is not edited from here; a user-maintained Xila branch is out of scope) | Example running on host and `wasm32-wasip1` (wasi-sdk in CI) |
| M9 | Full conf matrix (remaining entries of 14.6, same lockfile rule) and cross matrix with the remaining targets of 14.7 (`thumbv7em` and `thumbv8m.main` on the M1 arm toolchain, emscripten, macOS and Windows hosts, the optional xtensa toolchain), docs, repository rename to `xila-project/lvgl-rust`, release (3.3) | Full CI |
| Later | Exclusive input callbacks, `CustomClass<T>`, DELETE-event backend, growable registry, safe custom drawing | — |

## 16. Decisions log

| Decision | Choice | Alternatives rejected |
|---|---|---|
| Threading | Token only (`unsafe trait LvglAccess`, guards ordered by happens-before), crate never locks | Pluggable locks in the crate; internal locking per call; single-thread `!Send` |
| Token shape | Concrete zero-sized `Session`/`Lvgl` from the trait; callbacks shared-only | Generic `&mut A` on every call (code bloat; callbacks with `&mut` are unsound) |
| Handle validity | Generational registry + scoped views | Lifetimes only (C-side deletions stay unsafe); per-object `Arc` tombstone (alloc, CAS) |
| Generation | 16-bit, wraps, FIFO reuse, class check; stored descriptors validated against the event list | 24-bit with retirement (registry exhaustion) |
| Delete hook | OBJ_ID: the crate defines the five functions, +4 B per object | DELETE-event descriptor (16 to 92 B per object, skippable, fires before destructors) |
| Borrows | Shared views + `&mut Lvgl` for structural calls; `Lvgl::edit` selects an object for exclusive access | Exclusive-only borrows (+4 to 6 KB flash, one widget at a time) |
| Fields the crate relies on | `crate_owned` list (Skip or Raw, wins over every table); timer, display, indev, anim and group receivers Manual by default | Audit-only classification (the audit sees frees, not overwritten fields) |
| Retained data | `&'static` core, owned setters keeping values on LVGL's heap with `heap` | `'static` only; lifetime-bound to owner; a free-standing `retain` returning `&T` |
| Heap alignment | `LV_HEAP_ALIGN` = `usize`, over-allocation for more aligned values, `SizedHeader` header = its alignment | Rejecting over-aligned values (breaks 32-bit builds of host code); 8/16 B alignment for every allocation |
| Callbacks | `fn` + context; closures with `heap`; `Fn` only; `'static` and padding-free `UserData` of at most 4 B on every target; closures `Send` (zero-sized ones too), zero-sized closures forgotten, called through a dangling `&F` | Closures always (mandatory alloc); `FnMut` (re-entry aliasing); pointer-sized `D` (host builds accept what MCU builds reject) |
| Tick source | Crate atomic counter in the registry header, installed with `lv_tick_set_cb` (+4 B) | `lv_tick_inc` from an ISR or thread (data race on `sys_time`) |
| Object getter returns | `&Obj` unless an override certifies the class | Views typed by the getter's name (type confusion with `LV_USE_CHECK_ARG 0`) |
| Enum representation | Fixed `u32` newtypes, cast at the FFI boundary | Newtypes over the bindgen alias (`c_int` on MSVC) |
| Refresh-time events | Not delivered to safe handlers in v1 | Shared access during rendering (frees memory queued draw tasks read) |
| Event sending | Allowlist of codes whose handlers tolerate a NULL param; typed `send_key` | Denylist of internal codes |
| Drop | `Owned` RAII, store-only request byte, drain at depth 0; `delete_later` always takes a slot | Lock-free MPSC queue (CAS, 72 B, overflow); explicit delete only; `lv_obj_delete_async` (no checks at execution time) |
| Composite parts | Per-class part lists checked through at most `max_depth` ancestors | Ancestor walk to the root; fixed child roles |
| Async | Token-guarded queues, executor-agnostic; `QueueRef` = handle + box, freed at DELETE | `AtomicWaker` channels (CAS); in-object channels read unlocked; a registry slot per queue |
| Flush completion | Atomic in `DisplayCore` + crate `flush_wait_cb` | Writing `disp->flushing` from the ISR |
| Heap | LVGL's heap, user allocator through `LvglAllocator` | Rust `alloc` (second pool, global allocator on every target) |
| Groups | In v1 with a slot (M5) | Raw-only |
| Timer slot lookup | `TimerCtx` scans the slots up to `hwm` for the timer pointer (≈40 B flash, 0 B RAM) | Slot index in timer `user_data` (drops the per-timer `D`) |
| Safe-crate test config | Second vendored config (`lvgl-rust/conf`) | OBJ_ID in the `-sys` vendored config (breaks `-sys` linking); `LVGL_CFLAGS` (overridden by `lv_conf.h`) |
| Exclusive input callbacks | After v1 | In v1 |
| Custom components | Composite in v1, `CustomClass<T>` later | Xila `Window` in this crate |
| Deinit | Terminal | Re-init with session epochs |
| `set_text_str` | Character-boundary chunks through `lv_label_ins_text` | One `lv_label_set_text_fmt("%.*s")` (the builtin `vsnprintf` reads past the precision) |
| Toolchain | Rust 1.89 minimum, `rust-version` in every crate | 1.81 (wrong C ABI on `wasm32-unknown-unknown`) |
| Naming | Repo `xila-project/lvgl-rust`; crates `lvgl-rust-sys`, `lvgl-rust`, `lvgl-rust-gen` | `lvglue`, `lvgl9`, `lvgl-oxide` |
