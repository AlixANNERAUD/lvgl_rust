//! Display and input drivers (their own process: they create the display).

use lvgl_rust::{
    Display, DisplayCell, DrawBuf, DrawBuffers, Error, Flush, FlushDone, FlushRequest, Indev,
    IndevCell, IndevData, IndevKind, Lvgl, LvglAccess, Obj, Read, RenderMode, enums::EventCode,
    tick_inc, widgets::Button,
};
use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicBool, AtomicU32, Ordering::Relaxed},
};

static LOCK: Mutex<()> = Mutex::new(());

struct Guard(#[allow(dead_code)] MutexGuard<'static, ()>);

// SAFETY: a guard of the mutex serializing every LVGL call of this process.
unsafe impl LvglAccess for Guard {}

/// 240 × 160 at 32 bits per pixel at most.
const LINE: usize = 240 * 4;
// The whole screen at any color depth, so each refresh is one flush: the
// deferred flush below completes only after `run` returns, and LVGL would
// wait for it before flushing a second area of the same refresh.
static BUF_A: DrawBuf<{ LINE * 160 }> = DrawBuf::new();
static BUF_B: DrawBuf<{ LINE * 160 }> = DrawBuf::new();
static TINY: DrawBuf<8> = DrawBuf::new();

static FLUSHES: AtomicU32 = AtomicU32::new(0);
static DEFER: AtomicBool = AtomicBool::new(false);
static PENDING: Mutex<Option<FlushDone>> = Mutex::new(None);

struct Panel {
    width: usize,
}

impl Flush for Panel {
    fn flush(&mut self, mut req: FlushRequest<'_>) {
        let area = req.area();
        let rows = (area.y2 - area.y1 + 1) as usize;
        let cols = (area.x2 - area.x1 + 1) as usize;
        assert!(cols <= self.width);
        // Partial mode: exactly the rendered rows, readable and writable.
        let stride = req.stride();
        assert_eq!(req.pixels().len(), stride * rows);
        req.pixels()[0] ^= 0;
        FLUSHES.fetch_add(1, Relaxed);
        if DEFER.load(Relaxed) {
            *PENDING.lock().unwrap() = Some(req.defer());
        }
    }
}

static PANEL: DisplayCell<Panel> = DisplayCell::new();
static OTHER: DisplayCell<Panel> = DisplayCell::new();

/// A pointer that presses at (`x`, `y`) for the reads while `pressed`.
struct Touch;
static TOUCH_PRESSED: AtomicBool = AtomicBool::new(false);
static TOUCH_AT: Mutex<(i32, i32)> = Mutex::new((0, 0));

impl Read for Touch {
    fn read(&mut self, data: &mut IndevData<'_>) {
        let (x, y) = *TOUCH_AT.lock().unwrap();
        data.set_point(x, y);
        data.set_pressed(TOUCH_PRESSED.load(Relaxed));
    }
}

static TOUCH: IndevCell<Touch> = IndevCell::new();

fn run(lv: &mut Lvgl, ms: u32) {
    tick_inc(ms);
    lv.timer_handler();
}

#[test]
fn displays_and_input_devices() {
    let mut guard = Guard(LOCK.lock().unwrap());
    let mut lv = lvgl_rust::init(&mut guard).unwrap();

    // A buffer smaller than a line is refused, and leaves nothing claimed.
    assert_eq!(
        Display::create(
            &mut lv,
            &OTHER,
            Panel { width: 240 },
            240,
            160,
            DrawBuffers::single(&TINY),
            RenderMode::Partial
        )
        .err(),
        Some(Error::InvalidArg)
    );
    let display = Display::create(
        &mut lv,
        &PANEL,
        Panel { width: 240 },
        240,
        160,
        DrawBuffers::double(&BUF_A, &BUF_B),
        RenderMode::Partial,
    )
    .unwrap();
    // The cell and the buffers are claimed.
    assert_eq!(
        Display::create(
            &mut lv,
            &OTHER,
            Panel { width: 240 },
            240,
            160,
            DrawBuffers::single(&BUF_A),
            RenderMode::Partial
        )
        .err(),
        Some(Error::AlreadyOwned)
    );

    // Rendering flushes through the driver.
    static CLICKS: AtomicU32 = AtomicU32::new(0);
    Button::create(lv.screen_active().unwrap())
        .unwrap()
        .on(EventCode::CLICKED, |_| {
            CLICKS.fetch_add(1, Relaxed);
        })
        .unwrap();
    {
        let mut button = lv
            .edit(|lv| lv.screen_active()?.child(-1)?.downcast::<Button>())
            .unwrap();
        button.set_size(100, 50);
        button.set_pos(20, 30);
    }
    run(&mut lv, 40);
    assert!(FLUSHES.load(Relaxed) > 0);

    // A pointer clicks the button.
    let touch = Indev::create(&mut lv, &TOUCH, Touch, IndevKind::Pointer).unwrap();
    *TOUCH_AT.lock().unwrap() = (60, 50);
    TOUCH_PRESSED.store(true, Relaxed);
    run(&mut lv, 40);
    TOUCH_PRESSED.store(false, Relaxed);
    run(&mut lv, 40);
    assert_eq!(CLICKS.load(Relaxed), 1);
    touch.delete(&mut lv);

    // A deferred flush completes from another thread; until then the
    // display cannot be deleted.
    DEFER.store(true, Relaxed);
    lv.edit(|lv| lv.screen_active()?.child(0))
        .unwrap()
        .set_x(25);
    run(&mut lv, 40);
    DEFER.store(false, Relaxed);
    let done = PENDING.lock().unwrap().take();
    let display = match (done, display.try_delete(&mut lv)) {
        (Some(done), Err((display, Error::Busy))) => {
            std::thread::spawn(move || done.ready()).join().unwrap();
            display
        }
        (done, result) => panic!(
            "expected a pending flush: {:?}, {:?}",
            done.is_some(),
            result.err().map(|e| e.1)
        ),
    };
    display.try_delete(&mut lv).map_err(|e| e.1).unwrap();

    // The cell and the buffers can be used again.
    let display = Display::create(
        &mut lv,
        &PANEL,
        Panel { width: 240 },
        240,
        160,
        DrawBuffers::single(&BUF_A),
        RenderMode::Partial,
    )
    .unwrap();
    Obj::create(lv.screen_active().unwrap()).unwrap();
    run(&mut lv, 40);
    display.try_delete(&mut lv).map_err(|e| e.1).unwrap();
}
