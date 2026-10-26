# lvgl-rust

Safe, `no_std` Rust bindings to [LVGL](https://lvgl.io) 9, on top of
[`lvgl-rust-sys`](../lvgl-rust-sys). Work in progress: the core (sessions,
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
  dropped, from any thread. A generated function the access audit finds
  shared is a view method only (from an `ObjMut`, call it through
  `.view()`), an exclusive one an `ObjMut` method only: when a new audit
  moves a function from exclusive to shared, its method moves too.
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
- **Drivers in statics.** `Display::create(lv, &CELL, driver, w, h,
  DrawBuffers::double(&BUF_A, &BUF_B), RenderMode::Partial)` with a `Flush`
  driver, `Indev::create(lv, &CELL, reader, IndevKind::Pointer)` with a
  `Read` driver: `DisplayCell`, `IndevCell` and `DrawBuf` (aligned to
  `LV_DRAW_BUF_ALIGN`) are application statics, claimed while in use.
  Flushes can complete from an interrupt (`FlushRequest::defer`), without
  touching LVGL's own flag. `set_resolution`, `set_rotation`,
  `set_color_format` (`DisplayFormat`) and `set_render_mode` check that
  the draw buffers hold what LVGL will render (a row in partial mode, the
  screen in full and direct modes) and return `Error::TooSmall`
  otherwise. Button devices (`Indev::create_button(lv, &CELL, reader,
  &POINTS)`) report a button id (`IndevData::set_button`) and press
  `POINTS[id]`, a `'static` array; an id past it reads as released.
- **Focus groups.** `lv.create_group()` returns an `Owned<Group>`;
  `Handle<Group>` adds objects and moves focus, `Indev::set_group` attaches
  a keypad or encoder.
- **Async (feature `async`).** `obj.subscribe(code, &QUEUE)` records
  events into a `StaticQueue` through a `capture` function;
  `next_event(lock, &rx).await` takes your async lock, returns the next
  event with the guard, or waits. No executor dependency, no atomics.
- **Closures (feature `heap`).** `obj.on_boxed(code, move |e| ...)` and
  `lv.timer_boxed(period, move |ctx| ...)` keep the closure on LVGL's own
  heap (a 4-byte header), freed with the object or the timer.
  `Display::create_boxed` and `Indev::create_boxed` (or
  `create_button_boxed`) put the driver there instead of a static cell,
  freed when the display or device is deleted (never while a flush is
  pending).
- **Your allocator (feature `allocator`).** With
  `LV_USE_STDLIB_MALLOC LV_STDLIB_CUSTOM`, `lvgl_allocator!(HEAP)` makes
  LVGL allocate from a static implementing `LvglAllocator`;
  `SizedHeader(GlobalAlloc)` adapts a Rust allocator.
- **Formatting (feature `fmt`).** `label.set_text_fmt(format_args!(...))`
  streams into the label in 32-byte chunks; `Error` implements `Display`.
  Without it, `set_text_i32`/`u32` avoid `core::fmt`.
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

The desktop example (`mise run example`, `examples/desktop.rs`) puts most
of the API together: drivers in statics, a clock bound to a subject, a
launcher posting work, window components with a close button, a
textarea and a keyboard, styles, a timer and an animation.

## Migrating from raw `lvgl-rust-sys` code

The crate builds on `lvgl-rust-sys` and does not change it, so raw and
safe code can live side by side while modules move over:

1. In `lv_conf.h`, set `LV_USE_OBJ_ID 1` and `LV_USE_OBJ_ID_BUILTIN 0`
   (`LV_OBJ_ID_AUTO_ASSIGN` stays 1): the crate defines the five OBJ_ID
   functions, so the application must not. Cost: 4 bytes per object.
2. Make the lock return a guard type of its own and implement `LvglAccess`
   only on it (not on a `MutexGuard` shared with other mutexes).
3. The graphics task runs `Lvgl::enter(&mut guard)?.timer_handler()`.
   Raw `lv_timer_handler` stays valid meanwhile; deferred deletions and
   posted work then run at the next `enter` or `timer_handler`.
4. Replace hand-written allocator exports with `lvgl_allocator!` (feature
   `allocator`, `LV_STDLIB_CUSTOM`).
5. Windows and dialogs become components (a root handle and part
   handles); closing
   from a handler is `delete_later`, which replaces synchronous locks and
   unlocked `lv_obj_delete_async` in drops.
6. Displays and input devices move to `Display::create` and
   `Indev::create` with drivers in statics, or to `create_boxed` (feature
   `heap`) with the driver on LVGL's heap, freed with the display.
7. Modules migrate one by one with `as_ptr()` and `Obj::from_raw`; raw C
   callbacks that call safe code take `Lvgl::enter_callback()` first. What
   stays raw is listed in `$OUT_DIR/coverage.txt`.

