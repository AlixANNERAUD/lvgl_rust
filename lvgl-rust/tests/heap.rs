//! Closures on LVGL's heap (`--features heap`).
#![cfg(all(feature = "heap", not(lvgl_rust_views_only)))]

use lvgl_rust::{Lvgl, LvglAccess, Obj, Session, enums::EventCode, sys, tick_inc};
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicU32, Ordering::Relaxed},
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
        unsafe { sys::lv_display_create(240, 160) };
        lv
    } else {
        Lvgl::enter(guard).unwrap()
    }
}

fn send(object: &Obj, code: EventCode) {
    // SAFETY: session held, live object; no param.
    unsafe { sys::lv_obj_send_event(object.as_ptr(), code.bits() as _, core::ptr::null_mut()) };
}

#[test]
fn boxed_handlers_live_until_their_object_is_deleted() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let counter = Arc::new(AtomicU32::new(0));
    let mut screen = lv.screen_active_mut().unwrap();
    let object = Obj::create(&mut screen).unwrap();
    let mine = counter.clone();
    object
        .on_boxed(EventCode::CLICKED, move |_| {
            mine.fetch_add(1, Relaxed);
        })
        .unwrap();
    // A more aligned capture than the heap guarantees.
    #[repr(align(32))]
    struct Aligned(u8);
    let aligned = Aligned(7);
    object
        .on_boxed(EventCode::CLICKED, move |_| {
            assert_eq!((&raw const aligned).addr() % 32, 0);
            assert_eq!(aligned.0, 7);
        })
        .unwrap();
    assert_eq!(Arc::strong_count(&counter), 2);
    send(object, EventCode::CLICKED);
    send(object, EventCode::PRESSED); // another code: not called
    assert_eq!(counter.load(Relaxed), 1);
    lv.edit(|lv| lv.screen_active()?.child(-1))
        .unwrap()
        .delete()
        .unwrap();
    // The closure was dropped with its object.
    assert_eq!(Arc::strong_count(&counter), 1);
}

#[test]
fn boxed_timers_drop_their_closure_with_the_timer() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let counter = Arc::new(AtomicU32::new(0));
    let mine = counter.clone();
    let timer = lv
        .timer_boxed(10, move |_| {
            mine.fetch_add(1, Relaxed);
        })
        .unwrap();
    tick_inc(10);
    lv.timer_handler();
    assert_eq!(counter.load(Relaxed), 1);
    assert_eq!(Arc::strong_count(&counter), 2);
    drop(timer);
    lv.timer_handler();
    assert_eq!(Arc::strong_count(&counter), 1);
}
