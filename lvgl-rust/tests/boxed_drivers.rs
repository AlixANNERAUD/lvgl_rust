//! Display and input drivers on LVGL's heap (`--features heap`; their own
//! process: they create displays).
#![cfg(feature = "heap")]

use lvgl_rust::{
    Display, DrawBuf, DrawBuffers, Error, Flush, FlushDone, FlushRequest, Indev, IndevData,
    IndevKind, Lvgl, LvglAccess, Read, RenderMode, enums::EventCode, sys, tick_inc,
    widgets::Button,
};
use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicBool, AtomicU32, Ordering::Relaxed},
};

static LOCK: Mutex<()> = Mutex::new(());

struct Guard(#[allow(dead_code)] MutexGuard<'static, ()>);

// SAFETY: a guard of the mutex serializing every LVGL call of this process.
unsafe impl LvglAccess for Guard {}

// The whole screen at any color depth: each refresh is one flush, so the
// deferred flush below can complete after `run` returns.
const SCREEN: usize = 240 * 4 * 160;
static BUFFER: DrawBuf<SCREEN> = DrawBuf::new();

static FLUSHES: AtomicU32 = AtomicU32::new(0);
static DROPS: AtomicU32 = AtomicU32::new(0);
static DEFER: AtomicBool = AtomicBool::new(false);
static PENDING: Mutex<Option<FlushDone>> = Mutex::new(None);

/// A driver with state, alignment and a destructor, to check the block's
/// layout and that it is dropped exactly once.
#[repr(align(16))]
struct Panel {
    frames: u64,
    name: Vec<u8>,
}

impl Flush for Panel {
    fn flush(&mut self, mut req: FlushRequest<'_>) {
        assert_eq!(self.name, b"boxed");
        self.frames += 1;
        req.pixels()[0] ^= 0;
        FLUSHES.fetch_add(1, Relaxed);
        if DEFER.load(Relaxed) {
            *PENDING.lock().unwrap() = Some(req.defer());
        }
    }
}

impl Drop for Panel {
    fn drop(&mut self) {
        DROPS.fetch_add(1, Relaxed);
    }
}

static TOUCH_PRESSED: AtomicBool = AtomicBool::new(false);
static READER_DROPS: AtomicU32 = AtomicU32::new(0);

struct Touch {
    at: (i32, i32),
    _owned: Vec<u8>,
}

impl Read for Touch {
    fn read(&mut self, data: &mut IndevData<'_>) {
        data.set_point(self.at.0, self.at.1);
        data.set_pressed(TOUCH_PRESSED.load(Relaxed));
    }
}

impl Drop for Touch {
    fn drop(&mut self) {
        READER_DROPS.fetch_add(1, Relaxed);
    }
}

struct Keys;

impl Read for Keys {
    fn read(&mut self, data: &mut IndevData<'_>) {
        data.set_button(7);
        data.set_pressed(TOUCH_PRESSED.load(Relaxed));
    }
}

static POINTS: [sys::lv_point_t; 1] = [sys::lv_point_t { x: 60, y: 50 }];

fn run(lv: &mut Lvgl, ms: u32) {
    tick_inc(ms);
    lv.timer_handler();
}

fn panel() -> Panel {
    Panel {
        frames: 0,
        name: b"boxed".to_vec(),
    }
}

#[test]
fn boxed_drivers_live_until_deleted() {
    let mut guard = Guard(LOCK.lock().unwrap());
    let mut lv = lvgl_rust::init(&mut guard).unwrap();

    // A buffer too small is refused before anything is allocated.
    static TINY: DrawBuf<8> = DrawBuf::new();
    assert_eq!(
        Display::create_boxed(
            &mut lv,
            panel(),
            240,
            160,
            DrawBuffers::single(&TINY),
            RenderMode::Partial
        )
        .err(),
        Some(Error::InvalidArg)
    );
    // The driver given back by the refusal was dropped.
    assert_eq!(DROPS.load(Relaxed), 1);

    let display = Display::create_boxed(
        &mut lv,
        panel(),
        240,
        160,
        DrawBuffers::single(&BUFFER),
        RenderMode::Partial,
    )
    .unwrap();
    static CLICKS: AtomicU32 = AtomicU32::new(0);
    Button::create(&mut lv.screen_active_mut().unwrap())
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

    // A boxed pointer clicks the button, then is freed with its device.
    let touch = Indev::create_boxed(
        &mut lv,
        Touch {
            at: (60, 50),
            _owned: vec![1, 2, 3],
        },
        IndevKind::Pointer,
    )
    .unwrap();
    TOUCH_PRESSED.store(true, Relaxed);
    run(&mut lv, 40);
    TOUCH_PRESSED.store(false, Relaxed);
    run(&mut lv, 40);
    assert_eq!(CLICKS.load(Relaxed), 1);
    touch.delete(&mut lv);
    assert_eq!(READER_DROPS.load(Relaxed), 1);
    assert_eq!(
        Indev::create_boxed(&mut lv, Keys, IndevKind::Button).err(),
        Some(Error::InvalidArg)
    );
    // A boxed button device whose reader reports an id past its points.
    let keys = Indev::create_button_boxed(&mut lv, Keys, &POINTS).unwrap();
    TOUCH_PRESSED.store(true, Relaxed);
    run(&mut lv, 40);
    TOUCH_PRESSED.store(false, Relaxed);
    run(&mut lv, 40);
    assert_eq!(CLICKS.load(Relaxed), 1);
    keys.delete(&mut lv);

    // A deferred flush keeps the block alive: deletion is refused until it
    // completes, from another thread.
    DEFER.store(true, Relaxed);
    lv.edit(|lv| lv.screen_active()?.child(0))
        .unwrap()
        .set_x(25);
    run(&mut lv, 40);
    DEFER.store(false, Relaxed);
    let done = PENDING.lock().unwrap().take().expect("a deferred flush");
    let display = match display.try_delete(&mut lv) {
        Err((display, Error::Busy)) => display,
        other => panic!("expected Busy: {:?}", other.err().map(|e| e.1)),
    };
    assert_eq!(DROPS.load(Relaxed), 1);
    std::thread::spawn(move || done.ready()).join().unwrap();
    display.try_delete(&mut lv).map_err(|e| e.1).unwrap();
    assert_eq!(DROPS.load(Relaxed), 2);

    // Again, to reuse the buffer and free a second block.
    let display = Display::create_boxed(
        &mut lv,
        panel(),
        240,
        160,
        DrawBuffers::single(&BUFFER),
        RenderMode::Partial,
    )
    .unwrap();
    run(&mut lv, 40);
    display.try_delete(&mut lv).map_err(|e| e.1).unwrap();
    assert_eq!(DROPS.load(Relaxed), 3);
}
