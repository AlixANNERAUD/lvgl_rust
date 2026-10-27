//! Event handlers against LVGL.

use lvgl_rust::{
    Error, Event, Handler, Lvgl, LvglAccess, Obj, Session, enums::EventCode, handler_allowed, sys,
};
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

#[repr(C, align(64))]
struct Buffer([u8; 240 * 20 * 4]);
static mut BUFFER: Buffer = Buffer([0; 240 * 20 * 4]);

unsafe extern "C" fn flush(display: *mut sys::lv_display_t, _: *const sys::lv_area_t, _: *mut u8) {
    // SAFETY: called by LVGL for this display.
    unsafe { sys::lv_display_flush_ready(display) };
}

fn session(guard: &mut Guard) -> Session<'_> {
    if !*guard.0 {
        *guard.0 = true;
        let lv = lvgl_rust::init(guard).unwrap();
        // SAFETY: session held; the buffer is static and only LVGL uses it.
        unsafe {
            let display = sys::lv_display_create(240, 160);
            sys::lv_display_set_flush_cb(display, Some(flush));
            sys::lv_display_set_buffers(
                display,
                core::ptr::addr_of_mut!(BUFFER).cast(),
                core::ptr::null_mut(),
                core::mem::size_of::<Buffer>() as u32,
                sys::lv_display_render_mode_t_LV_DISPLAY_RENDER_MODE_PARTIAL,
            );
        }
        lv
    } else {
        Lvgl::enter(guard).unwrap()
    }
}

fn last(lv: &Lvgl) -> Option<&Obj> {
    lv.screen_active()?.child(-1)
}

/// Sends `code` with a NULL param, as safe sending will.
fn send(object: &Obj, code: EventCode) {
    // SAFETY: session held, live object; these codes take no param.
    unsafe { sys::lv_obj_send_event(object.as_ptr(), code.bits() as _, core::ptr::null_mut()) };
}

static CALLS: AtomicU32 = AtomicU32::new(0);

fn count(_: &Event<'_>) {
    CALLS.fetch_add(1, Relaxed);
}

static STATIC_HANDLER: Handler<AtomicU32> = Handler {
    ctx: AtomicU32::new(0),
    f: |_, ctx| {
        ctx.fetch_add(1, Relaxed);
    },
};

#[test]
fn handlers_of_every_kind_run_for_their_code() {
    static WITH: AtomicU32 = AtomicU32::new(0);
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let mut screen = lv.screen_active_mut().unwrap();
    let object = Obj::create(&mut screen).unwrap().into_view();
    CALLS.store(0, Relaxed);
    object.on_fn(EventCode::CLICKED, count).unwrap();
    object
        .on(EventCode::CLICKED, |e| {
            assert_eq!(e.code(), EventCode::CLICKED);
            assert!(e.target().is_some());
            CALLS.fetch_add(10, Relaxed);
        })
        .unwrap();
    object
        .on_with(EventCode::VALUE_CHANGED, 7u32, |_, d| {
            WITH.fetch_add(d, Relaxed);
        })
        .unwrap();
    object
        .on_static(EventCode::CLICKED, &STATIC_HANDLER)
        .unwrap();
    send(object, EventCode::CLICKED);
    send(object, EventCode::VALUE_CHANGED);
    send(object, EventCode::PRESSED); // no handler
    assert_eq!(CALLS.load(Relaxed), 11);
    assert_eq!(WITH.load(Relaxed), 7);
    assert_eq!(STATIC_HANDLER.ctx.load(Relaxed), 1);
    lv.edit(last).unwrap().delete().unwrap();
}

#[test]
fn refresh_time_codes_are_never_delivered() {
    static SEEN_DRAW: AtomicU32 = AtomicU32::new(0);
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let mut screen = lv.screen_active_mut().unwrap();
    let object = Obj::create(&mut screen).unwrap().into_view();
    assert_eq!(
        object.on_fn(EventCode::DRAW_MAIN, count),
        Err(Error::InvalidArg)
    );
    assert_eq!(
        object.on_fn(EventCode::COVER_CHECK, count),
        Err(Error::InvalidArg)
    );
    assert!(!handler_allowed(EventCode::DRAW_TASK_ADDED));
    object
        .on(EventCode::ALL, |e| {
            if !handler_allowed(e.code()) {
                SEEN_DRAW.fetch_add(1, Relaxed);
            }
        })
        .unwrap();
    // SAFETY: session held; renders the active screen, which sends the
    // draw events.
    unsafe { sys::lv_refr_now(core::ptr::null_mut()) };
    assert_eq!(SEEN_DRAW.load(Relaxed), 0);
    lv.edit(last).unwrap().delete().unwrap();
}

#[cfg(not(lvgl_rust_views_only))]
#[test]
fn removable_handlers_are_removed_and_deletions_deferred() {
    static HITS: AtomicU32 = AtomicU32::new(0);
    use lvgl_rust::WidgetExt;
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let mut screen = lv.screen_active_mut().unwrap();
    let object = Obj::create(&mut screen).unwrap().into_view();
    let sub = object
        .on_removable(EventCode::CLICKED, 1u8, |_, d| {
            HITS.fetch_add(u32::from(d), Relaxed);
        })
        .unwrap();
    // A handler can only request a deletion: it runs at the next drain.
    object
        .on(EventCode::LONG_PRESSED, |e| {
            e.target().unwrap().delete_later()
        })
        .unwrap();
    let handle = object.handle().unwrap();
    send(object, EventCode::CLICKED);
    sub.remove(&mut lv);
    let object = handle.get(&lv).unwrap();
    send(object, EventCode::CLICKED);
    assert_eq!(HITS.load(Relaxed), 1);
    let children = lv.screen_active().unwrap().get_child_count();
    send(object, EventCode::LONG_PRESSED);
    // Handles report it dead at once; LVGL deletes it at the drain.
    assert_eq!(handle.get(&lv).err(), Some(Error::Dead));
    assert_eq!(lv.screen_active().unwrap().get_child_count(), children);
    lv.timer_handler();
    assert_eq!(lv.screen_active().unwrap().get_child_count(), children - 1);
}

#[test]
fn recursion_stops_at_the_depth_limit() {
    static DEPTH: AtomicU32 = AtomicU32::new(0);
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let mut screen = lv.screen_active_mut().unwrap();
    let object = Obj::create(&mut screen).unwrap().into_view();
    object
        .on(EventCode::REFRESH, |e| {
            DEPTH.fetch_add(1, Relaxed);
            send(e.target().unwrap(), EventCode::REFRESH);
        })
        .unwrap();
    send(object, EventCode::REFRESH);
    // `LVGL_RUST_MAX_CALLBACK_DEPTH` defaults to 16.
    assert_eq!(DEPTH.load(Relaxed), 16);
    lv.edit(last).unwrap().delete().unwrap();
}

#[test]
fn accessors_check_the_code() {
    static KEY: AtomicU32 = AtomicU32::new(0);
    static CHILD: AtomicU32 = AtomicU32::new(0);
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let mut screen = lv.screen_active_mut().unwrap();
    let mut edit = Obj::create(&mut screen).unwrap();
    let object = edit.view();
    object
        .on(EventCode::KEY, |e| {
            KEY.store(e.key().unwrap_or(0), Relaxed);
            assert!(e.point().is_none() && e.param_obj().is_none());
        })
        .unwrap();
    object
        .on(EventCode::CHILD_CREATED, |e| {
            if e.param_obj().is_some() {
                CHILD.fetch_add(1, Relaxed);
            }
            assert!(e.key().is_none());
        })
        .unwrap();
    object
        .on(EventCode::CLICKED, |e| assert!(e.point().is_none()))
        .unwrap();
    let mut key = 0x41u32;
    // SAFETY: session held; KEY takes a pointer to a live `u32`.
    unsafe {
        sys::lv_obj_send_event(
            object.as_ptr(),
            EventCode::KEY.bits() as _,
            (&raw mut key).cast(),
        )
    };
    assert_eq!(KEY.load(Relaxed), 0x41);
    Obj::create(&mut edit).unwrap();
    assert_eq!(CHILD.load(Relaxed), 1);
    send(edit.view(), EventCode::CLICKED); // NULL param: no point
    lv.edit(last).unwrap().delete().unwrap();
}

#[test]
fn handlers_cannot_grow_a_list_being_sent_to() {
    static RESULTS: Mutex<Vec<Result<(), Error>>> = Mutex::new(Vec::new());
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let mut screen = lv.screen_active_mut().unwrap();
    let object = Obj::create(&mut screen).unwrap().into_view();
    object
        .on(EventCode::CLICKED, |e| {
            let target = e.target().unwrap();
            // The target's list is being sent to: refused.
            let own = target.on_fn(EventCode::PRESSED, count);
            // Another object's list is not: allowed.
            let other = target.parent().unwrap().on_fn(EventCode::PRESSED, count);
            RESULTS.lock().unwrap().extend([own, other]);
        })
        .unwrap();
    send(object, EventCode::CLICKED);
    assert_eq!(*RESULTS.lock().unwrap(), [Err(Error::Busy), Ok(())]);
    // Outside handlers registration works.
    object.on_fn(EventCode::PRESSED, count).unwrap();
    lv.edit(last).unwrap().delete().unwrap();
}

#[cfg(not(lvgl_rust_views_only))]
#[test]
fn objects_under_deletion_are_out_of_reach() {
    use lvgl_rust::{Handle, WidgetExt};
    static CHILD: Mutex<Option<Handle<Obj>>> = Mutex::new(None);
    static REACHED: AtomicU32 = AtomicU32::new(0);
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let mut screen = lv.screen_active_mut().unwrap();
    let mut parent = Obj::create(&mut screen).unwrap();
    let child = Obj::create(&mut parent).unwrap().into_view();
    *CHILD.lock().unwrap() = Some(child.handle().unwrap());
    let parent = parent.into_view();
    // The parent is marked first; its children are deleted after.
    parent
        .on(EventCode::DELETE, |e| {
            let child = CHILD.lock().unwrap().unwrap();
            if child.get(e.lv()).is_ok() {
                REACHED.fetch_add(1, Relaxed);
            }
        })
        .unwrap();
    lv.edit(last).unwrap().delete().unwrap();
    assert_eq!(REACHED.load(Relaxed), 0);
}

#[test]
fn posted_work_runs_at_the_drain_with_exclusive_access() {
    static RAN: Mutex<Vec<u32>> = Mutex::new(Vec::new());
    fn record(lv: &mut Lvgl, n: u32) {
        let _: &mut Lvgl = lv;
        RAN.lock().unwrap().push(n);
        if n == 1 {
            // Posted while running: waits for the next drain, not this one.
            lv.post(record, 99).unwrap();
        }
    }
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let mut screen = lv.screen_active_mut().unwrap();
    let object = Obj::create(&mut screen).unwrap().into_view();
    object
        .on(EventCode::CLICKED, |e| {
            let lv = e.lv();
            for n in 1..=4 {
                lv.post(record, n).unwrap();
            }
            // The queue holds 4 entries (`LVGL_RUST_POST_SLOTS`).
            assert_eq!(lv.post(record, 5), Err(Error::Busy));
        })
        .unwrap();
    send(object, EventCode::CLICKED);
    assert!(RAN.lock().unwrap().is_empty());
    // `timer_handler` drains before and after LVGL's timers: the entry
    // posted by `record(1)` runs at the second drain.
    lv.timer_handler();
    assert_eq!(*RAN.lock().unwrap(), [1, 2, 3, 4, 99]);
    // Exclusive work from a posted function.
    lv.post(
        |lv: &mut Lvgl, _: ()| {
            lv.edit(last).unwrap().delete().unwrap();
        },
        (),
    )
    .unwrap();
    let children = lv.screen_active().unwrap().get_child_count();
    lv.timer_handler();
    assert_eq!(lv.screen_active().unwrap().get_child_count(), children - 1);
}

#[test]
fn handler_mask_follows_the_audit() {
    use EventCode as C;
    for code in [
        C::CLICKED,
        C::KEY,
        C::VALUE_CHANGED,
        C::DELETE,
        C::STYLE_CHANGED,
    ] {
        assert!(handler_allowed(code), "{code:?}");
    }
    for code in [
        C::DRAW_MAIN,
        C::DRAW_TASK_ADDED,
        C::GET_SELF_SIZE,
        C::HIT_TEST,
        C::CREATE,
        C::INVALIDATE_AREA,
        C::PREPROCESS,
    ] {
        assert!(!handler_allowed(code), "{code:?}");
    }
}

#[test]
fn safe_codes_are_sent_with_shared_access() {
    use lvgl_rust::enums::{SafeEventCode, SendableEventCode};
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let mut screen = lv.screen_active_mut().unwrap();
    let object = Obj::create(&mut screen).unwrap().into_view();
    CALLS.store(0, Relaxed);
    object.on_fn(EventCode::REFRESH, count).unwrap();
    assert_eq!(object.send_event(SafeEventCode::REFRESH), Ok(()));
    assert_eq!(object.send_event(SafeEventCode::READY), Ok(()));
    assert_eq!(CALLS.load(Relaxed), 1);
    // Exclusive access sends safe codes too, and keeps the object.
    let edit = lv.edit(last).unwrap();
    let edit = edit
        .send_event(SendableEventCode::from(SafeEventCode::REFRESH))
        .unwrap();
    let edit = edit.send_event(SendableEventCode::VALUE_CHANGED).unwrap();
    edit.delete().unwrap();
    assert_eq!(CALLS.load(Relaxed), 2);
}

#[test]
fn deleting_codes_need_exclusive_access_and_report_the_deletion() {
    use lvgl_rust::{enums::SendableEventCode, widgets::MsgBox};
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let children = lv.screen_active().unwrap().get_child_count();
    let mut screen = lv.screen_active_mut().unwrap();
    let mbox = MsgBox::create(&mut screen).unwrap().into_view();
    // SAFETY: session held, live message box.
    unsafe { sys::lv_msgbox_add_close_button(mbox.as_ptr()) };
    // CLICKED on the close button deletes the message box, the button
    // included (lv_msgbox.c:258, 313-321).
    let close = lv
        .edit(|lv| last(lv)?.downcast::<MsgBox>()?.get_header()?.child(-1))
        .unwrap();
    assert!(matches!(
        close.send_event(SendableEventCode::CLICKED),
        Err(Error::Dead)
    ));
    lv.timer_handler();
    assert_eq!(lv.screen_active().unwrap().get_child_count(), children);
}
