# Generated bindings

Most of lvgl-rust's API is not written by hand. Its build script runs the
generator, `lvgl-rust-gen`, on the bindgen output of the same build of
`lvgl-rust-sys`, and writes `gen.rs` (the `widgets`, `enums`, `functions`
and `props` modules) and `coverage.txt` into `OUT_DIR`. This page explains
why, what the generator needs besides the bindings, and what it
deliberately leaves raw.

## Why generate, and why per configuration

LVGL is configured at compile time. `lv_conf.h` decides which widgets
exist, which functions are compiled, the layout of some structs and the
color format. A hand-written binding has to pick one configuration, or
guard every item with features that mirror `LV_USE_*` and drift out of
step with them. Generating from the bindings of your own build removes
the question: `lvgl-rust-sys` publishes the path of its `bindings.rs`
(`DEP_LVGL_RUST_SYS_BINDINGS`), the generator reads exactly the functions
your LVGL was compiled with, and a widget disabled in your `lv_conf.h` is
simply absent from the safe API. Hand-written code that depends on a
widget uses `lv_use_<x>` cfgs, generated from the same values.

LVGL has about two thousand public functions, and they change between
minor releases. Generation also keeps that surface reviewable: what
changes in a release shows up as a diff of tables, not as thousands of
lines of edited wrappers.

## What the bindings do not say

A C prototype says nothing about the questions that decide soundness:

- Can this call delete or move objects, directly or through a handler?
- Does LVGL copy this pointer argument, or keep it and read it later?
- Is this index bounds-checked, or used to index an array blindly?
- Does this returned string belong to the object, and when is it freed?

The answers live in reviewed tables, one directory per LVGL minor
version (`lvgl-rust-gen/tables/lvgl-9.6/`). The generator refuses
bindings of a version that has no tables.

| Table | Says | Written by |
| --- | --- | --- |
| `access.toml` | each function's access: `shared` (view method), `exclusive` (`ObjMut` or `&mut Lvgl` method), `pure` or `isr` (free function, no token) or `raw` (stays in `sys`) | the access audit, never by hand |
| `events.toml` | each event code's handler tier and whether safe code may send it | the access audit, never by hand |
| `classes.toml` | the widget hierarchy and names; functions wrapped by hand (`manual`) or reserved to the crate (`crate_owned`); the composite widget rules of the structural checks | hand, from the LVGL sources |
| `relations.toml` | functions that make one object keep a pointer to another (a keyboard's textarea) | hand |
| `overrides/**/*.toml` | per-parameter kinds and certified returns, each with a `why` citing the LVGL lines | hand |
| `universe.txt` | every function of the full configuration, to tell a typo from a function your `lv_conf.h` disabled | `mise run bump` |

How `access.toml` and `events.toml` are computed is explained in
[the access model](access-model.md#how-the-access-audit-decides) and
[event handlers](event-handlers.md#event-tiers).

## Raw by default

The generator wraps nothing it cannot justify:

- A function is wrapped only if `access.toml` lists it.
- A pointer parameter stays raw until an override gives its kind:
  `copied_str` and `copied` (LVGL copies it during the call), `static_str`
  (LVGL keeps it, so it must be `'static`), `out` (LVGL only writes it),
  `retained` with a `rust` type (LVGL keeps it and reads it later, mapped
  to a checked Rust type, below). The other kinds (`slice`,
  `retained_mut`, a plain `retained`) still leave the function raw.
  `@nullable` in LVGL's header comment gives an `Option`.
- An index parameter stays raw until an override marks it `checked`,
  citing the lines that bounds-check it. A check under `LV_USE_CHECK_ARG`
  or `LV_ASSERT` does not count, because configurations can turn those
  off.
- An object returned by a function is an untyped `&Obj` unless an
  override certifies its class.

Anything not wrapped stays available in `lvgl_rust::sys`, and
`coverage.txt` records what happened to every bound function and why:

```text
lv_anim_count_running: LvglShared anim_count_running
lv_anim_delete: raw (parameter `var`: pointer kept by LVGL (retained))
lv_anim_get_timer: raw (return: pointer return)
lv_anim_init: manual (crate-managed resource)
```

Each line is the function, then its fate: a view, `ObjMut`, `Lvgl`,
style, creation or free-function wrapper with its Rust name, `manual`
(wrapped by hand in the crate), `raw` with the parameter or return that
kept it raw, or `skip` (no table entry). Being raw is not a judgement
that a function is dangerous, only that nobody has yet shown it is safe.
Migrating code can mix both: `as_ptr()` on a view gives the pointer for
a raw call, and `Obj::from_raw` goes the other way.

## Data LVGL keeps

Many LVGL setters store a pointer and read it later: image sources,
button maps, gradients, transitions, color filters, color keys, subjects.
The C API accepts a pointer to a local array, which the next frame
overwrites. In the generated API these parameters take types of
lvgl-rust that are `'static` and whose layout is checked when they are
built: `ImageSrc` (an `ImageDsc` over aligned `ImageData`, checked
against its format and size), `ButtonMap` (terminated, built with
`button_map![...]`), `GradDsc`, `TransitionDsc`, `ColorFilter`,
`ImageColorkey`, `IntSubject` and `ColorSubject`. The constant ones
(images, maps, gradients, transitions, filters) are built in `const`
contexts, so they sit in flash and cost no RAM; subjects are `static`
values that initialise themselves on first use.

A few setters need more than a type and are wrapped by hand: keyboard
layouts (`KeyboardMap`), scale labels (`ScaleLabels`, `'static` and
NULL-terminated, since LVGL walks the array up to its terminator) and
grid templates
(`GridTemplate`, set together so that LVGL's unchecked subgrid path is
unreachable, with bounded cell spans). The raw template style setters
stay raw.

Styles and fonts are typed by the generator directly: a `const
lv_style_t *` parameter takes a `'static` `Style` or `ConstStyle`, a
`const lv_font_t *` a `&'static Font`, and `lv_style_set_<p>` becomes a
`StyleMut` method, exclusive whatever the audit says, because objects may
be reading the style.

## Strings

Strings show the per-parameter decisions most clearly:

| LVGL behaviour | Rust parameter or return | Example |
| --- | --- | --- |
| copies the string during the call | `&CStr` (`copied_str`) | `Label::set_text(&self, text: Option<&CStr>)` |
| keeps the pointer | `&'static CStr` (`static_str`) | `Checkbox::set_text_static(&self, txt: &'static CStr)` |
| returns a pointer into the object | `x_into(&self, buf)` on the view, `x(&mut self)` on `ObjMut` (`borrowed_internal`) | `Label::get_text_into(&self, buf)`, `ObjMut<Label>::get_text(&mut self)` |

A getter returning the object's own text cannot hand out a `&CStr` from a
view: another shared setter could free the text while the reference
lives. So the view method copies it into your buffer (`Error::TooSmall`
if it does not fit), and the `ObjMut` method lends it for as long as the
exclusive borrow lasts, when nothing can change the object. An override
marks a getter `borrowed_internal` only after checking that the pointer
really is the object's own text, a literal, or static text it was given.
For labels, hand-written setters also accept a `&str` and integers
without `core::fmt`.

## Errors at build time

Table errors (an unknown function, a parameter that no longer exists, an
override that contradicts `access.toml`) fail the build. An entry for a
function missing from your bindings is a warning, since your `lv_conf.h`
may disable it, unless `universe.txt` does not know the function either;
`LVGL_RUST_GEN_STRICT=1`, set in every development task, turns these
warnings into errors.

## Bumping LVGL

A new LVGL minor version gets a new table directory. The process, in
outline:

1. Add `tables/lvgl-<major>.<minor>/`, usually as a copy of the
   previous version's tables.
2. `mise run bump` rewrites `universe.txt` from the new bindings, lists
   hierarchy lines that disagree with the `.base_class` fields of the
   LVGL sources, and lists stale entries (functions or parameters that no
   longer exist). These are fixed by hand.
3. `mise run audit` recomputes `access.toml` and `events.toml` from the
   new sources. Their diffs are reviewed: a function that became
   exclusive moves from the view to `ObjMut`, and a function that became
   shared moves the other way.
4. Overrides and composite rules whose cited lines changed are read
   again against the new sources.

The compiler then reports every place where application code relied on
a decision that changed. That is the intended outcome: an LVGL update
that adds a path from a setter to a deletion becomes a compile error,
not a use-after-free in the field.

Related: [Access model](access-model.md),
[API overview](../reference/api-overview.md),
[Migrate from raw bindings](../how-to-guides/migrate-from-raw-bindings.md),
[Build configuration](../reference/build-configuration.md).
