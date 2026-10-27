# Migrate from raw bindings

This guide moves an application written against `lvgl-rust-sys` to
lvgl-rust one module at a time. lvgl-rust builds on `lvgl-rust-sys` and
does not change it, so raw and safe code live side by side during the
move: the raw API stays available as `lvgl_rust::sys`.

## What you need

- An application that calls LVGL through `lvgl-rust-sys` under a lock.
- Its `lv_conf.h`, which you will change in step 1.

## Steps

### 1. Turn on the OBJ_ID hooks

In `lv_conf.h`:

```c
#define LV_USE_OBJ_ID 1
#define LV_OBJ_ID_AUTO_ASSIGN 1
#define LV_USE_OBJ_ID_BUILTIN 0
```

lvgl-rust defines the five OBJ_ID functions (`lv_obj_assign_id`,
`lv_obj_free_id`, `lv_obj_set_id`, `lv_obj_id_compare`,
`lv_obj_stringify_id`): remove any definition of them from your
application. Each object grows by 4 bytes on 32-bit targets. If you
cannot change the configuration yet, build with
`LVGL_RUST_HANDLE_SLOTS=0` (views only, see
[Configure LVGL](configure-lvgl.md)).

### 2. Depend on lvgl-rust and reach the raw API through it

```toml
[dependencies]
lvgl-rust = { git = "https://github.com/AlixANNERAUD/lvgl_rust", features = ["color_depth_16"] }
```

Replace `use lvgl_rust_sys::*` with `use lvgl_rust::sys::*` (or keep a
direct `lvgl-rust-sys` dependency from the same repository with the same
features): there must be one LVGL in the program.

### 3. Give the lock a guard type of its own

Before:

```rust
static LVGL: Mutex<()> = Mutex::new(());

fn with_lvgl<R>(f: impl FnOnce() -> R) -> R {
    let _guard = LVGL.lock().unwrap();
    f()
}
```

After: the lock returns a dedicated guard, and only that type implements
`LvglAccess`:

```rust
use lvgl_rust::LvglAccess;
use std::sync::{Mutex, MutexGuard};

static LVGL: Mutex<()> = Mutex::new(());

pub struct Guard(MutexGuard<'static, ()>);

pub fn lock() -> Guard {
    Guard(LVGL.lock().unwrap())
}

// SAFETY: a guard of the only mutex that serializes LVGL calls.
unsafe impl LvglAccess for Guard {}
```

Never implement it on a `MutexGuard` shared with other mutexes. Details
in [Share LVGL between threads](share-lvgl-between-threads.md).

### 4. Initialise and drive LVGL through a session

Before:

```rust
unsafe {
    lv_init();
}
loop {
    let wait = with_lvgl(|| unsafe { lv_timer_handler() });
    sleep_ms(wait.min(10));
}
```

After:

```rust
use lvgl_rust::Lvgl;

{
    let mut guard = lock();
    let _lv = lvgl_rust::init(&mut guard)?;
}
loop {
    let mut guard = lock();
    let wait = Lvgl::enter(&mut guard)?.timer_handler();
    drop(guard);
    sleep_ms(wait.min(10));
}
```

`init` replaces `lv_init`. Raw `lv_timer_handler` stays valid while you
migrate; deferred deletions and posted work then run at the next
`Lvgl::enter` or `timer_handler`.

### 5. Move a module: from raw pointers to views

Before:

```rust
unsafe fn build_status(parent: *mut lv_obj_t) -> *mut lv_obj_t {
    let label = lv_label_create(parent);
    lv_label_set_text(label, c"Ready".as_ptr());
    lv_obj_center(label);
    label
}
```

After:

```rust
use lvgl_rust::{Error, Handle, Lvgl, WidgetExt, widgets::Label};

fn build_status(lv: &mut Lvgl) -> Result<Handle<Label>, Error> {
    let mut screen = lv.screen_active_mut().ok_or(Error::Dead)?;
    let label = Label::create(&mut screen)?; // a view, borrowing `screen`
    label.set_text(Some(c"Ready"));
    let label = label.handle()?;             // 4 bytes, storable
    label.get_mut(lv)?.center();             // geometry needs exclusive access
    Ok(label)
}
```

At the boundary between migrated and raw code:

- **Safe to raw:** `view.as_ptr()` (also `ObjMut::view().as_ptr()`)
  gives the `*mut lv_obj_t`. It stays valid until the next exclusive call,
  which may delete the object. `Display::as_ptr` and `Indev::as_ptr` give
  the `lv_display_t` and `lv_indev_t`.
- **Raw to safe:** `unsafe { Obj::from_raw(&lv, ptr) }` takes a
  `NonNull<lv_obj_t>` of a live object that is not being deleted and
  returns a `&Obj`; `obj.downcast::<Label>()` checks the class. Keep a
  handle (`obj.handle()?`) rather than the pointer.
- **Raw code that holds the lock without a session:**
  `unsafe { Lvgl::assume() }` returns `&mut Lvgl`. The caller must hold
  exclusive access for the returned lifetime and must not overlap it with
  a session or another `assume`.

### 6. Make raw C callbacks call safe code

A raw callback registered with `lv_obj_add_event_cb` runs inside LVGL.
Before calling safe code, it enters a callback level:

```rust
use core::ptr::NonNull;
use lvgl_rust::{Lvgl, Obj, sys};

unsafe extern "C" fn on_close(e: *mut sys::lv_event_t) {
    // None past LVGL_RUST_MAX_CALLBACK_DEPTH.
    let Some(depth) = (unsafe { Lvgl::enter_callback() }) else { return };
    let lv = depth.lv(); // shared access, as a Rust handler gets
    if let Some(target) = NonNull::new(unsafe { sys::lv_event_get_target_obj(e) }) {
        let object = unsafe { Obj::from_raw(lv, target) };
        let _ = object.delete_later();
    }
} // dropping `depth` leaves the level
```

`enter_callback` is `unsafe`: call it only from an event, timer or
animation callback that LVGL runs inside a call of the session holder,
never from a query or draw code handler (`GET_SELF_SIZE`, `DRAW_*`, ...).
New handlers are simpler written with `obj.on(...)` and friends
([Handle events](handle-events.md)).

### 7. Replace unsafe deletions

- Closing a window from its own button: `delete_later` replaces
  synchronous deletion under a re-entrant lock and unlocked
  `lv_obj_delete_async` calls in `Drop` implementations.
- An object that must die with a Rust value: keep an `Owned<W>`
  (`create_owned`, `own()`); dropping it on any thread deletes the object
  at the next drain.
- Windows and dialogs become components: a root handle and part handles,
  with the `Component` trait.

### 8. Replace hand-written allocator exports

Before, the application defined `lv_malloc_core`, `lv_realloc_core`,
`lv_free_core` and friends itself. After, with
`LV_USE_STDLIB_MALLOC LV_STDLIB_CUSTOM` and the `allocator` feature:

```rust
static LVGL_HEAP: lvgl_rust::SizedHeader<MyAllocator> = lvgl_rust::SizedHeader(MyAllocator);
lvgl_rust::lvgl_allocator!(LVGL_HEAP);
```

See [Use the heap](use-the-heap.md).

### 9. Move the drivers

Replace `lv_display_create` with `lv_display_set_flush_cb` and
`lv_display_set_buffers` by `Display::create` with a `Flush` driver and
`DrawBuf` statics, and `lv_indev_create` with a read callback by
`Indev::create` with a `Read` driver (or `create_boxed`, feature `heap`).
See [Write a display driver](write-a-display-driver.md) and
[Write an input driver](write-an-input-driver.md).

### 10. Find what stays raw

The build writes `coverage.txt` next to the generated wrappers, in
`target/<profile>/build/lvgl-rust-<hash>/out/`. One line per bound
function says what became of it:

```text
lv_label_set_text: View set_text
lv_obj_set_x: ObjMut { consumes: false } set_x
lv_obj_add_event_cb: raw (parameter `event_cb`: unsupported type lv_event_cb_t)
lv_obj_delete_async: raw (schedules a deletion: lv_obj_delete_async -> ...)
lv_obj_create: manual (wrapped by hand in the core)
```

`View` is a method on views, `ObjMut` one on exclusive access, `Create`
a widget's `create`, `LvglShared` and `LvglExclusive` methods on `Lvgl`,
`Style` one on `StyleMut`, `Free` a free function; `raw` functions stay
in `sys` with the reason; `manual` ones are wrapped by hand (or kept by
the crate); `skip` ones get nothing (deprecated, variadic, defined by the
crate, or not in the access table). Keep raw calls under the lock
until a wrapper exists.

## Pitfalls

- **Raw calls can invalidate views.** A raw call that deletes, cleans or
  moves objects while a view exists breaks the guarantees safe code relies
  on. Make such calls only with exclusive access (an `ObjMut` or `&mut
  Lvgl` in scope, no view alive), and never from a Rust handler.
- **Cross-object pointers are yours.** A keyboard's textarea or a widget
  part set up through `sys` points at objects that safe code may delete.
- **Do not call `lv_init` or `lv_deinit` directly** once `init` is used:
  `Lvgl::deinit(session)` is the terminal call.
- **Raw `lv_obj_delete` on a handled object is detected**: its handles
  report `Error::Dead` afterwards, whatever deleted it.

## See also

- [Access model](../explanation/access-model.md)
- [Generated bindings](../explanation/generated-bindings.md)
- [API overview](../reference/api-overview.md)
