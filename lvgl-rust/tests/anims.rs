//! Animations against LVGL.

use lvgl_rust::{Anim, AnimPath, Lvgl, LvglAccess, Obj, Session, tick_inc};
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

fn run(lv: &mut Lvgl, ms: u32) {
    tick_inc(ms);
    lv.timer_handler();
}

#[test]
fn animations_call_back_with_values_and_complete() {
    static LAST: AtomicI32 = AtomicI32::new(-1);
    static TARGETED: AtomicU32 = AtomicU32::new(0);
    static DONE: AtomicU32 = AtomicU32::new(0);
    let mut guard = guard();
    let mut lv = session(&mut guard);
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    let anim = Anim::new(7u8)
        .values(0, 100)
        .duration(100)
        .path(AnimPath::Linear)
        .exec(|ctx, data, value| {
            assert_eq!(data, 7);
            if ctx.target().is_some() {
                TARGETED.fetch_add(1, Relaxed);
            }
            LAST.store(value, Relaxed);
        })
        .on_completed(|_, data| {
            DONE.fetch_add(u32::from(data), Relaxed);
        });
    lv.edit(last).unwrap().animate(anim).unwrap();
    run(&mut lv, 50);
    let middle = LAST.load(Relaxed);
    assert!((1..100).contains(&middle), "{middle}");
    run(&mut lv, 60);
    assert_eq!(LAST.load(Relaxed), 100);
    assert_eq!(DONE.load(Relaxed), 7);
    assert!(TARGETED.load(Relaxed) >= 2);
    lv.edit(last).unwrap().delete().unwrap();
}

#[test]
fn deleting_the_object_or_its_animations_stops_them() {
    static CALLS: AtomicU32 = AtomicU32::new(0);
    fn count(_: &lvgl_rust::AnimCtx<'_>, _: (), _: i32) {
        CALLS.fetch_add(1, Relaxed);
    }
    let mut guard = guard();
    let mut lv = session(&mut guard);
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    let forever = || Anim::new(()).duration(100).repeat(None).exec(count);
    lv.edit(last).unwrap().animate(forever()).unwrap();
    run(&mut lv, 30);
    assert!(lv.edit(last).unwrap().delete_animations());
    let calls = CALLS.load(Relaxed);
    run(&mut lv, 50);
    assert_eq!(CALLS.load(Relaxed), calls);
    // Animations add up: delete them first to replace.
    let calls_over = |lv: &mut Lvgl| {
        let before = CALLS.load(Relaxed);
        for _ in 0..6 {
            run(lv, 34);
        }
        CALLS.load(Relaxed) - before
    };
    lv.edit(last).unwrap().animate(forever()).unwrap();
    let single = calls_over(&mut lv);
    assert!(single > 0);
    lv.edit(last).unwrap().animate(forever()).unwrap();
    assert_eq!(calls_over(&mut lv), 2 * single);
    {
        let mut object = lv.edit(last).unwrap();
        object.delete_animations();
        object.animate(forever()).unwrap();
    }
    assert_eq!(calls_over(&mut lv), single);
    // Deleting the object deletes its animations (ASan: no callback after).
    lv.edit(last).unwrap().delete().unwrap();
    let calls = CALLS.load(Relaxed);
    run(&mut lv, 100);
    assert_eq!(CALLS.load(Relaxed), calls);
}
