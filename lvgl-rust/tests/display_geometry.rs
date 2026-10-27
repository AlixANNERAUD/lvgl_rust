//! Display setters that change what LVGL renders into the draw buffers:
//! resolution, rotation, color format and render mode (their own process:
//! they create displays).

use lvgl_rust::{
    Display, DisplayCell, DisplayFormat, DrawBuf, DrawBuffers, Error, Flush, FlushRequest, Lvgl,
    LvglAccess, RenderMode, Rotation, Session, sys, tick_inc, widgets::Button,
};
use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicU32, Ordering::Relaxed},
};

/// Whether LVGL is initialized, under the lock.
static LOCK: Mutex<bool> = Mutex::new(false);

struct Guard(MutexGuard<'static, bool>);

// SAFETY: a guard of the mutex serializing every LVGL call of this process.
unsafe impl LvglAccess for Guard {}

fn guard() -> Guard {
    Guard(LOCK.lock().unwrap_or_else(|p| p.into_inner()))
}

/// The first test initializes LVGL, the others enter it.
fn session(guard: &mut Guard) -> Session<'_> {
    if !*guard.0 {
        *guard.0 = true;
        lvgl_rust::init(guard).unwrap()
    } else {
        Lvgl::enter(guard).unwrap()
    }
}

/// Checks every flush against the display's resolution and buffer.
struct Panel {
    display: &'static AtomicU32,
    flushes: &'static AtomicU32,
    buffer_len: usize,
}

impl Flush for Panel {
    fn flush(&mut self, mut req: FlushRequest<'_>) {
        let area = req.area();
        assert!(area.x1 >= 0 && area.y1 >= 0 && area.x2 >= area.x1 && area.y2 >= area.y1);
        let rows = (area.y2 - area.y1 + 1) as usize;
        let stride = req.stride();
        assert!(stride > 0);
        let pixels = req.pixels();
        assert!(pixels.len() <= self.buffer_len);
        assert!(stride * rows <= self.buffer_len);
        // Every byte handed out is readable and writable.
        for byte in pixels.iter_mut() {
            *byte = byte.wrapping_add(0);
        }
        let _ = self.display.load(Relaxed);
        self.flushes.fetch_add(1, Relaxed);
    }
}

fn run(lv: &mut Lvgl, frames: u32) {
    for _ in 0..frames {
        tick_inc(40);
        lv.timer_handler();
    }
}

/// One row of 240 pixels at 32 bits.
const LINE: usize = 240 * 4;
static LINE_BUFFER: DrawBuf<LINE> = DrawBuf::new();
static LINE_CELL: DisplayCell<Panel> = DisplayCell::new();
static LINE_FLUSHES: AtomicU32 = AtomicU32::new(0);
static LINE_ID: AtomicU32 = AtomicU32::new(1);

/// Exactly a 200 × 100 RGB565 screen (also 100 × 200 once turned).
const SCREEN: usize = 200 * 2 * 100;
static SCREEN_A: DrawBuf<SCREEN> = DrawBuf::new();
static SCREEN_B: DrawBuf<SCREEN> = DrawBuf::new();
static SCREEN_CELL: DisplayCell<Panel> = DisplayCell::new();
static SCREEN_FLUSHES: AtomicU32 = AtomicU32::new(0);
static SCREEN_ID: AtomicU32 = AtomicU32::new(2);

#[test]
fn geometry_changes_are_checked_against_the_buffers() {
    let mut guard = guard();
    let mut lv = session(&mut guard);

    // Partial mode, one row of buffer.
    let line = Display::create(
        &mut lv,
        &LINE_CELL,
        Panel {
            display: &LINE_ID,
            flushes: &LINE_FLUSHES,
            buffer_len: LINE,
        },
        240,
        160,
        DrawBuffers::single(&LINE_BUFFER),
        RenderMode::Partial,
    )
    .unwrap();
    run(&mut lv, 2);
    assert!(LINE_FLUSHES.load(Relaxed) > 0);

    line.set_color_format(&mut lv, DisplayFormat::Rgb565)
        .unwrap();
    line.set_rotation(&mut lv, Rotation::Degrees90).unwrap();
    assert_eq!(line.resolution(&lv), (160, 240));
    run(&mut lv, 2);
    line.set_rotation(&mut lv, Rotation::Degrees0).unwrap();

    // 480 RGB565 pixels fill the row; a 32-bit row of them does not.
    line.set_resolution(&mut lv, 480, 160).unwrap();
    run(&mut lv, 2);
    assert_eq!(
        line.set_color_format(&mut lv, DisplayFormat::Xrgb8888),
        Err(Error::TooSmall)
    );
    assert_eq!(line.set_resolution(&mut lv, 481, 160), Err(Error::TooSmall));
    // Turned, a row is 160 pixels: the format fits, but not the turn back.
    line.set_rotation(&mut lv, Rotation::Degrees270).unwrap();
    line.set_color_format(&mut lv, DisplayFormat::Xrgb8888)
        .unwrap();
    run(&mut lv, 2);
    assert_eq!(
        line.set_rotation(&mut lv, Rotation::Degrees180),
        Err(Error::TooSmall)
    );
    // Refusals change nothing.
    assert_eq!(line.resolution(&lv), (160, 480));
    // SAFETY: under the token, a live display.
    unsafe {
        assert_eq!(
            sys::lv_display_get_color_format(line.as_ptr()),
            sys::lv_color_format_t_LV_COLOR_FORMAT_XRGB8888
        );
        assert_eq!(
            sys::lv_display_get_rotation(line.as_ptr()),
            sys::lv_display_rotation_t_LV_DISPLAY_ROTATION_270
        );
    }
    // A row is not a screen.
    assert_eq!(
        line.set_render_mode(&mut lv, RenderMode::Full),
        Err(Error::TooSmall)
    );
    assert_eq!(
        line.set_render_mode(&mut lv, RenderMode::Direct),
        Err(Error::TooSmall)
    );
    line.set_render_mode(&mut lv, RenderMode::Partial).unwrap();
    assert_eq!(line.set_resolution(&mut lv, 0, 10), Err(Error::InvalidArg));
    assert_eq!(
        line.set_resolution(&mut lv, u32::MAX, 10),
        Err(Error::InvalidArg)
    );
    assert_eq!(line.set_tile_count(&mut lv, 0), Err(Error::InvalidArg));
    assert_eq!(line.set_tile_count(&mut lv, 256), Err(Error::InvalidArg));
    line.set_tile_count(&mut lv, 4).unwrap();
    run(&mut lv, 2);
    line.try_delete(&mut lv).map_err(|e| e.1).unwrap();

    // Two screen-sized buffers. Created in partial mode, which fits any
    // default color depth, then turned into a direct display.
    let screen = Display::create(
        &mut lv,
        &SCREEN_CELL,
        Panel {
            display: &SCREEN_ID,
            flushes: &SCREEN_FLUSHES,
            buffer_len: SCREEN,
        },
        200,
        100,
        DrawBuffers::double(&SCREEN_A, &SCREEN_B),
        RenderMode::Partial,
    )
    .unwrap();
    screen.set_default(&mut lv);
    screen
        .set_color_format(&mut lv, DisplayFormat::Rgb565)
        .unwrap();
    screen.set_render_mode(&mut lv, RenderMode::Direct).unwrap();
    assert_eq!(
        screen.set_resolution(&mut lv, 200, 101),
        Err(Error::TooSmall)
    );
    assert_eq!(
        screen.set_color_format(&mut lv, DisplayFormat::Rgb888),
        Err(Error::TooSmall)
    );

    // Small invalidations between frames: direct mode with two buffers
    // copies the areas of the previous frame into the other buffer.
    Button::create(lv.screen_active().unwrap()).unwrap();
    let moves = |lv: &mut Lvgl, frames: u32| {
        for i in 0..frames {
            let mut button = lv
                .edit(|lv| lv.screen_active()?.child(0)?.downcast::<Button>())
                .unwrap();
            button.set_size(30, 20);
            button.set_pos(5 + 7 * i as i32, 5 + 3 * i as i32);
            run(lv, 1);
        }
    };
    moves(&mut lv, 4);
    let before = SCREEN_FLUSHES.load(Relaxed);
    // Turned: 100 × 200, the same bytes with a smaller stride and more
    // rows. The copy of the previous frame's areas uses the new shape.
    screen.set_rotation(&mut lv, Rotation::Degrees90).unwrap();
    assert_eq!(screen.resolution(&lv), (100, 200));
    moves(&mut lv, 4);
    screen.set_rotation(&mut lv, Rotation::Degrees0).unwrap();
    moves(&mut lv, 4);
    screen.set_render_mode(&mut lv, RenderMode::Full).unwrap();
    screen.set_rotation(&mut lv, Rotation::Degrees270).unwrap();
    moves(&mut lv, 4);
    screen
        .set_render_mode(&mut lv, RenderMode::Partial)
        .unwrap();
    screen
        .set_color_format(&mut lv, DisplayFormat::Xrgb8888)
        .unwrap();
    moves(&mut lv, 2);
    assert_eq!(
        screen.set_render_mode(&mut lv, RenderMode::Direct),
        Err(Error::TooSmall)
    );
    assert!(SCREEN_FLUSHES.load(Relaxed) > before);
    screen.try_delete(&mut lv).map_err(|e| e.1).unwrap();
}

/// A row of 100 RGB565 pixels.
static ROW_BUFFER: DrawBuf<200> = DrawBuf::new();
static ROW_CELL: DisplayCell<Panel> = DisplayCell::new();
static ROW_FLUSHES: AtomicU32 = AtomicU32::new(0);
static ROW_ID: AtomicU32 = AtomicU32::new(3);

/// Sizes whose stride or buffer size wraps LVGL's `uint32_t` arithmetic
/// (`width * bpp`, `lv_draw_buf.c:705`) and tile counts above the buffer
/// size (a zero divisor, `lv_refr.c:935-937`) are refused.
#[test]
fn wrapping_sizes_and_tile_counts_are_refused() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let row = Display::create(
        &mut lv,
        &ROW_CELL,
        Panel {
            display: &ROW_ID,
            flushes: &ROW_FLUSHES,
            buffer_len: 200,
        },
        50,
        10,
        DrawBuffers::single(&ROW_BUFFER),
        RenderMode::Partial,
    )
    .unwrap();
    row.set_color_format(&mut lv, DisplayFormat::Rgb565)
        .unwrap();
    row.set_resolution(&mut lv, 100, 10).unwrap();
    // 16 * (2^28 + 1) wraps to 16 bits: a stride of 2 bytes.
    assert_eq!(
        row.set_resolution(&mut lv, (1 << 28) + 1, 1),
        Err(Error::InvalidArg)
    );
    // Above LV_COORD_MAX.
    assert_eq!(
        row.set_resolution(&mut lv, 1 << 29, 1),
        Err(Error::InvalidArg)
    );
    row.set_resolution(&mut lv, 50, 10).unwrap();
    row.set_color_format(&mut lv, DisplayFormat::Xrgb8888)
        .unwrap();
    // 32 * (2^27 + 1) wraps to 32 bits (a stride of 4 bytes), below
    // LV_COORD_MAX.
    assert_eq!(
        row.set_resolution(&mut lv, (1 << 27) + 1, 1),
        Err(Error::InvalidArg)
    );
    assert_eq!(
        row.set_resolution(&mut lv, 1, (1 << 27) + 1),
        Err(Error::InvalidArg)
    );
    assert_eq!(row.set_resolution(&mut lv, 51, 10), Err(Error::TooSmall));
    assert_eq!(row.resolution(&lv), (50, 10));

    // 200 bytes: at most 200 tiles.
    assert_eq!(row.set_tile_count(&mut lv, 201), Err(Error::InvalidArg));
    assert_eq!(row.set_tile_count(&mut lv, 255), Err(Error::InvalidArg));
    row.set_tile_count(&mut lv, 200).unwrap();
    run(&mut lv, 2);
    row.set_tile_count(&mut lv, 1).unwrap();
    row.try_delete(&mut lv).map_err(|e| e.1).unwrap();
}

static TWICE: DrawBuf<SCREEN> = DrawBuf::new();
static TWICE_CELL: DisplayCell<Panel> = DisplayCell::new();
/// One byte short of a row of 101 RGB565 pixels rounded up to
/// `LV_DRAW_BUF_ALIGN` (4): 202 bytes, 204 once rounded.
static SHORT: DrawBuf<203> = DrawBuf::new();
static SHORT_CELL: DisplayCell<Panel> = DisplayCell::new();

/// Creation refuses the same buffer twice (LVGL would render into the
/// buffer a deferred flush still reads) and buffers that fit only before
/// LVGL rounds sizes to `LV_DRAW_BUF_ALIGN`.
#[test]
fn creation_refuses_shared_and_unaligned_buffers() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let panel = |len| Panel {
        display: &SCREEN_ID,
        flushes: &SCREEN_FLUSHES,
        buffer_len: len,
    };
    assert!(matches!(
        Display::create(
            &mut lv,
            &TWICE_CELL,
            panel(SCREEN),
            200,
            100,
            DrawBuffers::double(&TWICE, &TWICE),
            RenderMode::Partial,
        ),
        Err(Error::AlreadyOwned)
    ));
    // Neither the buffer nor the cell stays claimed.
    let twice = Display::create(
        &mut lv,
        &TWICE_CELL,
        panel(SCREEN),
        200,
        100,
        DrawBuffers::single(&TWICE),
        RenderMode::Partial,
    )
    .unwrap();
    twice.try_delete(&mut lv).map_err(|e| e.1).unwrap();

    // Refused at any color depth: 203 bytes hold 202 but not 204 (16
    // bits), nor 404 (32 bits).
    assert!(matches!(
        Display::create(
            &mut lv,
            &SHORT_CELL,
            panel(203),
            101,
            10,
            DrawBuffers::single(&SHORT),
            RenderMode::Partial,
        ),
        Err(Error::InvalidArg)
    ));
    let short = Display::create(
        &mut lv,
        &SHORT_CELL,
        panel(203),
        50,
        10,
        DrawBuffers::single(&SHORT),
        RenderMode::Partial,
    )
    .unwrap();
    short.try_delete(&mut lv).map_err(|e| e.1).unwrap();
}
