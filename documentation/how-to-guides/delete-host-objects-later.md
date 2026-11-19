# Delete host objects later

Goal: let the owner type of an object of your own LVGL class (a window, an
application logo) delete the object when it is dropped, from any thread,
without taking the LVGL lock, and without the handle registry.

An owner's `Drop` often runs while another task holds the LVGL lock, for
instance suspended at an `.await` on the same single-threaded executor.
Waiting for the lock there deadlocks. `Owned<W>` solves this with the
registry; the `deferred-delete` feature does it for host classes, in
views-only mode too: the drop queues the object, and the next drain
([`Lvgl::enter`, `Lvgl::timer_handler`](../reference/api-overview.md#deferred-work))
deletes it.

## Steps

1. Enable the `deferred-delete` feature of `lvgl-rust`, and link a
   [`critical-section`](https://docs.rs/critical-section) implementation
   in the final binary: the `std` feature of `critical-section` on a host,
   your HAL's or OS's on a target. The queues use it instead of atomics,
   so they work on targets without compare-and-swap (`thumbv6m`,
   `riscv32imc`).
2. Add a `DeferredLink` to the instance struct of your class, and give
   the class its size:

   ```rust
   #[repr(C)]
   struct Window {
       obj: lv_obj_t,
       link: DeferredLink,
   }
   // In the class descriptor: instance_size = size_of::<Window>().
   ```

   LVGL zeroes every new instance, which is the link's "not queued"
   value. Nothing else may write the link.
3. Declare one `DeferredClass` per class, in a `static`:

   ```rust
   // SAFETY: only `Window`s are given to WINDOWS, and their `link` is used
   // through WINDOWS only.
   static WINDOWS: DeferredClass = unsafe { DeferredClass::new(offset_of!(Window, link)) };
   ```
4. In the class's destructor (`destructor_cb`), call
   `WINDOWS.forget(obj)`. LVGL runs it on every deletion path (a
   deletion, a parent's, `clean`, a screen deleted by a load,
   `lv_deinit`), so an object that dies before the drain leaves its queue
   and the queue never keeps a freed object.
5. In the owner's `Drop`, call `WINDOWS.request_delete(obj)`. It needs no
   session, allocates nothing and calls no LVGL function. A second request
   for the same object does nothing.
6. When the owner already has exclusive access (`&mut Lvgl`), it can
   delete at once with `WINDOWS.delete_now(&mut lv, obj)`, which makes the
   same checks as the drain.

`lvgl-rust/tests/deferred_delete.rs` is a complete host class, with an
owner dropped without a session and on another thread.

## What the drain does

The drain deletes the queued objects first, before the crate's own
requests. It takes each object out of its queue inside the critical
section, then calls `lv_obj_delete` outside it, so the destructors (yours
and LVGL's OBJ_ID free hook, which the free observer and a host id table
rely on) run as for any deletion. A screen being loaded is queued again
for the next drain; a display layer is refused (counted in `refused` with
`debug-counters`). Objects queued while the drain runs wait for the next
one.

## Pitfalls

- `request_delete` needs a live object: its memory must not be freed
  before the call returns. If the LVGL side can delete the object while
  its owner lives on another thread (a parent deleted with it), let the
  destructor tell the owner inside `critical_section::with`, and check
  that flag in the owner's `Drop` inside its own `critical_section::with`
  before requesting (critical sections nest). A request that arrives
  while the object is being deleted, before its memory is freed, is
  ignored.
- Use the same `DeferredClass` for every call about an object. A subclass
  of your class keeps the link at the same offset, so its objects can use
  the base's `DeferredClass`.
- Not from an interrupt handler, unless your critical-section
  implementation masks interrupts.
- After `Lvgl::deinit` starts, requests and `forget` touch nothing: LVGL
  frees every object, and owners dropped later hold freed pointers.

## Costs

One pointer per object of a participating class (the link), four per
class (the `DeferredClass`, 16 B on 32-bit targets), and a chain head of
a pointer and a flag in the crate's `.bss` (5 B on 32-bit targets once
LLVM splits it, 8 B otherwise). Code: about 420 to 500 B on Cortex-M
(`thumbv6m`, `thumbv7em`) and 580 to 660 B on `riscv32imc`, measured by
the size gate ([Cargo features](../reference/features.md)).

Queues are first in, first out: a request appends at the end. Inside the
critical section, a drain counts each queue once, and removing an object
from the middle of a queue walks it, so keep queues short when the
critical section masks interrupts.
