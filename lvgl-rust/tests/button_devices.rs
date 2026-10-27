//! Button input devices: hardware buttons pressing points of the screen
//! (their own process: they create the display).

use lvgl_rust::{
    Display, DisplayCell, DrawBuf, DrawBuffers, Error, Flush, FlushRequest, Indev, IndevCell,
    IndevData, IndevKind, Lvgl, LvglAccess, Read, RenderMode, enums::EventCode, sys, tick_inc,
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

struct Panel;

impl Flush for Panel {
    fn flush(&mut self, _: FlushRequest<'_>) {}
}

static BUFFER: DrawBuf<{ 240 * 4 * 10 }> = DrawBuf::new();
static PANEL: DisplayCell<Panel> = DisplayCell::new();

/// Reports `BUTTON` pressed while `PRESSED`.
struct Keys;
static BUTTON: AtomicU32 = AtomicU32::new(0);
static PRESSED: AtomicBool = AtomicBool::new(false);

impl Read for Keys {
    fn read(&mut self, data: &mut IndevData<'_>) {
        data.set_button(BUTTON.load(Relaxed));
        data.set_pressed(PRESSED.load(Relaxed));
    }
}

static KEYS: IndevCell<Keys> = IndevCell::new();
static OTHER: IndevCell<Keys> = IndevCell::new();

/// Button 0 presses inside the button below, button 1 outside it.
static POINTS: [sys::lv_point_t; 2] = [
    sys::lv_point_t { x: 60, y: 50 },
    sys::lv_point_t { x: 200, y: 140 },
];

fn run(lv: &mut Lvgl, ms: u32) {
    tick_inc(ms);
    lv.timer_handler();
}

fn press(lv: &mut Lvgl, button: u32) {
    BUTTON.store(button, Relaxed);
    PRESSED.store(true, Relaxed);
    run(lv, 40);
    PRESSED.store(false, Relaxed);
    run(lv, 40);
}

#[test]
fn buttons_press_their_points() {
    let mut guard = Guard(LOCK.lock().unwrap());
    let mut lv = lvgl_rust::init(&mut guard).unwrap();
    let display = Display::create(
        &mut lv,
        &PANEL,
        Panel,
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

    // A button device needs its points.
    assert_eq!(
        Indev::create(&mut lv, &OTHER, Keys, IndevKind::Button).err(),
        Some(Error::InvalidArg)
    );

    let keys = Indev::create_button(&mut lv, &KEYS, Keys, &POINTS).unwrap();
    press(&mut lv, 0);
    assert_eq!(CLICKS.load(Relaxed), 1);
    // Outside the button.
    press(&mut lv, 1);
    assert_eq!(CLICKS.load(Relaxed), 1);
    // An id past the points reads as released: no press, no read past
    // the array.
    press(&mut lv, 2);
    press(&mut lv, u32::MAX);
    assert_eq!(CLICKS.load(Relaxed), 1);
    press(&mut lv, 0);
    assert_eq!(CLICKS.load(Relaxed), 2);
    // The cell is claimed until the device is deleted.
    assert_eq!(
        Indev::create_button(&mut lv, &KEYS, Keys, &POINTS).err(),
        Some(Error::AlreadyOwned)
    );
    keys.delete(&mut lv);
    let keys = Indev::create_button(&mut lv, &KEYS, Keys, &POINTS).unwrap();
    press(&mut lv, 0);
    assert_eq!(CLICKS.load(Relaxed), 3);
    keys.delete(&mut lv);

    display.try_delete(&mut lv).map_err(|e| e.1).unwrap();
}
