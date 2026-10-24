# lvgl-rust

Safe, `no_std` Rust bindings to [LVGL](https://lvgl.io) 9, on top of
[`lvgl_rust_sys`](../lvgl-rust-sys). Work in progress: the core (sessions,
views, handles, ownership, deferred deletion) is in place; widget wrappers
will be generated from the bindings.

- **No locking, no allocator.** Prove exclusive access with your own lock by
  implementing `LvglAccess` for its guard, then work through a `Session`.
- **No dangling objects.** Views (`&Obj`) borrow the session; deleting needs
  exclusive access. `Handle`s (4 bytes) report deleted objects as
  `Error::Dead`, whatever deleted them. `Owned` deletes its object when
  dropped, from any thread.
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
With `LVGL_RUST_HANDLE_SLOTS=0`, no hook is defined and any configuration
works.

## Example

```rust,ignore
use lvgl_rust::{init, LvglAccess, Obj, ScreenAnim, WidgetExt};

struct Guard(/* your mutex guard */);
unsafe impl LvglAccess for Guard {}

let mut guard = Guard(/* lock */);
let mut lv = init(&mut guard)?;
let screen = lv.create_screen()?;              // Owned<Obj>
let panel = Obj::create(screen.get(&lv)?)?;    // &Obj, borrows `lv`
let panel = panel.handle()?;                   // Handle<Obj>, storable
lv.load_screen(screen.handle(), ScreenAnim::NONE)?;
loop {
    let wait = lv.timer_handler();
    // ...
}
```
