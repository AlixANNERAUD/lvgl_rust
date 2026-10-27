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

/// DELETE handler sending READY to the object's parent, which trickles it
/// back down to the object while LVGL still holds its handlers (safe code
/// can do the same: `set_event_trickle(true)` on the parent, then
/// `Obj::send_event(SafeEventCode::READY)` through a handle).
unsafe extern "C" fn send_to_parent(e: *mut sys::lv_event_t) {
    // SAFETY: a live event of a live object, under the session.
    unsafe {
        let object = sys::lv_event_get_current_target(e).cast::<sys::lv_obj_t>();
        let parent = sys::lv_obj_get_parent(object);
        sys::lv_obj_send_event(
            parent,
            sys::lv_event_code_t_LV_EVENT_READY,
            core::ptr::null_mut(),
        );
    }
}

#[test]
fn boxed_handlers_ignore_events_trickled_after_delete() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let parent = Obj::create(lv.screen_active().unwrap()).unwrap();
    // SAFETY: session held, live object.
    unsafe {
        sys::lv_obj_add_flag(
            parent.as_ptr(),
            sys::lv_obj_flag_t_LV_OBJ_FLAG_EVENT_TRICKLE,
        )
    };
    let child = Obj::create(parent).unwrap();
    let counter = Arc::new(AtomicU32::new(0));
    let mine = counter.clone();
    child
        .on_boxed(EventCode::READY, move |_| {
            mine.fetch_add(1, Relaxed);
        })
        .unwrap();
    // SAFETY: session held, live object; the callback needs no data.
    unsafe {
        sys::lv_obj_add_event_cb(
            child.as_ptr(),
            Some(send_to_parent),
            sys::lv_event_code_t_LV_EVENT_DELETE,
            core::ptr::null_mut(),
        )
    };
    send(parent, EventCode::READY);
    assert_eq!(counter.load(Relaxed), 1);
    lv.edit(|lv| lv.screen_active()?.child(-1)?.child(0))
        .unwrap()
        .delete()
        .unwrap();
    // The closure was dropped at DELETE and not called again; its block
    // waited for the drain (AddressSanitizer checks the order).
    assert_eq!(counter.load(Relaxed), 1);
    assert_eq!(Arc::strong_count(&counter), 1);
    lv.edit(|lv| lv.screen_active()?.child(-1))
        .unwrap()
        .delete()
        .unwrap();
}
