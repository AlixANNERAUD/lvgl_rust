//! Timers against LVGL.
#![cfg(not(lvgl_rust_views_only))]

use lvgl_rust::{Error, Handle, Lvgl, LvglAccess, Obj, Session, Timer, TimerCtx, tick_inc};
use std::sync::{
    Mutex, MutexGuard,
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
        unsafe { lvgl_rust::sys::lv_display_create(240, 160) };
        lv
    } else {
        Lvgl::enter(guard).unwrap()
    }
}

/// Lets `ms` pass and runs the timers.
fn run(lv: &mut Lvgl, ms: u32) {
    tick_inc(ms);
    lv.timer_handler();
}

#[test]
fn timers_run_with_their_data_until_dropped() {
    static TICKS: AtomicU32 = AtomicU32::new(0);
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let timer = lv
        .timer(100, 3u32, |_, step| {
            TICKS.fetch_add(step, Relaxed);
        })
        .unwrap();
    run(&mut lv, 50);
    assert_eq!(TICKS.load(Relaxed), 0);
    run(&mut lv, 60);
    assert_eq!(TICKS.load(Relaxed), 3);
    let handle = timer.handle();
    handle.pause(&lv).unwrap();
    run(&mut lv, 200);
    assert_eq!(TICKS.load(Relaxed), 3);
    handle.resume(&lv).unwrap();
    handle.ready(&lv).unwrap();
    run(&mut lv, 1);
    assert_eq!(TICKS.load(Relaxed), 6);
    // Dropping the owner deletes the timer at the drain.
    drop(timer);
    run(&mut lv, 200);
    assert_eq!(TICKS.load(Relaxed), 6);
    assert_eq!(handle.pause(&lv), Err(Error::Dead));
}

#[test]
fn a_finished_repeat_count_pauses_instead_of_freeing() {
    static TICKS: AtomicU32 = AtomicU32::new(0);
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let timer = lv
        .timer(10, (), |_, ()| {
            TICKS.fetch_add(1, Relaxed);
        })
        .unwrap();
    timer.handle().set_repeat_count(&lv, 2).unwrap();
    for _ in 0..5 {
        run(&mut lv, 10);
    }
    assert_eq!(TICKS.load(Relaxed), 2);
    // Still owned and alive: LVGL paused it instead of deleting it.
    timer.handle().set_repeat_count(&lv, 1).unwrap();
    timer.handle().resume(&lv).unwrap();
    run(&mut lv, 10);
    assert_eq!(TICKS.load(Relaxed), 3);
    timer.delete(&mut lv).unwrap();
}

#[test]
fn timer_functions_find_their_handle_and_defer_deletions() {
    static OWN: Mutex<Option<Handle<Timer>>> = Mutex::new(None);
    static FOUND: AtomicU32 = AtomicU32::new(0);
    let mut guard = guard();
    let mut lv = session(&mut guard);
    fn tick(ctx: &TimerCtx<'_>, _: ()) {
        if ctx.handle() == *OWN.lock().unwrap() {
            FOUND.fetch_add(1, Relaxed);
        }
        // Shared access: ask for deletions (creating objects is exclusive,
        // posted with `post`).
        let screen = ctx.lv().screen_active().unwrap();
        screen.child(-1).unwrap().delete_later().unwrap();
        ctx.delete_later();
    }
    let children = lv.screen_active().unwrap().get_child_count();
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    let timer = lv.timer(10, (), tick).unwrap();
    *OWN.lock().unwrap() = Some(timer.handle());
    run(&mut lv, 10);
    assert_eq!(FOUND.load(Relaxed), 1);
    // The object and the timer were deleted at the drain.
    assert_eq!(lv.screen_active().unwrap().get_child_count(), children);
    assert_eq!(timer.handle().reset(&lv), Err(Error::Dead));
    run(&mut lv, 10);
    assert_eq!(FOUND.load(Relaxed), 1);
    // The owner releases the dead slot.
    drop(timer);
    run(&mut lv, 1);
}
