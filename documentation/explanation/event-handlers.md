# Event handlers

Event handlers are where an LVGL binding is most likely to be unsound,
because they run in the middle of LVGL's own work. This page explains
what a lvgl-rust handler may do, why it may do no more, and why the
default handler is a closure without captures.

## Handlers run inside the call that sent the event

LVGL calls handlers synchronously, from whatever function sent the event.
A click is delivered from inside `lv_timer_handler` (which reads the
input device), a `STYLE_CHANGED` from inside the style setter that
changed a property, a `SIZE_CHANGED` from inside a layout pass:

```text
your code                      Lvgl::timer_handler(&mut lv)
LVGL                             lv_timer_handler
                                   input device read timer
                                     lv_obj_send_event(button, CLICKED)
                                       ...event list traversal...
trampoline                               dispatch: tier check, depth + 1
your handler                               |e| { ... }      shared access
```

Every frame above the handler is LVGL code that may be holding pointers
into objects, children arrays, event lists or style data, and will use
them after the handler returns. If the handler frees any of that memory,
those frames continue on freed memory. This is why handlers get **shared
access only**: `Event::lv()` returns a `&Lvgl`, never a `&mut Lvgl`, so
the compiler rejects deleting, cleaning, reparenting or creating from a
handler. What needs exclusive access is deferred to the drain, where no
LVGL frame is on the stack: `obj.delete_later()`, `clean_later()`,
`lv.load_screen_later(..)`, and `lv.post(f, data)` for anything else,
creating objects included (see [Object lifetime](object-lifetime.md)).

Shared access is not automatically harmless either. The
[access audit](access-model.md#how-the-access-audit-decides) checks every
place where LVGL sends an event while holding a pointer into memory that
some *shared* function could free, and decides each case. The result is a
tier for every event code, in `events.toml`.

## Event tiers

| Tier | Codes (LVGL 9.6) | Handler gets | Registered with |
| --- | --- | --- | --- |
| shared | input, value, state, focus, scroll, child, screen, size, style and layout codes, `DELETE`, and custom codes from `lv_event_register_id` | an `Event` with shared access | `on`, `on_with`, `on_static`, `on_fn`, `on_removable`, `on_boxed` |
| restricted | `HIT_TEST`, `COVER_CHECK`, `REFR_EXT_DRAW_SIZE`, `GET_SELF_SIZE`, the `DRAW_*` codes, `DRAW_TASK_ADDED` | a `QueryEvent`: the typed parameter only | `on_query`, `on_query_with`, `on_query_fn` |
| none | `CREATE`, and display and refresh codes (`INVALIDATE_AREA`, `REFR_*`, `RENDER_*`, `FLUSH_*`, `VSYNC`, ...) | nothing | refused |

**Shared** codes are those whose send sites hold nothing a shared call
could free, or where LVGL checks at run time (an event list being
traversed, an animation list that changed) and the audit records the
check.

**Restricted** codes are sent in the middle of layout or rendering. A
queued label draw task keeps a pointer to the label's text
(`lv_label.c:936`), so a handler that
merely called `label.set_text(..)`, an ordinary shared setter, would
leave the renderer reading freed memory. Rather than making every such
setter exclusive everywhere, these codes get a handler with no access at
all. A `QueryEvent` exposes the event's parameter, typed per code, and
the answer the code expects (`e.self_size()` with `extend`, `hit_test()`
with `set_result`, `draw_task()` with `set_color`), and nothing else. The
handler is a zero-sized `Send + 'static` closure whose data is a 4-byte
value, so it cannot capture a view, and resolving a handle needs a
`&Lvgl` it does not have. The one remaining way in would be a
thread-local holding a `&'static Lvgl`, which needs a guard borrowed for
`'static`; the `LvglAccess` contract forbids that, and that rule exists
for this reason.

**None** covers `CREATE`, sent while the object is still being built and
before any handler can be registered, and display codes, which reach
display handlers rather than object ones.

`handler_allowed(code)` tests a bitmask generated from `events.toml`, so
the tier check in every trampoline is a shift and a mask. Registering a
handler for a code outside the shared tier returns `Error::InvalidArg`;
`EventCode::ALL` registers for every code but only shared ones are
delivered.

### Sending events

The same table decides which codes safe code may *send*, with a NULL
parameter, through two code types that have no public constructor:

- `SafeEventCode` (`Obj::send_event`, shared access): codes whose
  handlers cannot delete anything and that read no parameter;
- `SendableEventCode` (`ObjMut::send_event`, exclusive access): those plus
  codes whose handlers may delete objects. The call consumes the `ObjMut`
  and returns it only if the object survived.

Codes whose handlers read the parameter (`PRESSED` reads the input
device) cannot be sent at all, since safe code would pass NULL. `DELETE`
is never sendable: its handlers free per-object state on the assumption
that the object goes away.

## Re-entrancy

A handler can trigger other events, which run other handlers, which can
trigger more. The crate bounds and polices this:

- **Depth limit.** Each trampoline enters a callback level; at
  `LVGL_RUST_MAX_CALLBACK_DEPTH` (default 16, from 1 to 254) further
  handlers are skipped instead of run, which keeps stack use bounded on
  microcontrollers. Skips are counted as `skipped` with feature
  `debug-counters`. Raw C callbacks that call safe code enter a level
  with the unsafe `Lvgl::enter_callback()`.
- **No nested session.** `Lvgl::enter` fails with `Error::Reentrant`
  while a callback is running, as it would with a recursive lock.
- **No growing a list LVGL is traversing.** Adding a handler to an object
  LVGL is currently sending to would reallocate its event list under a
  copy of the list header that LVGL keeps for the traversal, and frees
  again if the object is deleted during the send (`lv_event.c:111, 145`).
  Inside callbacks, registering on such an object returns `Error::Busy`.
- **No views of dying objects.** LVGL sends `DELETE` to the children of
  an object it has already marked as deleting. Inside callbacks, views
  from getters and resolved handles are refused for objects under a
  deleting ancestor: `Event::target()` returns `None`, and
  `handle.get()` returns `Error::Dead`.

## Why closures without captures

`obj.on(code, |e| ...)` takes a closure whose type is zero-sized: no
captures, or only zero-sized ones. The compiler checks this with a
`const` assertion, and a capturing closure fails to build. The reasons:

- **No allocation.** LVGL's event descriptor has one pointer of user
  data. A zero-sized closure needs none of it: the trampoline is
  monomorphised for the closure's type and conjures the value from a
  dangling pointer, so nothing is stored or allocated. Registering costs
  what a raw `lv_obj_add_event_cb` costs.
- **No destructor to run.** LVGL frees event descriptors with the object
  and calls no Rust code when it does. A zero-sized closure has nothing to
  drop, so nothing leaks and no drop can run at the wrong moment.
- **Same behaviour on every target.** State that must travel with the
  handler goes through `on_with(code, data, |e, data| ...)`, where `data`
  is a `UserData` value: `Copy`, padding-free and at most 4 bytes (an
  integer, a `bool`, a `Handle`), stored by value in the user data
  pointer. The 4-byte limit holds on 64-bit hosts too, so a program that
  builds on the host builds for a 32-bit microcontroller. Larger state
  lives in a `static` given to `on_static`, or in objects reached through
  handles.

The trampolines differ only in how they recover the closure and its data;
the shared part (the tier check, the depth level) is a single non-generic
function, so each handler adds little code. In the benchmark, an event
reaches an `on` handler 2.6 ns later than a raw `extern "C"` callback
(20.7 ns against 18.1 ns), and an `on_with` handler 1.6 ns later
([measurements](../reference/overhead.md)).

### Boxed closures, with feature `heap`

Some state does not fit 4 bytes and does not belong in a `static`. With
feature `heap`, `obj.on_boxed(code, move |e| ...)` accepts any
`Send + 'static` closure and stores it in one block of LVGL's own heap,
after an 8-byte header on 32-bit targets. There is still no Rust global
allocator involved: the block comes from whatever allocator LVGL is
configured with.

The difficulty is freeing it. The closure is dropped when its object
receives `DELETE`, but LVGL may still call the handler after that (a
`DELETE` handler can send to an ancestor that trickles events back down,
until LVGL removes the object's handlers). So the header counts the
frames running the closure and marks it doomed at `DELETE`; the
outermost frame drops the closure, and the block itself goes on the
deferred-free list and is freed at the next drain, when no LVGL frame can
reach it. The benchmark puts a boxed handler at 2.7 ns over a raw
callback.

### Handling events outside the frame

When a handler would need exclusive access for most of what it does, it
is simpler not to do the work in the handler. With feature `async`,
`obj.subscribe(code, &QUEUE)` records events into a `StaticQueue`, and
your task receives them with `next_event(lock, &rx).await`, together
with the guard: the work then runs in a session, with exclusive access
and no LVGL frame below it. See
[Receive events asynchronously](../how-to-guides/receive-events-asynchronously.md).

Related: [Access model](access-model.md),
[Handle events](../how-to-guides/handle-events.md),
[Use the heap](../how-to-guides/use-the-heap.md).
