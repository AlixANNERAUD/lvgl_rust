//! Focus groups against LVGL.
#![cfg(not(lvgl_rust_views_only))]

use lvgl_rust::{
    Error, Indev, IndevCell, IndevData, IndevKind, Lvgl, LvglAccess, Obj, Read, WidgetExt,
    enums::EventCode, tick_inc, widgets::Button,
};
use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicBool, AtomicU32, Ordering::Relaxed},
};

static LOCK: Mutex<()> = Mutex::new(());

struct Guard(#[allow(dead_code)] MutexGuard<'static, ()>);

// SAFETY: a guard of the mutex serializing every LVGL call of this process.
unsafe impl LvglAccess for Guard {}

/// A keypad pressing Enter while `PRESSED` is set.
struct Keys;
static PRESSED: AtomicBool = AtomicBool::new(false);

impl Read for Keys {
    fn read(&mut self, data: &mut IndevData<'_>) {
        data.set_key(lvgl_rust::sys::lv_key_t_LV_KEY_ENTER);
        data.set_pressed(PRESSED.load(Relaxed));
    }
}

static KEYS: IndevCell<Keys> = IndevCell::new();

fn run(lv: &mut Lvgl, ms: u32) {
    tick_inc(ms);
    lv.timer_handler();
}

#[test]
fn groups_focus_their_objects_and_take_keys() {
    static KEYS_SEEN: AtomicU32 = AtomicU32::new(0);
    let mut guard = Guard(LOCK.lock().unwrap());
    let mut lv = lvgl_rust::init(&mut guard).unwrap();
    // SAFETY: session held.
    unsafe { lvgl_rust::sys::lv_display_create(240, 160) };
    let mut screen = lv.screen_active_mut().unwrap();
    let first = Button::create(&mut screen)
        .unwrap()
        .into_view()
        .handle()
        .unwrap();
    let second = Button::create(&mut screen).unwrap().into_view();
    second
        .on(EventCode::KEY, |e| {
            if e.key() == Some(lvgl_rust::sys::lv_key_t_LV_KEY_ENTER) {
                KEYS_SEEN.fetch_add(1, Relaxed);
            }
        })
        .unwrap();
    let second = second.handle().unwrap();

    let group = lv.create_group().unwrap();
    let g = group.handle();
    g.add_obj(&mut lv, first.upcast()).unwrap();
    g.add_obj(&mut lv, second.upcast()).unwrap();
    assert_eq!(g.obj_count(&lv), Ok(2));
    let focused = |lv: &Lvgl| g.focused(lv).unwrap().map(Obj::as_ptr);
    assert_eq!(focused(&lv), Some(first.get(&lv).unwrap().as_ptr()));
    g.focus_next(&mut lv).unwrap();
    assert_eq!(focused(&lv), Some(second.get(&lv).unwrap().as_ptr()));

    // A keypad sends its keys to the focused object.
    let mut keypad = Indev::create(&mut lv, &KEYS, Keys, IndevKind::Keypad).unwrap();
    keypad.set_group(&mut lv, Some(g)).unwrap();
    PRESSED.store(true, Relaxed);
    run(&mut lv, 40);
    PRESSED.store(false, Relaxed);
    run(&mut lv, 40);
    assert!(KEYS_SEEN.load(Relaxed) >= 1);

    // An object leaves its group.
    second
        .upcast::<Obj>()
        .get_mut(&mut lv)
        .unwrap()
        .remove_from_group();
    assert_eq!(g.obj_count(&lv), Ok(1));

    // Dropping the group deletes it at the drain: the keypad and the first
    // button are detached, the handle reports it dead.
    drop(group);
    run(&mut lv, 1);
    assert_eq!(g.obj_count(&lv), Err(Error::Dead));
    assert!(first.get(&lv).is_ok());
    PRESSED.store(true, Relaxed);
    run(&mut lv, 40);
    PRESSED.store(false, Relaxed);
    run(&mut lv, 40);
    keypad.delete(&mut lv);
}
