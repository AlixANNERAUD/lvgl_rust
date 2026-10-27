//! The diagnostic counters (feature `debug-counters`), in their own process:
//! the counters are global and the test creates displays.
#![cfg(feature = "debug-counters")]

use lvgl_rust::{
    DebugCounters, Display, DisplayCell, DrawBuf, DrawBuffers, Flush, FlushRequest, Indev,
    IndevCell, IndevData, IndevKind, Lvgl, LvglAccess, Read, RenderMode,
};
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

    #[cfg(not(lvgl_rust_views_only))]
    deferred_requests(&mut lv);
    // Without handles nothing is deferred.
    #[cfg(lvgl_rust_views_only)]
    lv.timer_handler();
    let requests = DebugCounters::read();
    let expected = if cfg!(lvgl_rust_views_only) {
        (0, 0)
    } else {
        (8, 4)
    };
    assert_eq!((requests.drained, requests.refused), expected);

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
    let counters = DebugCounters::read();
    assert_eq!(
        (counters.drained, counters.refused, counters.leaked),
        (requests.drained, requests.refused, 2)
    );

    DebugCounters::reset();
    assert_eq!(DebugCounters::read(), DebugCounters::default());
}

/// Every kind of deferred request, carried out, refused for good, retried
/// or made moot by a deletion: 8 drained and 4 refused in all.
#[cfg(not(lvgl_rust_views_only))]
fn deferred_requests(lv: &mut Lvgl) {
    use lvgl_rust::{Obj, ScreenAnim, ScreenLoadAnim, WidgetExt, sys, tick_inc};
    use std::ptr::NonNull;

    let counts = || {
        let counters = DebugCounters::read();
        (counters.drained, counters.refused)
    };
    let screen = lv.screen_active().unwrap().handle().unwrap();

    // A dropped `Owned`, a `delete_later` and a `clean_later` are carried
    // out by the drain; `Owned::delete` and a dropped group count too.
    let owned = Obj::create_owned(screen.get(lv).unwrap()).unwrap();
    drop(owned);
    Obj::create(screen.get(lv).unwrap())
        .unwrap()
        .delete_later()
        .unwrap();
    let parent = Obj::create(screen.get(lv).unwrap()).unwrap();
    Obj::create(parent).unwrap();
    parent.clean_later().unwrap();
    drop(lv.create_group().unwrap());
    assert_eq!(counts(), (0, 0));
    lv.timer_handler();
    assert_eq!(counts(), (4, 0));
    Obj::create_owned(screen.get(lv).unwrap())
        .unwrap()
        .delete(lv)
        .unwrap();
    assert_eq!(counts(), (5, 0));

    // Requests for objects deleted through their parent before the drain
    // are not counted.
    let parent = Obj::create(screen.get(lv).unwrap()).unwrap();
    let owned = Obj::create_owned(parent).unwrap();
    Obj::create(parent).unwrap().delete_later().unwrap();
    let parent = parent.handle().unwrap();
    parent.get_mut(lv).unwrap().delete().unwrap();
    drop(owned);
    lv.timer_handler();
    assert_eq!(counts(), (5, 0));

    // Requests that become permanently refused before the drain (the
    // objects are moved, by raw code, into a widget the part tables do not
    // describe) are dropped and counted as refused; the objects stay.
    // SAFETY: session held; the menu is deleted below, with its children.
    let menu = unsafe { sys::lv_menu_create(screen.get(lv).unwrap().as_ptr()) };
    let owned = Obj::create_owned(screen.get(lv).unwrap()).unwrap();
    let dropped = owned.get(lv).unwrap().as_ptr();
    let later = Obj::create(screen.get(lv).unwrap()).unwrap();
    later.delete_later().unwrap();
    let later = later.as_ptr();
    let cleaned = Obj::create(screen.get(lv).unwrap()).unwrap();
    cleaned.clean_later().unwrap();
    let cleaned = cleaned.as_ptr();
    // SAFETY: session held; the menu and the three objects are live.
    let internal = unsafe { sys::lv_obj_get_child_count(menu) };
    // SAFETY: as above.
    unsafe {
        sys::lv_obj_create(cleaned);
        for o in [dropped, later, cleaned] {
            sys::lv_obj_set_parent(o, menu);
        }
    }
    drop(owned);
    lv.timer_handler();
    assert_eq!(counts(), (5, 3));
    // SAFETY: session held; the refused objects are still the menu's.
    unsafe {
        assert_eq!(sys::lv_obj_get_child_count(menu), internal + 3);
        assert_eq!(sys::lv_obj_get_child_count(cleaned), 1);
        sys::lv_obj_delete(menu);
    }

    // A deferred load of a display layer: refused when the drain applies it.
    // SAFETY: session held; the top layer lives as long as the display.
    let layer = unsafe {
        Obj::from_raw(lv, NonNull::new(sys::lv_layer_top()).unwrap())
            .handle()
            .unwrap()
    };
    lv.load_screen_later(layer, ScreenAnim::NONE).unwrap();
    lv.timer_handler();
    assert_eq!(counts(), (5, 4));

    // A deletion of the previous screen of an animated load is retried
    // (`Busy`), uncounted, until the animation completes.
    let first = lv.create_screen().unwrap();
    let second = lv.create_screen().unwrap();
    lv.load_screen(first.handle(), ScreenAnim::NONE).unwrap();
    let slide = ScreenAnim::new(ScreenLoadAnim::MOVE_LEFT, 1000, 0, false);
    lv.load_screen(second.handle(), slide).unwrap();
    for _ in 0..10 {
        tick_inc(10);
        lv.timer_handler();
    }
    // SAFETY: session held.
    let prev = unsafe { sys::lv_display_get_screen_prev(core::ptr::null_mut()) };
    assert_eq!(prev, first.get(lv).unwrap().as_ptr());
    first.get(lv).unwrap().delete_later().unwrap();
    for _ in 0..3 {
        tick_inc(10);
        lv.timer_handler();
    }
    assert_eq!(counts(), (5, 4));
    for _ in 0..30 {
        tick_inc(100);
        lv.timer_handler();
    }
    assert_eq!(counts(), (6, 4));
    assert!(first.get(lv).is_err());
    // Dropping the owner of the deleted screen counts nothing.
    drop(first);
    lv.timer_handler();
    assert_eq!(counts(), (6, 4));

    // The loads: `load_screen` is immediate and not counted, a
    // `load_screen_later` is counted when the drain applies it.
    lv.load_screen_later(screen, ScreenAnim::NONE).unwrap();
    lv.timer_handler();
    assert_eq!(counts(), (7, 4));
    second.delete(lv).unwrap();
    assert_eq!(counts(), (8, 4));
}

/// Every public plain type logs with defmt (checked at compile time: a
/// host has no defmt logger).
#[cfg(feature = "defmt")]
#[test]
fn public_types_implement_defmt_format() {
    fn format<T: defmt::Format>() {}
    format::<lvgl_rust::Error>();
    format::<DebugCounters>();
    #[cfg(not(lvgl_rust_views_only))]
    {
        format::<lvgl_rust::Handle<lvgl_rust::Obj>>();
        format::<lvgl_rust::Owned<lvgl_rust::Obj>>();
    }
    format::<lvgl_rust::Selector>();
    format::<lvgl_rust::StyleProp>();
    format::<lvgl_rust::ScreenAnim>();
    format::<lvgl_rust::enums::EventCode>();
    format::<lvgl_rust::enums::SafeEventCode>();
    format::<IndevKind>();
    format::<RenderMode>();
    format::<lvgl_rust::PixelFormat>();
}
