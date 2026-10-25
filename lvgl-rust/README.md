# lvgl-rust

Safe, `no_std` Rust bindings to [LVGL](https://lvgl.io) 9, on top of
[`lvgl_rust_sys`](../lvgl-rust-sys). Work in progress: the core (sessions,
views, handles, ownership, deferred deletion) is in place, and wrappers are
generated at build time from your configuration's bindings and the reviewed
tables of [`lvgl-rust-gen`](../lvgl-rust-gen). They cover the base object and
a first set of widgets (button, label, bar, slider, switch) so far; anything
else stays reachable through `lvgl_rust::sys`.

- **No locking, no allocator.** Prove exclusive access with your own lock by
  implementing `LvglAccess` for its guard, then work through a `Session`.
- **No dangling objects.** Views (`&Obj`) borrow the session; deleting needs
  exclusive access. `Handle`s (4 bytes) report deleted objects as
  `Error::Dead`, whatever deleted them. `Owned` deletes its object when
  dropped, from any thread.
- **Styles without RAM.** `const_style![...]` builds a `ConstStyle` in
  `.rodata`; a `Style` is a 12-byte static changed through
  `lv.style(&STYLE)`, which refreshes the objects using it. Styles and fonts
  are always `'static`, since LVGL keeps pointers to them.
- **Constant data stays in flash.** Images (`ImageDsc` over aligned
  `ImageData`, checked against their format and size), button maps
  (`button_map![...]`), gradients and transitions are `const` and
  `'static`, as LVGL keeps pointers to them.
- **Event handlers without allocation.** `obj.on(code, |e| ...)` (a closure
  without captures), `on_with(code, data, ...)` (a value of up to 4 bytes),
  `on_static` (a `static` context), `on_removable`. Handlers get shared
  access; `delete_later` and `lv.post(f, data)` defer what needs exclusive
  access to the next drain. Codes sent in the middle of layout or
  rendering are not offered.
- **Timers.** `lv.timer(period_ms, data, f)` returns an `Owned<Timer>`
  (dropping it deletes the timer); `Handle<Timer>` pauses, resumes and
  changes the period and repeat count. No allocation besides LVGL's timer.
- **Animations.** `Anim::new(data).values(0, 255).duration(300).exec(f)`,
  started with `ObjMut::animate`; LVGL copies it and deletes it with the
  object.
- **Subjects.** `static VOLUME: IntSubject = IntSubject::new(50);` (also
  `ColorSubject`), bound with `slider.bind_value(&VOLUME)`, `bind_checked`,
  `label.bind_text`, observed with `observe`/`observe_obj`. Subjects
  initialise themselves on first use, so LVGL never sees one of the wrong
  type.
- **Small.** The handle registry is a static array of 8-byte slots on 32-bit
  targets (`LVGL_RUST_HANDLE_SLOTS`, default 32), plus a 24-byte header.
  `LVGL_RUST_HANDLE_SLOTS=0` keeps only views (8 bytes of state).

## Configuration

Handles rely on LVGL's OBJ_ID hooks, which the crate defines. Add to your
`lv_conf.h`:

```c
#define LV_USE_OBJ_ID 1
#define LV_OBJ_ID_AUTO_ASSIGN 1
#define LV_USE_OBJ_ID_BUILTIN 0
```

`conf/lv_conf.h` is the vendored configuration used by this crate's tests.
With `LVGL_RUST_HANDLE_SLOTS=0` the crate defines no hook: use a
configuration where LVGL provides them (`LV_USE_OBJ_ID_BUILTIN 1`), where
OBJ_ID is off, or where your application defines them.

## Example

```rust,ignore
use lvgl_rust::{
    color, const_style, init, ConstStyle, Font, LvglAccess, Obj, ScreenAnim, Selector,
    WidgetExt,
};

static PANEL: ConstStyle = const_style![
    radius(8),
    bg_color(color::hex(0x202020)),
    text_font(Font::montserrat_14()),
];

struct Guard(/* your mutex guard */);
unsafe impl LvglAccess for Guard {}

let mut guard = Guard(/* lock */);
let mut lv = init(&mut guard)?;
let screen = lv.create_screen()?;              // Owned<Obj>
let panel = Obj::create(screen.get(&lv)?)?;    // &Obj, borrows `lv`
let panel = panel.handle()?;                   // Handle<Obj>, storable
panel.get_mut(&mut lv)?.add_style(&PANEL, Selector::DEFAULT);
lv.load_screen(screen.handle(), ScreenAnim::NONE)?;
loop {
    let wait = lv.timer_handler();
    // ...
}
```
