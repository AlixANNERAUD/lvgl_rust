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
