//! Subjects, observers and bindings against LVGL.

use lvgl_rust::{
    ColorSubject, IntSubject, Lvgl, LvglAccess, Obj, Session, color,
    enums::State,
    widgets::{Label, Slider},
};
use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicI32, AtomicU32, Ordering::Relaxed},
};

static LOCK: Mutex<bool> = Mutex::new(false);

struct Guard(MutexGuard<'static, bool>);

// SAFETY: a guard of the mutex serializing every test's LVGL calls.
unsafe impl LvglAccess for Guard {}

fn guard() -> Guard {
    Guard(LOCK.lock().unwrap_or_else(|p| p.into_inner()))
}

fn session(guard: &mut Guard) -> Session<'_> {
    if !*guard.0 {
        *guard.0 = true;
        let lv = lvgl_rust::init(guard).unwrap();
        // SAFETY: session held.
        unsafe { lvgl_rust::sys::lv_display_create(240, 160) };
        lv
    } else {
        Lvgl::enter(guard).unwrap()
    }
}

fn last(lv: &Lvgl) -> Option<&Obj> {
    lv.screen_active()?.child(-1)
}

#[test]
fn subjects_initialise_on_first_use_and_notify() {
    static COUNT: IntSubject = IntSubject::new(5);
    static SEEN: AtomicI32 = AtomicI32::new(0);
    static CALLS: AtomicU32 = AtomicU32::new(0);
    let mut guard = guard();
    let mut lv = session(&mut guard);
    assert_eq!(COUNT.get(&lv), 5);
    // Called once right away, then on every change.
    COUNT
        .observe(&lv, 2i32, |ctx, factor, value| {
            assert!(ctx.target().is_none());
            SEEN.store(value * factor, Relaxed);
            CALLS.fetch_add(1, Relaxed);
        })
        .unwrap();
    assert_eq!((SEEN.load(Relaxed), CALLS.load(Relaxed)), (10, 1));
    COUNT.set(&mut lv, 7);
    assert_eq!((SEEN.load(Relaxed), CALLS.load(Relaxed)), (14, 2));
    assert_eq!((COUNT.get(&lv), COUNT.previous(&lv)), (7, 5));
}

#[test]
fn object_observers_go_with_their_object() {
    static LEVEL: IntSubject = IntSubject::new(0);
    static CALLS: AtomicU32 = AtomicU32::new(0);
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let mut screen = lv.screen_active_mut().unwrap();
    let object = Obj::create(&mut screen).unwrap();
    LEVEL
        .observe_obj(object, (), |ctx, (), _| {
            assert!(ctx.target().is_some());
            CALLS.fetch_add(1, Relaxed);
        })
        .unwrap();
    LEVEL.set(&mut lv, 1);
    assert_eq!(CALLS.load(Relaxed), 2);
    lv.edit(last).unwrap().delete().unwrap();
    // ASan: no call into the deleted object's observer.
    LEVEL.set(&mut lv, 2);
    assert_eq!(CALLS.load(Relaxed), 2);
}

#[test]
fn widgets_bind_to_int_subjects() {
    static VALUE: IntSubject = IntSubject::new(30);
    static CHECKED: IntSubject = IntSubject::new(0);
    let mut guard = guard();
    let mut lv = session(&mut guard);
    fn last_slider(lv: &Lvgl) -> Option<&Slider> {
        last(lv)?.downcast()
    }
    Slider::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    lv.edit(last_slider).unwrap().bind_value(&VALUE).unwrap();
    assert_eq!(last_slider(&lv).unwrap().get_value(), 30);
    VALUE.set(&mut lv, 60);
    assert_eq!(last_slider(&lv).unwrap().get_value(), 60);
    lv.edit(last).unwrap().bind_checked(&CHECKED).unwrap();
    CHECKED.set(&mut lv, 1);
    assert!(last(&lv).unwrap().has_state(State::CHECKED));
    lv.edit(last).unwrap().delete().unwrap();

    fn last_label(lv: &Lvgl) -> Option<&Label> {
        last(lv)?.downcast()
    }
    Label::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    lv.edit(last_label).unwrap().bind_text(&VALUE).unwrap();
    let mut buf = [0u8; 8];
    assert_eq!(
        last_label(&lv).unwrap().get_text_into(&mut buf),
        Ok(Some(c"60"))
    );
    VALUE.set(&mut lv, -4);
    assert_eq!(
        last_label(&lv).unwrap().get_text_into(&mut buf),
        Ok(Some(c"-4"))
    );
    lv.edit(last).unwrap().delete().unwrap();
}

#[test]
fn color_subjects_notify_with_colors() {
    static ACCENT: ColorSubject = ColorSubject::new(color::rgb(1, 2, 3));
    static RED: AtomicU32 = AtomicU32::new(0);
    let mut guard = guard();
    let mut lv = session(&mut guard);
    ACCENT
        .observe(&lv, (), |_, (), c| RED.store(u32::from(c.red), Relaxed))
        .unwrap();
    assert_eq!(RED.load(Relaxed), 1);
    ACCENT.set(&mut lv, color::hex(0xff0000));
    assert_eq!(RED.load(Relaxed), 0xff);
    assert_eq!(ACCENT.previous(&lv).green, 2);
}
