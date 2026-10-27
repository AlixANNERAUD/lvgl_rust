# Write a display driver

This guide connects a screen to LVGL: a `Flush` driver that sends
rendered pixels, its draw buffers in `static`s, and the checks lvgl-rust
makes when the geometry changes.

## What you need

- An initialised session (`lvgl_rust::init`), see the
  [first application tutorial](../tutorials/first-application.md).
- A way to send pixels to the panel: SPI, a parallel bus, a framebuffer.
- The color format the panel takes, matching `LV_COLOR_FORMAT_DEFAULT`
  or a `color_depth_*` feature ([Configure LVGL](configure-lvgl.md)).

## Steps

### 1. Implement `Flush`

LVGL hands each rendered area to `Flush::flush` in a `FlushRequest`:

```rust
use lvgl_rust::{Flush, FlushRequest};

struct Panel {
    // the SPI bus, the pins, ...
}

impl Flush for Panel {
    fn flush(&mut self, mut req: FlushRequest<'_>) {
        let area = req.area();     // lv_area_t: x1, y1, x2, y2, inclusive
        let stride = req.stride(); // bytes between the starts of two rows
        let width = (area.x2 - area.x1 + 1) as usize;
        let bytes_per_row = width * 2; // RGB565
        for (row, y) in req.pixels().chunks(stride).zip(area.y1..=area.y2) {
            // send &row[..bytes_per_row] for line `y`, columns x1..=x2
            let _ = (row, y, bytes_per_row);
        }
    } // returning tells LVGL the buffer is free again
}
```

- In partial mode, `pixels()` is `stride` × area height bytes. In direct
  and full modes it is the whole buffer, rows of the screen.
- `pixels()` is `&mut [u8]`: you may convert in place (swap bytes, rotate)
  before sending.
- The driver must be `Send + 'static`. LVGL calls it from
  `Lvgl::timer_handler`, with the lock held.

### 2. Declare the cell and the draw buffers as statics

```rust
use lvgl_rust::{DisplayCell, DrawBuf};

const WIDTH: usize = 320;
const HEIGHT: usize = 240;
const BYTES_PER_PIXEL: usize = 2; // RGB565

static PANEL: DisplayCell<Panel> = DisplayCell::new();
// A tenth of the screen each: 320 × 24 × 2 = 15 360 bytes.
static BUF_A: DrawBuf<{ WIDTH * BYTES_PER_PIXEL * 24 }> = DrawBuf::new();
static BUF_B: DrawBuf<{ WIDTH * BYTES_PER_PIXEL * 24 }> = DrawBuf::new();
```

`DisplayCell` holds the driver and the crate's state of the display;
`DrawBuf` is aligned to `LV_DRAW_BUF_ALIGN`. Both are claimed by the
display that uses them until it is deleted, and live in `.bss`, so no
heap is needed.

### 3. Create the display

```rust
use lvgl_rust::{Display, DrawBuffers, RenderMode};

let display = Display::create(
    &mut lv,
    &PANEL,
    Panel { /* ... */ },
    WIDTH as u32,
    HEIGHT as u32,
    DrawBuffers::double(&BUF_A, &BUF_B),
    RenderMode::Partial,
)?;
```

- `DrawBuffers::single(&BUF)` uses one buffer: LVGL waits for each flush
  before rendering the next area. `DrawBuffers::double(&A, &B)` renders
  into one while the other is sent; LVGL uses the size of the smaller one.
- `RenderMode::Partial` renders areas in turn into buffers of at least one
  row. `RenderMode::Direct` renders in place into screen-sized buffers, and
  `RenderMode::Full` the whole screen into screen-sized buffers.
- The first display becomes the default one; `display.set_default(&mut
  lv)` changes it.

`Display::create` fails with:

| Error | When |
| --- | --- |
| `Error::AlreadyOwned` | The cell or a buffer is in use, or both buffers are the same `static`. |
| `Error::InvalidArg` | A size is 0 or above `LV_COORD_MAX`, or a buffer is too small for `mode` (one row in partial mode, the screen otherwise). |
| `Error::OutOfMemory` | LVGL could not allocate the display. |

The returned `Display` is two pointers (8 bytes on 32-bit targets).
Dropping it keeps the display (with the `debug-counters` feature,
`DebugCounters::read().leaked` counts it);
delete it explicitly with `display.try_delete(&mut lv)`, which deletes its
screens, drops the driver and releases the cell and the buffers. It fails
with `Error::Busy`, giving the display back, while a flush is pending.

### 4. Complete flushes from DMA or an interrupt (optional)

To return from `flush` before the transfer ends, call
`FlushRequest::defer`. The buffer stays busy until the returned
`FlushDone` is completed with `ready()` or dropped:

```rust
use core::cell::RefCell;
use critical_section::Mutex;
use lvgl_rust::{Flush, FlushDone, FlushRequest};

static PENDING: Mutex<RefCell<Option<FlushDone>>> = Mutex::new(RefCell::new(None));

impl Flush for Panel {
    fn flush(&mut self, mut req: FlushRequest<'_>) {
        start_dma(req.pixels());
        let done = req.defer();
        critical_section::with(|cs| *PENDING.borrow_ref_mut(cs) = Some(done));
    }
}

// In the DMA-complete interrupt handler:
fn dma_complete() {
    if let Some(done) = critical_section::with(|cs| PENDING.borrow_ref_mut(cs).take()) {
        done.ready(); // one Release store
    }
}
```

`FlushDone` is `Send` and safe to complete from an interrupt.

### 5. Change the geometry at run time (optional)

These methods take `&mut Lvgl` and check that the draw buffers still hold
what LVGL will render:

| Method | Fails with |
| --- | --- |
| `set_resolution(&mut lv, width, height)` | `InvalidArg` for 0, a size above `LV_COORD_MAX` or one whose buffer sizes would wrap LVGL's 32-bit arithmetic; `TooSmall` if the buffers cannot hold a row of the new width (partial mode) or the new screen (full and direct modes). |
| `set_rotation(&mut lv, Rotation::Degrees90)` | `TooSmall` if the buffers cannot hold what LVGL renders once the resolutions are swapped. The driver rotates the pixels it sends. |
| `set_color_format(&mut lv, DisplayFormat::Rgb565Swapped)` | `TooSmall` if the buffers cannot hold the screen or row in the new format. |
| `set_render_mode(&mut lv, RenderMode::Full)` | `TooSmall` unless the buffers hold the whole screen (partial mode always succeeds). |
| `set_tile_count(&mut lv, count)` | `InvalidArg` outside `1..=255` or above the buffer size in bytes. |

`DisplayFormat` has `Rgb565`, `Rgb565Swapped` (for SPI panels that take
the high byte first), `Rgb888`, `Xrgb8888`, `Argb8888`, `L8`, `Al88` and
`A8`; each needs its `LV_DRAW_SW_SUPPORT_*` option in `lv_conf.h`.
`display.resolution(&lv)` returns the size after rotation.

### 6. Keep the driver on LVGL's heap instead (optional, feature `heap`)

`Display::create_boxed` takes the same arguments without the cell. The
driver and the crate's state go in one block of LVGL's heap, freed when
the display is deleted:

```rust
let display = Display::create_boxed(
    &mut lv,
    Panel { /* ... */ },
    WIDTH as u32,
    HEIGHT as u32,
    DrawBuffers::single(&BUF_A),
    RenderMode::Partial,
)?;
```

It fails with `Error::OutOfMemory` if the block cannot be allocated. The
draw buffers stay `static`s.

## Pitfalls

- **Complete deferred flushes elsewhere.** LVGL may need the buffer again
  before `timer_handler` returns (to flush the next area, or to render
  into it with a single buffer), and then spins on the thread that runs
  `timer_handler` until the flush completes. Complete from an interrupt or
  another thread, never later on that same thread, or it waits forever.
- **Size buffers for the mode.** Direct and full modes need buffers that
  hold the whole screen; partial mode needs at least one row, and more
  rows mean fewer flushes per refresh.
- **Raw calls bypass the checks.** `display.as_ptr()` gives the
  `lv_display_t` for raw calls. LVGL itself checks buffer sizes only when
  they are set, and only with `LV_USE_CHECK_ARG`: a raw
  `lv_display_set_resolution` can make it render past the buffers.
- **Ticks and the timer handler still have to run.** Nothing is drawn
  unless something calls `tick_inc` (or sets a tick callback) and
  `Lvgl::timer_handler` runs regularly.

## See also

- [Write an input driver](write-an-input-driver.md)
- [Use the heap](use-the-heap.md)
- [Errors reference](../reference/errors.md)
