# Use the heap

lvgl-rust needs no allocator: by default every closure, driver and image
is a function item, a 4-byte value or a `static`. This guide enables the
two heap features: `heap`, which keeps closures and owned values on
LVGL's own heap, and `allocator`, which makes that heap a Rust allocator.

## What you need

- lvgl-rust configured as in [Configure LVGL](configure-lvgl.md).
- For `heap`: room in LVGL's heap. With the builtin allocator
  (`LV_USE_STDLIB_MALLOC LV_STDLIB_BUILTIN`), that is `LV_MEM_SIZE`
  (65536 bytes in the vendored configuration).
- For `allocator`: a `static` allocator, for example the program's
  `GlobalAlloc`.

## Steps

### 1. Enable `heap`

```toml
[dependencies]
lvgl-rust = { git = "https://github.com/AlixANNERAUD/lvgl_rust", features = ["heap"] }
```

Every block goes through `lv_malloc`, so it comes from whatever allocator
LVGL is configured with. The feature adds 4 bytes of `.bss` (the list of
blocks waiting to be freed) and nothing else until you call its methods.

### 2. Keep closures with captures

```rust
use lvgl_rust::enums::EventCode;

let threshold: u64 = 1_000_000;
slider.on_boxed(EventCode::VALUE_CHANGED, move |e| {
    let _ = (threshold, e.code());
})?;

let timer = lv.timer_boxed(500, move |ctx| {
    let _ = (threshold, ctx.lv());
})?; // Owned<Timer>: dropping it deletes the timer
```

- `on_boxed` stores the closure after an 8-byte header (32-bit targets).
  The closure is dropped when the object is deleted, and its block freed at
  the next drain, because LVGL may still call it during the deletion.
- `timer_boxed` stores it after a 4-byte header. The block is freed by the
  drain that deletes the timer (after the `Owned<Timer>` is dropped or
  deleted).

Both fail with `Error::OutOfMemory` when LVGL's heap is full.

### 3. Give an image a source it owns

```rust
use lvgl_rust::{ImageBuffer, PixelFormat};

let mut image = image_handle.get_mut(&mut lv)?; // ObjMut<'_, Image>

// A small image built at run time: 8 × 8 pixels of 8-bit luminance.
image.set_src_owned(ImageBuffer::new(PixelFormat::L8, 8, 8, 0, [0x80; 64])?)?;

// A larger one, filled in place on the heap: never copied through the stack.
image.set_src_buffer::<{ 64 * 64 * 2 }>(PixelFormat::Rgb565, 64, 64, 0, |pixels| {
    pixels.fill(0xff);
})?;
```

The image (or an `ImageDsc`) moves into a block with a 4-byte header.
When a later `set_src_owned` replaces it or the image is deleted, LVGL's
image caches drop it and the block is freed at the next drain.
`ImageBuffer::new` and `set_src_buffer` return `Error::InvalidArg` if the
data does not match the format and size (`set_src_buffer` before
allocating); `set_src_owned` and `set_src_buffer` return it if LVGL
refuses the image, and `Error::OutOfMemory` if the block cannot be
allocated. `ImageBuffer::new` and `set_src_buffer` exist on
little-endian targets only.

### 4. Put an event queue on the heap (with feature `async`, not in views-only mode)

```rust
use lvgl_rust::{Event, OwnedQueue, enums::EventCode};

fn capture(e: &Event<'_>) -> Option<u32> {
    Some(e.code().bits())
}

let rx: OwnedQueue<u32, 8> = button.subscribe_owned(EventCode::CLICKED, capture)?;
```

The queue (indices, an `Option<Waker>`, the capture function and `N`
events) goes in a block with a 4-byte header; the `OwnedQueue` receiver is
8 bytes and uses the object's registry slot. The block is freed at the
next drain after the object is deleted or `rx.close(&mut lv)` detaches it.
See [Receive events asynchronously](receive-events-asynchronously.md).

### 5. Keep drivers on the heap

`Display::create_boxed`, `Indev::create_boxed` and
`Indev::create_button_boxed` take the driver without a `static` cell and
free it when the display or device is deleted (never while a flush is
pending). See [Write a display driver](write-a-display-driver.md) and
[Write an input driver](write-an-input-driver.md).

### 6. Make LVGL's heap a Rust allocator (feature `allocator`)

1. In `lv_conf.h`:

   ```c
   #define LV_USE_STDLIB_MALLOC LV_STDLIB_CUSTOM
   ```

   Without it, the build stops with "the `allocator` feature needs
   `#define LV_USE_STDLIB_MALLOC LV_STDLIB_CUSTOM` in lv_conf.h".
   `LV_MEM_SIZE` no longer applies.

2. Enable the feature (with `heap` if you also want step 2 to 5):

   ```toml
   lvgl-rust = { git = "https://github.com/AlixANNERAUD/lvgl_rust", features = ["allocator", "heap"] }
   ```

3. Install the allocator once, in the binary crate:

   ```rust
   use lvgl_rust::SizedHeader;

   static LVGL_HEAP: SizedHeader<std::alloc::System> = SizedHeader(std::alloc::System);
   lvgl_rust::lvgl_allocator!(LVGL_HEAP);
   ```

   `lvgl_allocator!` defines the functions LVGL calls with
   `LV_STDLIB_CUSTOM` (`lv_malloc_core`, `lv_realloc_core`,
   `lv_free_core`, ...). `SizedHeader` adapts any `GlobalAlloc + Sync`:
   LVGL's `realloc` and `free` do not pass the size, so each block starts
   with a header of one pointer holding it. On an embedded target, wrap
   the same allocator your `#[global_allocator]` uses.

   To use something else, implement the `unsafe` trait `LvglAllocator`
   (`alloc`, `realloc`, `free`): blocks must be aligned to a pointer, and
   `realloc` must keep the old block valid when it fails.

## Costs

From the [overhead reference](../reference/overhead.md), on 32-bit
targets:

| Item | Cost |
| --- | --- |
| Deferred-free list | 4 B of `.bss` |
| `on_boxed` | one LVGL heap block: 8 B header, then the closure |
| `timer_boxed` | one LVGL heap block: 4 B header, then the closure |
| `set_src_owned`, `set_src_buffer` | one LVGL heap block: 4 B header, then the image |
| `subscribe_owned` | one LVGL heap block: 4 B header, then the queue; the receiver is 8 B |

LVGL's allocator adds its own block header to each, and `SizedHeader`
one pointer more. Delivering an event to a boxed closure measured 21.7 ns
against 19.0 ns for a raw `extern "C"` callback reading its user data, on
the machine of that page.

## Pitfalls

- **Blocks are freed at the next drain, not at once.** A deleted object's
  closure is dropped during the deletion, but its block waits for the
  next `Lvgl::enter` or `timer_handler`. Keep some headroom in a tight
  heap.
- **`bubble_children` is permanent.** A `subscribe_owned` with
  `SubscribeOptions::new(code).bubble_children()` leaves the children's
  `EVENT_BUBBLE` flag for the object's lifetime, after `close` too.
- **A plain `set_src` keeps the old block.** After a raw or generated
  `set_src`, the previous owned source stays allocated until the next
  `set_src_owned` or the image's deletion.
- **One allocator only.** `lvgl_allocator!` defines unmangled symbols:
  use it once per binary, and not together with your own `lv_malloc_core`.
- **`lv_mem_monitor` reports zeros** with `allocator`: the Rust allocator
  keeps its own statistics.

## See also

- [Reduce the footprint](reduce-the-footprint.md)
- [Features reference](../reference/features.md)
- [Overhead explanation](../explanation/overhead.md)
