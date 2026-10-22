# lv_rust_sys

The bundled LVGL version is 9.6.0. Initialize the Git submodules recursively when
building from a checkout.

Enable `shadcn-theme` to compile the pinned
[lvgl_theme_shadcn](https://github.com/AlixANNERAUD/lvgl_theme_shadcn) C theme and
generate its Rust FFI bindings alongside LVGL. The feature uses the same LVGL
headers, color depth, custom allocator and `DEP_LV_CONFIG_PATH` configuration;
it does not build a second LVGL library or add a Rust runtime dependency.

The feature is disabled by default. Initialize it with
`lv_theme_shadcn_init`, update the palette with `lv_theme_shadcn_set_palette`,
and use its surface/variant helpers instead of local color overrides. Theme
state is released when its display is deleted, or explicitly by
`lv_theme_shadcn_deinit`. All calls follow LVGL's synchronization requirements.

# Build features

By default, this crate generates Rust bindings and compiles the vendored LVGL
C sources into a native library. To generate bindings without compiling the
LVGL library, disable default features (or otherwise omit `build-lvgl`):

```toml
lvgl_rust_sys = { version = "0.1", default-features = false }
```

The `build-lvgl` feature is enabled by default and controls only compilation of
the native LVGL library; bindgen still runs when it is disabled. Configure
`DEP_LV_CONFIG_PATH` as usual so bindgen can use the intended `lv_conf.h`.

## Native build metadata

Downstream build scripts can use the same LVGL headers and configuration as
this crate through Cargo's `links = "lvgl_rust_sys"` metadata:

- `DEP_LVGL_RUST_SYS_INCLUDE_PATHS`: semicolon-separated include directories
- `DEP_LVGL_RUST_SYS_DEFINES`: semicolon-separated `NAME` or `NAME=VALUE`
  preprocessor definitions
- `DEP_LVGL_RUST_SYS_CFLAGS`: configured `LVGL_CFLAGS` entries in their existing
  comma-separated `NAME` or `NAME=VALUE` define syntax
- `DEP_LVGL_RUST_SYS_TARGET`: Cargo target triple used for the native build

These values are derived from the LVGL build configuration, including the
selected `lv_conf.h`, color-depth feature, and optional native include paths.
Use the target-aware compiler supplied by `cc` in the dependent build script;
do not substitute the host target or independently guess ABI-affecting defines.

## Font build modes

The recommended mode is to build generated `lv_font_conv --format lvgl` C
sources in a separate dependent crate/archive. Font changes then rebuild that
archive and relink the firmware without rerunning the LVGL build script or
bindgen. The native metadata above supplies the headers and definitions needed
for an ABI-compatible compile.

For compatibility, setting `LVGL_FONTS_DIR` explicitly keeps the legacy
in-crate behavior: C sources from that directory are compiled into the LVGL
archive and its headers are included by bindgen. Updates in this mode rerun
`lvgl_rust_sys`'s build and bindgen. Implicit discovery of a working-directory
`fonts/` directory is no longer performed. Move existing assets into an
external font crate for incremental build isolation; no runtime font loading
or RAM copy is needed.

### External font crate example

`examples/external-fonts` is a separate Cargo package. Its build script consumes
the `DEP_LVGL_RUST_SYS_*` metadata—including both standard and custom defines—
and compiles all generated C files under its `fonts/` directory into a separate
`font_assets` archive. Run it with the same
LVGL configuration used by the application:

```sh
DEP_LV_CONFIG_PATH=/path/to/lvgl-config \
  cargo run --manifest-path examples/external-fonts/Cargo.toml -- --cjk
```

Replace the small generated-font-shaped C fixtures there with outputs from
`lv_font_conv --format lvgl`. Give each profile a unique `--lv-font-name` (for
example, `font_montserrat_14_latin` and `font_montserrat_14_cjk`) so profiles
can coexist in one firmware. The Rust crate exports each descriptor as an
existing `*const lv_font_t`; application code can select and pass the pointer
to LVGL when changing languages without rebuilding LVGL.

Generated descriptors and glyph/map/kerning tables are compile-time `const`
data and should remain in the target's read-only Flash/ROM sections. The
host-side integration test checks the profile descriptors and tables in
read-only sections; position-independent host executables may place
pointer-bearing const descriptors in `.data.rel.ro` (read-only after relocation).
For embedded firmware, inspect the target ELF/map with its linker script to
confirm the final placement. The font crate neither loads files during redraw
nor copies glyph data into persistent RAM. LVGL's temporary drawing buffers
are separate runtime scratch space, not duplicate font assets.
