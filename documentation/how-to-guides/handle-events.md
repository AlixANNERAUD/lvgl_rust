# Handle events

This guide registers Rust handlers for LVGL events, shows what a handler
may do, and how it hands exclusive work (creating or deleting objects) to
the next drain.

## What you need

- A view of the object (`&Obj`, `&Button`, ...), from a creation, a handle
  (`handle.get(&lv)?`) or an `ObjMut` (`obj.view()`).
- The event codes: `lvgl_rust::enums::EventCode` (`CLICKED`,
  `VALUE_CHANGED`, `PRESSED`, `KEY`, `DELETE`, ...).

## Steps

### 1. Pick a registration method

All of them take `&self` on a view and store nothing on the heap unless
the name says `boxed`:

| Method | Handler | State |
| --- | --- | --- |
| `obj.on(code, \|e\| ...)` | closure without captures | none |
| `obj.on_fn(code, f)` | `fn(&Event<'_>)` | none |
| `obj.on_with(code, data, \|e, data\| ...)` | closure without captures | `data`: a `UserData` value of at most 4 bytes (`u8` to `u32`, `i8` to `i32`, `f32`, `bool`, `char`, `()`, `Handle<W>`, `Option<Handle<W>>`), stored in LVGL's event descriptor |
| `obj.on_static(code, &HANDLER)` | `Handler { ctx, f }` in a `static` | `ctx: C` where `C: Sync` |
| `obj.on_removable(code, data, \|e, data\| ...)` | as `on_with` | returns an `EventSub` |
| `obj.on_boxed(code, move \|e\| ...)` | any `Fn` closure (feature `heap`) | captures on LVGL's heap |

```rust
use lvgl_rust::{Event, WidgetExt, enums::EventCode, widgets::{Button, Label}};

// Each view borrows `parent`: take the label's handle before creating
// the button, whose view stays in use below.
let count = Label::create(&mut parent)?.handle()?;
let button = Button::create(&mut parent)?;

// Pass a handle as the handler's data: 4 bytes, no allocation.
button.on_with(EventCode::CLICKED, count, |e: &Event<'_>, count| {
    // `Error::Dead` if the label was deleted: dropped, like every error.
    count.get(e.lv())?.set_text_str("clicked")
})?;
```

A handler returns `()` or `Result<(), Error>`; errors are dropped, since
LVGL cannot report them. `EventCode::ALL` registers for every code a
handler may receive. Each of these handlers costs LVGL's event descriptor,
as a raw callback does; the
[overhead reference](../reference/overhead.md) measures the extra time per
event at a few nanoseconds.

The `Event` gives `lv()` (shared access), `code()`, `target()` and
`current_target()` (views, `None` while the object is being deleted),
`target_is(handle)`, `param_obj()` for `CHILD_CHANGED` and
`CHILD_CREATED`, `key()` for `KEY`, `point()` for pointer events,
`stop_bubbling()` and `stop_processing()`.

### 2. Do shared work directly in the handler

A handler runs inside the LVGL call that sent the event, with shared
access: `e.lv()` gives `&Lvgl`. Everything a view can do is allowed:
getters, view setters (`set_text_str`, `set_ext_click_area`, ...),
resolving handles with `get`, starting timers with `e.lv().timer(...)`.

What needs `&mut Lvgl` or an `ObjMut` (creating, deleting, moving or
cleaning objects, geometry setters such as `set_size`, setting subjects)
does not compile in a handler. Defer it with one of the next steps.

### 3. Delete objects or load screens from a handler

```rust
use lvgl_rust::{ScreenAnim, enums::EventCode};

// A close button deleting its window (`window: Handle<Obj>`).
close.on_with(EventCode::CLICKED, window, |e, window| {
    window.get(e.lv())?.delete_later()
})?;

// Switch screens at the next drain (`settings: Handle<Obj>`).
open.on_with(EventCode::CLICKED, settings, |e, settings| {
    e.lv().load_screen_later(settings, ScreenAnim::NONE)
})?;
```

- `delete_later` and `clean_later` record the request; the next drain (the
  next `Lvgl::enter`, or the end of `timer_handler`) carries it out. They
  take a registry slot until then if the object has none. A request the
  structural checks refuse for good (a part of a widget, a display layer)
  fails at once with `Error::Internal`.
- `load_screen_later(screen, anim)` replaces any pending load; a screen
  deleted meanwhile is skipped.

### 4. Post exclusive work to the next drain

`e.lv().post(f, data)` runs `f(&mut Lvgl, data)` at the next drain, with
exclusive access. `f` is a function item or a closure without captures,
and `data` a `UserData` value:

```rust
use lvgl_rust::{Lvgl, Obj, enums::EventCode};

fn open_window(lv: &mut Lvgl, number: u32) {
    if let Some(mut screen) = lv.screen_active_mut() {
        if let Ok(window) = Obj::create(&mut screen) {
            let _ = number; // build the window
            let _ = window;
        }
    }
}

launcher.on(EventCode::CLICKED, |e| {
    let _ = e.lv().post(open_window, 1u32);
})?;
```

`post` fails with `Error::Busy` when the queue is full
(`LVGL_RUST_POST_SLOTS`, default 4 entries of 8 bytes each on 32-bit
targets). Work posted
while posted functions run waits for the following drain.

### 5. Remove a handler later

```rust
let sub = slider.on_removable(EventCode::VALUE_CHANGED, (), |e, ()| {
    // ...
})?;
// Later, with exclusive access:
sub.remove(&mut lv);
```

`on_removable` takes a registry slot for the object. `remove` does
nothing if the object was deleted (its handlers went with it).

### 6. Send events

| Call | Codes | Access |
| --- | --- | --- |
| `obj.send_event(code)` | `SafeEventCode` (`READY`, `CANCEL`, `CHECKED`, `SCROLL`, ...): no handler LVGL installs deletes or frees anything for them | shared, `&self` |
| `obj_mut.send_event(code)` | `SendableEventCode`: `SafeEventCode`'s codes plus those whose handlers may delete objects (`CLICKED`, `VALUE_CHANGED`, `DEFOCUSED`, `SIZE_CHANGED`, ...) | consumes the `ObjMut`, returns it if the object survived, `Error::Dead` otherwise |

```rust
use lvgl_rust::enums::{SafeEventCode, SendableEventCode};

// Shared access is enough:
label.send_event(SafeEventCode::READY)?;

// CLICKED may delete objects (a message box's close button):
let button = handle.get_mut(&mut lv)?;
let button = button.send_event(SendableEventCode::CLICKED)?;
```

Both send a NULL parameter, so codes whose handlers read a parameter
(`PRESSED`, `KEY`, ...) are in neither type and stay raw
(`lv_obj_send_event` through `lvgl_rust::sys`). Neither is `DELETE`: its
handlers free per-object state on the assumption that the object goes
away. Neither type has a public
constructor: the access audit decides which codes they hold.

### 7. Answer query and draw codes with restricted handlers

Codes sent in the middle of layout or rendering (`GET_SELF_SIZE`,
`HIT_TEST`, `COVER_CHECK`, `REFR_EXT_DRAW_SIZE`, `DRAW_MAIN`, `DRAW_POST`
and their `_BEGIN` and `_END` variants, `DRAW_TASK_ADDED`) are refused by
`on` and friends with `Error::InvalidArg`. Register them with `on_query`,
`on_query_with` or `on_query_fn`, which take a `QueryCode`:

```rust
use lvgl_rust::{QueryEvent, enums::QueryCode};

// Content of 120 × 40.
obj.on_query(QueryCode::GET_SELF_SIZE, |e: &QueryEvent<'_>| {
    if let Some(size) = e.self_size() {
        size.extend(120, 40);
    }
})?;

// Clickable only in its left half (`half_width` given as data).
obj.on_query_with(QueryCode::HIT_TEST, 60i32, |e: &QueryEvent<'_>, half_width| {
    if let Some(hit) = e.hit_test() {
        hit.set_result(hit.point().x < half_width);
    }
})?;
```

A `QueryEvent` has no `Lvgl` access and no views: it reads the code's
typed parameter (`self_size`, `hit_test`, `cover_check`, `ext_draw_size`,
`layer`, `draw_task`) and writes the answer. `draw_task()` can change a
task's color or opacity (`set_color`, `set_opacity`); drawing into a layer
stays raw (`Layer::as_ptr`).

### 8. Keep a capturing closure (optional, feature `heap`)

```rust
// 8 bytes of state: too large for `on_with`.
let limit: u64 = 1_000_000;
button.on_boxed(EventCode::CLICKED, move |e| {
    let _ = (limit, e.code());
})?;
```

The closure and its captures move into one block of LVGL's heap (8-byte
header on 32-bit targets, plus the closure), dropped when the object is
deleted and freed at the next drain. See [Use the heap](use-the-heap.md).

## Pitfalls

- **`on` rejects captures at compile time.** A closure with captures
  given to `on`, `on_with`, `on_removable`, `timer` or `post` fails to
  build, with a message naming the alternative (`on` says "use on_with or
  on_static for state"). Pass a value with `on_with`, a `static` with
  `on_static`, or use `on_boxed`.
- **Some codes have no handlers.** `CREATE` (sent before a handler can be
  registered), display and input-device codes are refused with
  `Error::InvalidArg`. `handler_allowed(code)` and
  `query_handler_allowed(code)` tell which tier accepts a code.
- **Registering on an object LVGL is sending to fails.** Inside a handler,
  adding a handler to an object whose event list LVGL is walking returns
  `Error::Busy`. Post the registration instead.
- **`Lvgl::enter` fails in handlers** with `Error::Reentrant`: use
  `e.lv()`. Do not take your LVGL lock there either.
- **Objects under deletion.** Views and handles of objects whose ancestor
  is being deleted are refused (`target()` returns `None`,
  `handle.get` returns `Error::Dead`).
- **Deep recursion is cut.** Past `LVGL_RUST_MAX_CALLBACK_DEPTH` nested
  callbacks (default 16), handlers are skipped (counted by
  `DebugCounters::read().skipped` with `debug-counters`).

## See also

- [Event handlers](../explanation/event-handlers.md)
- [Access model](../explanation/access-model.md)
- [Receive events asynchronously](receive-events-asynchronously.md)
