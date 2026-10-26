//! The diagnostic counters (feature `debug-counters`), in their own process:
//! the counters are global and the test creates displays.
#![cfg(feature = "debug-counters")]

use lvgl_rust::{
    DebugCounters, Display, DisplayCell, DrawBuf, DrawBuffers, Flush, FlushRequest, Indev,
    IndevCell, IndevData, IndevKind, Lvgl, LvglAccess, Obj, Read, RenderMode, ScreenAnim,
    WidgetExt, sys,
};
use std::ptr::NonNull;
use std::sync::{Mutex, MutexGuard};

static LOCK: Mutex<()> = Mutex::new(());

struct Guard(#[allow(dead_code)] MutexGuard<'static, ()>);

// SAFETY: a guard of the mutex serializing every LVGL call of this process.
unsafe impl LvglAccess for Guard {}

struct Panel;

impl Flush for Panel {
    fn flush(&mut self, _: FlushRequest<'_>) {}
}

struct Keys;

impl Read for Keys {
    fn read(&mut self, _: &mut IndevData<'_>) {}
}

static BUF: DrawBuf<{ 64 * 4 }> = DrawBuf::new();
static KEPT: DrawBuf<{ 64 * 4 }> = DrawBuf::new();
static PANEL: DisplayCell<Panel> = DisplayCell::new();
static LEAKED: DisplayCell<Panel> = DisplayCell::new();
static KEYS: IndevCell<Keys> = IndevCell::new();
static LEAKED_KEYS: IndevCell<Keys> = IndevCell::new();

fn display(
    lv: &mut Lvgl,
    cell: &'static DisplayCell<Panel>,
    buf: &'static DrawBuf<256>,
) -> Display {
    Display::create(
        lv,
        cell,
        Panel,
        64,
        64,
        DrawBuffers::single(buf),
        RenderMode::Partial,
    )
    .unwrap()
}

#[test]
fn counters_report_drains_refusals_skipped_callbacks_and_leaks() {
    let mut guard = Guard(LOCK.lock().unwrap());
    let mut lv = lvgl_rust::init(&mut guard).unwrap();
    let main = display(&mut lv, &PANEL, &BUF);
    DebugCounters::reset();
    assert_eq!(DebugCounters::read(), DebugCounters::default());

    // A dropped `Owned` and a `delete_later` are carried out by the drain.
    let owned = Obj::create_owned(lv.screen_active().unwrap()).unwrap();
    drop(owned);
    Obj::create(lv.screen_active().unwrap())
        .unwrap()
        .delete_later()
        .unwrap();
    lv.timer_handler();
    let counters = DebugCounters::read();
    assert_eq!((counters.drained, counters.refused), (2, 0));

    // A deferred load of a display layer: refused when the drain applies it.
    // SAFETY: session held; the top layer lives as long as the display.
    let layer = unsafe {
        Obj::from_raw(&lv, NonNull::new(sys::lv_layer_top()).unwrap())
            .handle()
            .unwrap()
    };
    lv.load_screen_later(layer, ScreenAnim::NONE).unwrap();
    lv.timer_handler();
    let counters = DebugCounters::read();
    assert_eq!((counters.drained, counters.refused), (2, 1));

    // Callbacks past `LVGL_RUST_MAX_CALLBACK_DEPTH` are skipped.
    let mut levels = Vec::new();
    // SAFETY: test-only nesting, no LVGL call made through the guards.
    while let Some(level) = unsafe { Lvgl::enter_callback() } {
        levels.push(level);
    }
    drop(levels);
    assert_eq!(DebugCounters::read().skipped, 1);

    // Deleted drivers are not leaks; dropped ones are, on any thread.
    let keys = Indev::create(&mut lv, &KEYS, Keys, IndevKind::Keypad).unwrap();
    keys.delete(&mut lv);
    let kept = display(&mut lv, &LEAKED, &KEPT);
    let kept_keys = Indev::create(&mut lv, &LEAKED_KEYS, Keys, IndevKind::Keypad).unwrap();
    assert_eq!(DebugCounters::read().leaked, 0);
    drop(kept_keys);
    std::thread::spawn(move || drop(kept)).join().unwrap();
    assert_eq!(DebugCounters::read().leaked, 2);
    main.try_delete(&mut lv).map_err(|e| e.1).unwrap();
    assert_eq!(DebugCounters::read().leaked, 2);

    DebugCounters::reset();
    assert_eq!(DebugCounters::read(), DebugCounters::default());
}

/// Every public plain type logs with defmt (checked at compile time: a
/// host has no defmt logger).
#[cfg(feature = "defmt")]
#[test]
fn public_types_implement_defmt_format() {
    fn format<T: defmt::Format>() {}
    format::<lvgl_rust::Error>();
    format::<DebugCounters>();
    format::<lvgl_rust::Handle<Obj>>();
    format::<lvgl_rust::Owned<Obj>>();
    format::<lvgl_rust::Selector>();
    format::<lvgl_rust::StyleProp>();
    format::<lvgl_rust::ScreenAnim>();
    format::<lvgl_rust::enums::EventCode>();
    format::<lvgl_rust::enums::SafeEventCode>();
    format::<IndevKind>();
    format::<RenderMode>();
    format::<lvgl_rust::PixelFormat>();
}
