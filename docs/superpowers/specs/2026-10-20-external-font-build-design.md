# External LVGL Font Build Design

## Goal

Allow generated LVGL font assets to be compiled and linked independently from
`lvgl_rust_sys`, so updating a language-specific glyph subset does not rerun
LVGL C compilation or bindgen. Font descriptors and glyph tables remain
compile-time `const` data in firmware Flash/ROM; this design does not load,
copy, or rasterize fonts at runtime.

## Proposed architecture

The default `lvgl_rust_sys` build compiles LVGL, its shims, and enabled theme or
driver sources, but does not discover font directories or include font headers
in bindgen. It exports the native build interface needed by dependent crates:
LVGL and vendor include paths, the selected `lv_conf.h` directory, required
defines, and relevant extra compiler flags. Cargo `links` metadata and
`DEP_<LINKS>_*` build-script environment variables provide this interface;
the exact key names will be stable and documented.

A separate fonts crate owns generated `.c` assets and its own build script. It
consumes the exported configuration, compiles those assets into a distinct
static archive, and provides Rust declarations or a small C/Rust shim for the
font symbols. Applications link both crates and select among their
`lv_font_t` pointers at runtime. Font input changes therefore rerun only the
font crate build and relink the application, not `lvgl_rust_sys` or bindgen.

The exported configuration must account for the actual target and enabled
features, including target-specific compiler configuration and options that
affect LVGL ABI. It must not assume desktop filesystem or toolchain behavior.

## Compatibility and documentation

`LVGL_FONTS_DIR` remains supported as an explicit legacy, in-crate font mode
for existing users. It compiles and binds fonts as before, with clear
documentation that this mode couples font changes to the LVGL package build.
Implicit discovery of a working-directory `fonts/` directory is removed so
that unrelated application assets do not silently alter this crate's build.
Documentation will describe the external-crate workflow, generated font
declarations, runtime pointer selection, and migration from the legacy mode.

## Validation

- Build an example with generated-font-shaped C in a separate package and
  verify successful linking against `lvgl_rust_sys`.
- Measure Cargo rebuild behavior: changing only a font source rebuilds the
  font archive and final binary, without invoking LVGL C compilation or
  bindgen; LVGL, configuration, and feature changes still rebuild LVGL.
- Exercise host/native builds and available embedded/cross targets, including
  ESP32-S3 where the toolchain is available, to verify compatible headers,
  defines, and flags.
- For an available embedded ELF, inspect map/section output to confirm font
  descriptors and glyph tables reside in read-only/Flash sections and are not
  duplicated into persistent RAM.
- Verify multiple font profiles can link together and be selected using their
  existing `lv_font_t` pointers.

Unavailable embedded tooling or hardware will be reported explicitly rather
than represented as validated.

## Out of scope

Runtime font file loading, filesystem access during redraw, binary-font
loaders, FreeType, dynamic rasterization, CJK shaping, and UI language/style
refresh behavior are not part of this change.
