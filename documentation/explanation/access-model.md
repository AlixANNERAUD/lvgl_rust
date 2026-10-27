# The access model

LVGL is a single-threaded C library that keeps raw pointers everywhere:
parents to children, widgets to their parts, groups to focused objects,
the refresh code to the screens it draws. Almost any call can run event
handlers, and almost any handler can delete or move objects. A safe Rust
API therefore has to answer two questions for every call: who may make
it, and which references may still exist when it returns. lvgl-rust
answers the first with a token and the second with the borrow checker.

## Tokens instead of locks

The crate never locks. It does not know how your program serialises
access to LVGL, and it does not try to: you prove it, once, by
implementing the unsafe trait `LvglAccess` on the guard of the lock you
already use. `init(&mut guard)` and `Lvgl::enter(&mut guard)` then return
a `Session<'g>` that borrows that guard, and every LVGL call goes through
the session.

```text
  your lock ──► guard: impl LvglAccess
                  │  &mut borrow for 'g
                  ▼
              Session<'g>  ── Deref ──►  Lvgl   (zero-sized, !Send, !Sync)
                                          │
                       &Lvgl ─────────────┴──────────── &mut Lvgl
                    shared access                    exclusive access
```

`Session` and `Lvgl` are zero-sized, so all of this costs nothing at run
time beyond the few atomic loads and stores of `Lvgl::enter` (see
[Overhead](overhead.md)). The reasons for not locking inside the crate:

- **Portability.** `thumbv6m-none-eabi` and `riscv32imc-unknown-none-elf`
  have no compare-and-swap, so a portable spin lock is not available. The
  registry uses only atomic loads and stores (`base/sync.rs`), which every
  target has.
- **Your lock is already the right one.** An RTOS mutex, a critical
  section, a `std::sync::Mutex` or an async lock each fit some program and
  not others. The asynchronous API (`next_event(lock, &rx)`, feature
  `async`) takes whatever async lock function you pass. A second lock
  inside the crate would add cost and a lock-ordering hazard and prove
  nothing your own lock does not.
- **No hidden blocking.** Entering a session either succeeds or returns
  an error (`Error::Reentrant`, `Error::NotInitialized`); it never waits.
  Waiting, if any, happens in your lock, where you can see it.

The trait's safety contract is what makes the token meaningful. The type
must prove exclusive, non-recursive access (a guard dedicated to LVGL,
never a guard type shared with unrelated mutexes); the end of one holder
must synchronise-with the start of the next, as mutexes provide, so that
the crate's state and LVGL's C globals are ordered between threads; and a
guard must never be borrowed for `'static` (a leaked guard would give a
`&'static Lvgl` that could reach a restricted handler, see
[Event handlers](event-handlers.md)).

Recursion is detected rather than deadlocked: `Lvgl::enter` fails with
`Error::Reentrant` while a session is active or a callback is running.
A few calls need no token at all: `tick_inc` only updates an atomic, so
a timer interrupt can call it; dropping an `Owned` only records a
request (see [Object lifetime](object-lifetime.md)); and the few
generated functions the tables mark `pure` touch no LVGL state, so they
are free functions.

## Shared and exclusive access

The session splits into two kinds of access, mirroring `&` and `&mut`.

| | Shared: `&Lvgl` | Exclusive: `&mut Lvgl`, `ObjMut<'_, W>` |
| --- | --- | --- |
| Gives | views (`&Obj`, `&Label`, ...) | one object's `ObjMut`, or the whole `Lvgl` |
| Allows | getters, most setters, adding handlers, `delete_later`, `post` | everything shared allows, plus deleting, cleaning, reparenting, reordering, creating |
| Available in | sessions, event handlers, timer callbacks | sessions, posted functions |
| Size | views are one pointer | `ObjMut` is one pointer |

A view is a reference to a zero-sized type (`Obj` wraps an empty
`UnsafeCell`) placed at the object's address. It is a plain pointer, and
a call through it is the FFI call and nothing else: the benchmark
measures `obj.set_ext_click_area(v)` and the raw
`lv_obj_set_ext_click_area(p, v)` at the same time (1.7 ns, see
[the measurements](../reference/overhead.md)). Its lifetime is the
borrow of the `Lvgl` it came from, so it cannot outlive the session, and
it cannot survive any exclusive call: the borrow checker rejects a
program that uses a view after `timer_handler`, `delete` or `clean`.

The invariant this buys is simple: **while a view exists, nothing that
can free its object can run.** Every operation that can free an object,
shrink a children array or free memory objects point to takes
`&mut Lvgl` or an `ObjMut`, and an `ObjMut` borrows `&mut Lvgl`, so no
view coexists with it.

### What needs exclusive access, and why

- **Deleting and cleaning** free objects directly.
- **Reparenting and reordering** (`set_parent`, `move_to_index`) shrink
  one children array and grow another; LVGL code iterating over children
  would read past the end.
- **Creating an object.** This looks harmless, but it is not. Creating an
  object initialises its class, which adds it to the default group; the
  group refocuses and sends focus events; a dropdown's focus handler opens
  its list and moves it to another parent (`lv_dropdown_open` calls
  `lv_obj_set_parent`). The access audit finds this path and marks
  `lv_obj_create` exclusive. So `Obj::create` and every generated
  `create` take the parent's `ObjMut`.
- **Any generated function the audit finds can reach one of these
  effects**, which includes surprising ones such as `lv_obj_add_flag`
  (hiding an object moves focus to the next one, which can reach the same
  dropdown handler) and `lv_obj_set_x` (a style refresh sends events, and
  a chain through the image, tab view and observer handlers ends at the
  same dropdown handler).

A newly created object is lent as a **view** borrowing the parent's
`ObjMut`, never as an `ObjMut` of its own. If the child had its own
`ObjMut` while the parent's was in scope, an exclusive call on the child
(sending a deleting event, for instance) could delete the parent through
a handler while the parent's `ObjMut` still claims it is live. Exclusive
work on the child goes through its `Handle` (`handle.get_mut(&mut lv)`)
or `Lvgl::edit` once the parent's borrow ends.

The same reasoning shapes the `ObjMut` methods. Calls that may delete the
receiver consume it: `delete(self)`, `load_screen(self, ..)`, and
`send_event(self, code)`, which returns the `ObjMut` only if the object
survived its handlers. `into_base` takes `self` too, since a reborrowed
`ObjMut` of a base class could be consumed by such a call while the
original stays in scope. In the generator, an exclusive function that the
audit finds can reach a *deleting* sink takes `self` for the same reason.

Callbacks only ever get shared access, because they run inside an LVGL
call that is itself holding pointers. What they need exclusive access for
is deferred: `Obj::delete_later`, `clean_later` and
`Lvgl::load_screen_later` are recorded and carried out at the next drain;
`Lvgl::post(f, data)` runs `f` with `&mut Lvgl` there (see
[Object lifetime](object-lifetime.md)).

## How the access audit decides

Writing "shared" or "exclusive" by hand for about two thousand LVGL
functions would be both tedious and wrong, since the answer depends on
what every handler LVGL installs can do. The access audit
(`mise run audit`, in `lvgl-rust-gen`) computes it from LVGL's own code:

1. It compiles the vendored LVGL, with `lvgl-rust/conf/lv_conf.h`, to
   LLVM IR with a pinned clang and a fixed `x86_64-unknown-none-elf` target, so the result does not depend
   on the host.
2. It builds a call graph, including indirect calls. Callbacks are
   tracked per struct field, so a call through a class's event callback
   reaches only the functions LVGL stores in that field; event sends with
   a constant code reach only the handlers of that code; widget code
   reaches only the handlers registered by core code, by itself or by
   related widgets.
3. A function that can reach a **sink** (deleting an object or a display,
   removing a child from its parent, removing objects from a group or
   deleting a group) is exclusive. The path is kept as the `why` of its
   entry in `access.toml`, for example
   `lv_obj_add_flag -> lv_obj_set_hidden -> lv_group_focus_next -> ... ->
   lv_dropdown_open -> lv_obj_set_parent -> lv_obj_remove_child`.
4. A function that schedules a deletion for later stays raw. Everything
   else is shared, unless the held-region analysis below says otherwise.

### Held regions

Reaching a sink is not the only hazard. A handler runs with shared access
inside the frame that sent its event, and that frame may keep a pointer
into memory a *shared* function can free, then use it after the send.
The held-region analysis looks for exactly this: a local of some LVGL
function that received a pointer into a region (an object of a given
type, a buffer at a given field path, or a struct copy), read after a
call site that can send events. Each such hazard is resolved in one of
three ways:

1. LVGL or the crate already checks it at run time (an entry under
   `[guarded]` in `audit.toml` names the check: LVGL's own
   `anim_list_changed` or `timer_deleted` flags, or the crate refusing to
   grow the handler list of an object LVGL is sending to);
2. the event codes involved are query or draw codes, sent in the middle of
   layout or rendering: those codes go to restricted handlers that have no
   access at all, and the freeing function stays shared;
3. otherwise the freeing function becomes exclusive, with the hold
   described in its `why`.

The second case is how a setter that frees memory can stay a shared view
method: `lv_label_set_text` frees the label's old text, yet it is shared.
A queued label draw task keeps a pointer to that text
(`lv_label.c:936`) while LVGL sends draw codes, and no shared handler
runs for draw codes.

The audit writes `access.toml` and `events.toml`, and CI checks they are
up to date (`mise run audit-check`). Nobody edits them by hand; changes
go through `audit.toml`, each with a comment citing the LVGL source
lines that justify it. The analysis has known limits (holds are computed
per local rather than per path, stale indices are not tracked); where its
precision stops, it reports instead of assuming safety.

## Why a method can move between a view and `ObjMut`

A generated function is a view method if the audit finds it shared and an
`ObjMut` method if it finds it exclusive, never both (from an `ObjMut`,
shared methods are reached through `.view()`). The classification
follows LVGL's code, not the function's name, so it can change when LVGL
does: a new handler in some widget that moves objects makes every
function that can reach it exclusive, and a refactoring that removes such
a path makes them shared again.

This is deliberate. When you update LVGL, a function that became
exclusive stops compiling on a view, and the compiler points at every
place where your program relied on an assumption that no longer holds. A
hand-maintained list would keep compiling and silently become unsound.

## Alternatives considered

- **Wrappers that hold raw pointers.** Wrapping each object in a Rust
  value that holds its pointer is the simplest design, but nothing stops
  LVGL, or a handler, from deleting the object while the value lives, so
  safe code could use freed memory.
  lvgl-rust's views cannot outlive the access that keeps their object
  alive, and its handles check liveness on every use.
- **A lock inside the crate.** Rejected for the portability and
  composition reasons above.
- **Run-time borrow flags per object.** They would cost memory per
  object and a check per call, and still could not stop a handler from
  deleting an object that some frame is holding. The type-level split
  costs nothing per call and covers handlers.
- **Everything exclusive.** Sound, but every getter would need
  `&mut Lvgl`, and handlers could do nothing. The audit makes the shared
  set as large as LVGL's code allows.

Related: [Object lifetime](object-lifetime.md),
[Event handlers](event-handlers.md),
[Generated bindings](generated-bindings.md),
[Share LVGL between threads](../how-to-guides/share-lvgl-between-threads.md).
