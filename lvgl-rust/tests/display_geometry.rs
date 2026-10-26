//! Display setters that change what LVGL renders into the draw buffers:
//! resolution, rotation, color format and render mode (their own process:
//! they create displays).

use lvgl_rust::{
    Display, DisplayCell, DisplayFormat, DrawBuf, DrawBuffers, Error, Flush, FlushRequest, Lvgl,
    LvglAccess, RenderMode, Rotation, sys, tick_inc, widgets::Button,
};
use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicU32, Ordering::Relaxed},
};

static LOCK: Mutex<()> = Mutex::new(());

struct Guard(#[allow(dead_code)] MutexGuard<'static, ()>);

// SAFETY: a guard of the mutex serializing every LVGL call of this process.
unsafe impl LvglAccess for Guard {}

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
    let mut guard = Guard(LOCK.lock().unwrap());
    let mut lv = lvgl_rust::init(&mut guard).unwrap();

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
