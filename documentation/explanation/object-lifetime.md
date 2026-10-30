# Object lifetime

In LVGL an object lives until something deletes it: your code, its
parent's deletion, a screen loaded with `auto_delete`, a message box's
close button, `lv_deinit`. None of these tell the Rust side. lvgl-rust
gives three ways to refer to an object, each with a different answer to
"what if it was deleted?", and a small amount of static bookkeeping that
LVGL itself keeps up to date.

## Three kinds of reference

| | View (`&Obj`, `&Label`) | `Handle<W>` | `Owned<W>` |
| --- | --- | --- | --- |
| Size on 32-bit targets | 4 B (a pointer) | 4 B | 4 B |
| Lives | while its `&Lvgl` borrow lasts | as long as you keep it | as long as you keep it |
| `Copy`, `Send` | `Copy`, not `Send` | both | neither `Copy` nor `Clone`; `Send` |
| If the object is deleted | cannot happen while it exists | `get` returns `Error::Dead` | `get` returns `Error::Dead`; dropping it does nothing more |
| Deletes the object | no | no | when dropped, from any thread |
| Costs | nothing | one registry slot | one registry slot |

`Option<Handle<W>>` and `Option<Owned<W>>` are 4 bytes too, through the
niche of the `NonZeroU32` inside. The sizes are checked by the overhead
benchmark (see [the measurements](../reference/overhead.md)).

**Views** are covered by the [access model](access-model.md): they borrow
shared access, and nothing that can free an object runs while shared
access is borrowed. A view needs no bookkeeping at all, which is why a
build with `LVGL_RUST_HANDLE_SLOTS=0` (views only) works, with an 8-byte
registry header in place of the whole registry.

**Handles** are for everything a view cannot do: keeping a reference in a
struct, in a `static` (`HandleCell`), across sessions, or sending it to
another thread. A handle is a slot index and a generation in one 32-bit
word (`generation << 16 | index + 1`). Resolving it, with
`handle.get(&lv)` or `handle.get_mut(&mut lv)`, checks:

1. that the slot's generation matches the handle's;
2. that the slot is live and tracks an object (not a timer or a group);
3. that no deletion is pending for it (a dropped `Owned`, a
   `delete_later`);
4. inside a callback only, that neither the object nor an ancestor is
   being deleted (LVGL sends `DELETE` to children of an object it has
   already marked);
5. for a typed handle such as `Handle<Label>`, that the object's class is
   `W` or a subclass (`Handle<Obj>` skips this).

These are a few loads and compares: 2.4 ns for a `Handle<Obj>` and 3.1 ns
for a `Handle<Label>` in the benchmark.

**Generations and slot reuse.** The generation is 16 bits wide and
advances each time a slot is freed. A handle kept after its object died
is reported dead until the slot has been reused 65,536 times; after that
it may resolve to whatever the slot tracks then, if its class is
compatible. That is a logic error, never undefined behaviour: the
resolved object is live and of the right class. The registry also delays
reuse: it hands out never-used slots first, then the free slot that has
been free longest (a FIFO list). Drop handles to dead objects rather than
keeping them for days.

## The registry

Handles point into a single static array, the registry, placed in `.bss`
so that it costs no flash and no start-up copy:

```text
REG: Registry
┌──────────────────────────── Header, 24 B ───────────────────────────┐
│ ticks │ high-water mark │ free list head, tail │ pending screen load │
│ load animation │ callback depth │ any_req │ entered │ init           │
├──────────────────────────── N slots, 8 B each (32-bit) ─────────────┤
│ slot 0: pointer │ generation (16 bits) │ meta (8 bits) │ req (atomic)│
│ slot 1: ...                                                         │
└─────────────────────────────────────────────────────────────────────┘
```

N is `LVGL_RUST_HANDLE_SLOTS`, rounded up to a power of two (default 32,
at most 32,768), so the registry takes 8·N + 24 bytes on 32-bit targets
(280 B by default, 152 B for the 16 slots of the size-gate firmware) and
16·N + 24 bytes on 64-bit hosts. The power of two lets an index be masked
instead of bounds-checked, so no panic path is linked. Slots are taken
lazily: only objects you take a handle to, own, or request a deferred
operation on use one, plus owned timers and groups. When none is left,
the call returns `Error::RegistryFull`; nothing is allocated.

A fixed capacity is the default trade-off. A growable table needs an
allocator and a fallible operation in the middle of ordinary calls; most
interfaces hold a known, small number of objects they need to find
again, and everything else is reached through views from a parent or an
event.

### Growing the registry (`growable-registry`)

For applications whose number of handles is not known in advance, the
`growable-registry` feature keeps the N static slots and adds chunks
from the global allocator (the `alloc` crate, no `std`) when every slot
is in use and the free list is empty:

```text
static REG                      global allocator (never freed)
┌ Header, 24 B ┐                chunk 0: slots N   .. 2N - 1   (N slots)
├ N slots ─────┤                chunk 1: slots 2N  .. 4N - 1   (2N slots)
├ chunk table ─┤ ─ pointers ─▶  chunk 2: slots 4N  .. 8N - 1   (4N slots)
│ sentinel slot│                ...
└──────────────┘                last:    slots ... 65,534
```

Each chunk is as large as the registry before it, so the capacity
doubles (N·2^k after k chunks) and a slot index finds its chunk in
constant time: chunk `log2(index) − log2(N)`, at offset
`index − 2^log2(index)`. The limit is 65,535 slots, since a handle keeps
`index + 1` in 16 bits (handles stay 4 bytes); the last chunk stops
there. The chunk table holds one pointer per possible chunk,
16 − log2(N) of them, appended only; with the sentinel slot (what an
index of no published chunk resolves to: always free, so dead) it costs
52 B of `.bss` on 32-bit targets with the default 32 slots.

Slots never move: a `BTreeMap` or a reallocated array would move them,
while an `Owned` dropped on another thread writes into its slot without
the token. For the same reason chunks are never freed, not even by
`Lvgl::deinit` (an `Owned` dropped after it still stores its request):
at most (65,535 − N) slots of 8 B (524,024 B with N = 32 on 32-bit
targets; 16 B per slot on 64-bit ones) stay allocated once reached. The
global allocator must outlive LVGL for the same reason: it must not take
its memory from LVGL's own heap, which `Lvgl::deinit` releases. When the allocator returns null, the call that
needed a slot returns `Error::OutOfMemory` and takes nothing;
`Error::RegistryFull` remains for the 65,535-slot limit.

A chunk is published once: zeroed (every slot free, generation 0, no
request), then its pointer stored with Release ordering; every lookup
loads it with Acquire. The token holder sees its own stores and those of
earlier holders (ordered by the `LvglAccess` contract). A thread without
the token only reaches a chunk through an `Owned` minted in it, after
the store; moving that `Owned` to the thread synchronized with it (as
`Send` requires: a channel, a spawn, a lock), so the pointer is visible
there. Like the rest of the protocol below, this is loads and stores
only, without compare-and-swap.

### Kept in sync by LVGL

A registry is only useful if it knows when objects die, whatever deleted
them. lvgl-rust does not wrap every deletion path to find out; it lets
LVGL tell it. With `LV_USE_OBJ_ID 1` and `LV_USE_OBJ_ID_BUILTIN 0` in
`lv_conf.h`, LVGL calls five hook functions that the crate defines
(`base/hook.rs`):

- `lv_obj_t::id` holds the object's slot index plus one, or 0 when the
  object has no slot. `lv_obj_assign_id` does nothing: ids are assigned
  when safe code first takes a handle.
- `lv_obj_free_id` runs at the end of every object's destructor, on every
  deletion path. It frees the object's slot (advancing its generation),
  or marks it dead if an `Owned` still refers to it.

The price is the `id` field LVGL adds to every object: 4 bytes per
object, whether or not it has a handle. With the hooks, a handle stays
correct when raw `sys` code, a widget or LVGL's screen loading deletes
its object. `lv_obj_set_id` is reserved to the crate, and taking a handle
to an object finds its existing slot through `id` instead of scanning.

## `Owned`, and dropping from any thread

`Owned<W>` is the single owner of an object (or a timer or a group):
dropping it deletes what it owns. Drop needs no session, so an `Owned`
can live in a struct that is dropped on any thread, at any time. It
cannot delete there (that would need exclusive access); instead it
records a request that the next session carries out.

The registry has to accept that request from a thread that does not hold
the token, on targets with no compare-and-swap. The protocol uses stores
and fences only:

```text
 any thread, no token: Owned::drop          token holder: drain
 ─────────────────────────────────         ───────────────────────────────
 slot.req   = 1          (store)            if any_req (load):
 fence(SeqCst)                                any_req = false     (store)
 any_req    = true       (store)              fence(SeqCst)
                                              for each slot used so far:
                                                act on slot.req   (load)
```

The two SeqCst fences pair: either the drain's scan sees `req`, or the
drop's store to `any_req` comes after the drain cleared it, and the next
drain looks again. Code without the token writes only `req`, `any_req`
and `ticks`; every other field is a `Cell` touched only by the token
holder, and successive holders are ordered by the `LvglAccess` contract.
The loom models in `lvgl-rust/src/tests.rs` (`mise run loom`) explore
the interleavings of drops racing the drain (and, with
`growable-registry`, racing the publication of a chunk), and Miri runs the core tests
against a Rust model of the LVGL functions the core calls (`mise run
miri`).

`Owned::delete(self, &mut lv)` deletes at once instead, under the
exclusive access you already hold, after the same structural checks as
any other deletion.

## The drain

Everything deferred is carried out by the drain, which runs with the
token held and no callback on the stack:

- at the start of every session (`Lvgl::enter`);
- in `Lvgl::timer_handler`, both before `lv_timer_handler` (so dropped
  timers and objects are gone before timers run) and after it (so
  requests made by handlers and timers do not wait a whole period);
- in `Lvgl::deinit`, before `lv_deinit`.

It runs, in order: the slot requests (dropped `Owned` values,
`delete_later`, `clean_later`), a pending `load_screen_later`, the
functions queued with `Lvgl::post` (in the order they were posted), and
with feature `heap`, the frees of heap blocks retired by the deletions
before it. When nothing is pending, the drain is one load of `any_req`
and a few cheap checks; the benchmark measures a whole session at 5.8 ns.

`Obj::delete_later` and `clean_later` are how shared code, event handlers
in particular, delete objects. They run the structural checks at once,
so a permanent refusal is returned to the caller, and take a temporary
registry slot if the object has none; that slot is released when the
request completes.

## Structural checks

Exclusive access makes deletion safe for Rust references. It does not
make it safe for LVGL's own pointers, and LVGL accepts operations that
leave it holding freed memory:

- a display keeps pointers to its three layers and deletes them again in
  `lv_display_delete`;
- widgets keep pointers to some of their children (a textarea's label, a
  message box's header, title, footer and content) and find others by
  index (a tab view's tab bar buttons and pages, a roller's label as
  child 0);
- a tile view casts every child to a tile, and a calendar header's
  handlers read its parent as the calendar.

The structural checks (`base/structural.rs`) refuse these before they
reach LVGL, with `Error::Internal`. Their rules come from the
`[composite.<widget>]` tables of `classes.toml`, generated into the
crate: `parts` names how to find the objects a widget points to, `indexed`
how many levels below a part LVGL finds objects by position, `closed` a
widget whose children only its own API may create, and `pinned` a widget
that must not change parent. Parts are looked up when the check runs, so
lazily created ones count. The checks then decide:

| Operation | Refused when |
| --- | --- |
| delete, own | the object is a display layer, is or contains a part, is found by index, or sits inside a widget class the tables do not describe |
| clean | the object has parts, contains a part of a widget above it, or its children are counted by index |
| set parent, move to index | as for delete, or the (new) parent is a closed or indexed widget; for set parent, also when the object is pinned |
| create | the parent is closed or indexed (a new child would shift or be cast) |

A widget class with no entry is treated as unknown, and nothing inside it
can be deleted, owned, moved, cleaned or created: refusing is the safe
default for a widget nobody has read. Subclasses inherit their base's
entry. These checks walk the object's ancestors, and they are the main
work the crate adds when creating and deleting an object: the benchmark
measures 1,395.9 ns for the pair against 1,250.8 ns for the raw calls,
a difference of 145.1 ns.

### Busy screens

One refusal is temporary. While LVGL loads a screen, and during an
animated load for the previous screen too, it keeps pointers to both:
deleting either now would leave the load animation working on freed
memory, and loading the previous screen while `auto_delete` is set would
make LVGL delete it and then load the freed object. Those operations
return `Error::Busy`. A deferred request that meets `Busy` stays pending,
and the drain retries it until the load completes; `Owned::delete`
behaves the same way. Every other refusal is permanent: the drain drops
the request (counted as `refused` with feature `debug-counters`), and
`Owned::delete` returns it (`Error::Internal`), leaving the object alive
and unowned, as a refused drop does.

Related: [Access model](access-model.md),
[Event handlers](event-handlers.md), [Overhead](overhead.md),
[Errors](../reference/errors.md).
