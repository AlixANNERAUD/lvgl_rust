# Overhead

A safe layer over a C library costs something: checks the C code does not
make, bookkeeping it does not need. lvgl-rust is designed so that the
cost is small, fixed and visible. This page explains where each cost
comes from and why it is the size it is. The numbers quoted here come
from [the overhead measurements](../reference/overhead.md), which
`mise run overhead` regenerates; read that page for the machine, the
method and the full tables.

## Design principles

- **Access is a type, not a value.** `Session` and `Lvgl` are zero-sized;
  shared and exclusive access are borrows checked at compile time. A call
  through a view is the FFI call and nothing else.
- **Static memory, sized at build time.** The handle registry and the
  `post` queue are statics in `.bss`, sized by `LVGL_RUST_HANDLE_SLOTS`
  and `LVGL_RUST_POST_SLOTS`. Nothing is allocated unless you enable
  feature `heap` and call a function that says so, and even then the
  block comes from LVGL's own heap.
- **Pay where you use it.** Handles cost a registry slot only when you
  take one; deferred requests cost a drain only when one is pending;
  diagnostic features cost nothing when off.
- **No formatting or panic machinery.** Indices are masked instead of
  bounds-checked, numbers are formatted without `core::fmt`, and the size
  gate checks that no `core::fmt`, `core::panicking` or `unwrap_failed`
  symbol is linked in the firmware. Formatting is opt-in with feature
  `fmt`.

## Time

Host build, release profile, median of 21 samples; raw calls include the
OBJ_ID hooks, which LVGL runs for every object either way.

| Operation | lvgl-rust | Raw | Where the difference goes |
| --- | ---: | ---: | --- |
| Shared setter through a view | 1.7 ns | 1.7 ns | nowhere: the same call |
| Start and end a session | 5.8 ns | none | atomic loads and stores of the session flags, and a drain with nothing pending |
| Resolve a `Handle<Obj>` | 2.4 ns | none | generation, state and pending-request checks in the registry |
| Resolve a `Handle<Label>` | 3.1 ns | none | the same, plus a class check |
| Event to an `on` handler | 20.7 ns | 18.1 ns | the trampoline: tier bitmask test, callback depth level |
| Event to an `on_boxed` closure | 21.7 ns | 19.0 ns | the same, plus the box's header bookkeeping |
| Create and delete an object | 1395.9 ns | 1250.8 ns | structural checks on creation and deletion, walking the ancestors' composite rules |
| `Owned` dropped, deleted by the drain | 1613.7 ns | 1269.6 ns | the registry slot, the request, a session to drain it, the checks |
| `lv.post(f, data)` run by the drain | 7.5 ns | 2.0 ns | queueing, and a session to drain it |
| `timer_handler` with nothing pending | 24.5 ns | 20.1 ns | a drain before and after `lv_timer_handler` |

The pattern is that the safe layer adds a few nanoseconds to
operations that are themselves cheap, and between about 12 % (145.1 ns
over 1250.8 ns) and 27 % (344.1 ns over 1269.6 ns) to creating and
deleting objects, depending on the path, which LVGL itself makes expensive (more
than a microsecond here). Rows that drain include the cost of a whole
session per operation; a real program drains once per session, for all
pending requests together.

Why each cost is small:

- **Views.** `&Obj` is a pointer to a zero-sized type at the object's
  address. The generated method passes it straight to LVGL.
- **Sessions.** `Lvgl::enter` checks the session flags, sets one and runs the
  drain, which with nothing pending reads `any_req`, the pending screen
  load, the length of the `post` queue and the deferred-free list head.
  Dropping the session stores one flag.
- **Handle resolution.** The handle's low 16 bits, masked, index the
  registry directly (no search); the rest is a few compares on one slot.
  The check for ancestors being deleted runs only inside callbacks,
  where it can matter. The class check is skipped for `Handle<Obj>`.
- **Trampolines.** The closure is zero-sized and its 4 bytes of data are
  stored by value in LVGL's user data, so the trampoline loads nothing
  extra. The shared part (the tier mask and the depth counter) is one
  non-generic function.
- **Structural checks.** They walk up from the object, and for each
  ancestor test its class against the composite tables; parts are found
  by calling LVGL's getters. The walk is proportional to the depth of the
  tree, not its size, and runs only for operations that change the tree.
  It is the price of refusing operations that would leave LVGL with
  dangling pointers (see [Object lifetime](object-lifetime.md)).

## Memory

### Fixed costs, on 32-bit targets

| Item | Cost |
| --- | --- |
| Handle registry | 8·N + 24 B of `.bss` (280 B for the default N = 32); 8 B in views-only mode |
| `post` queue | 8·P + 4 B of `.bss` (36 B for the default P = 4) |
| Debug counters (feature `debug-counters`) | 8 B of `.bss` |
| Deferred-free list (feature `heap`) | 4 B of `.bss` |
| `.data` | none |

The size gate builds firmware for `thumbv6m-none-eabi` and
`riscv32imc-unknown-none-elf` and checks the registry, the `post` queue
and the debug counters exactly, along with the absence of `.data`, so
they cannot drift unnoticed. Its firmware does not enable `heap`, so the
deferred-free list is not part of that check.

### Per-object and per-item costs

| Item | Cost |
| --- | --- |
| Each LVGL object | 4 B for `lv_obj_t::id` (`LV_USE_OBJ_ID`), whether or not it has a handle; none in views-only mode, which needs no OBJ_ID hooks |
| Each object, timer or group with a handle | one registry slot, no heap |
| `on`, `on_with`, `on_fn`, `on_static` handlers | LVGL's event descriptor only, as for a raw callback |
| `on_boxed` (feature `heap`) | one LVGL heap block: 8 B header, then the closure |
| `timer_boxed` (feature `heap`) | one LVGL heap block: 4 B header, then the closure |
| `set_src_owned`, `set_src_buffer` (feature `heap`) | one LVGL heap block: 4 B header, then the image |
| `Handle<W>`, `Owned<W>`, and their `Option` | 4 B |
| `ObjMut<'_, W>`, views | one pointer |
| `Session`, `Lvgl` | 0 B |

LVGL's allocator adds its own header to each heap block.

### Code

Firmware of the size gate (size profile, no LTO), whole image including
LVGL:

| Target | Configuration | Crate code | Firmware text | Crate share |
| --- | --- | ---: | ---: | ---: |
| `thumbv6m-none-eabi` | 16 handle slots | 4498 B | 233252 B | 1.9 % |
| `thumbv6m-none-eabi` | views only | 2246 B | 230368 B | 1.0 % |
| `riscv32imc-unknown-none-elf` | 16 handle slots | 5658 B | 256368 B | 2.2 % |
| `riscv32imc-unknown-none-elf` | views only | 2820 B | 252948 B | 1.1 % |

The crate's code is held under a budget per target (4608 B and 2304 B on
thumbv6m, 5888 B and 2944 B on riscv32imc), checked in CI by
`mise run size-gate`. A change that needs more raises the budget in the
same change, with the reason. Generated wrappers are `#[inline]`: a
wrapper you call compiles to the FFI call it wraps, plus any argument
conversion, and one you do not call costs nothing.

## What is not counted, and caveats

- Host timings depend on the machine; the reference page was measured
  in a virtual machine, where results move by tens of nanoseconds between
  runs for the operations that create objects. Compare the two columns
  of one run, not runs on different machines.
- Embedded sizes are without LTO, so that the crate's symbols stay
  separate and countable. With LTO, code a program does not call is
  removed.
- LVGL's own costs (its heap, its object sizes, its rendering) are the
  same with or without lvgl-rust, apart from the 4-byte `id` per object
  when handles are enabled.

## Regenerating the numbers

`mise run overhead` runs the benchmark (`lvgl-rust/benches/overhead.rs`,
release profile, feature `heap`, the vendored `lvgl-rust/conf/lv_conf.h`)
and then the size gate, and rewrites
[`documentation/reference/overhead.md`](../reference/overhead.md). It is
not part of CI, because host timings vary between machines; the size
gate itself is, so the memory numbers are checked on every change.
Without the cross C toolchains (`mise bootstrap packages apply`), the
script takes the crate's sizes from the size-gate job of CI and says so
on the page.

Related: [Reduce the footprint](../how-to-guides/reduce-the-footprint.md),
[Features](../reference/features.md),
[Build configuration](../reference/build-configuration.md).
