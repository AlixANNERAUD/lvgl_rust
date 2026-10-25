//! Restricted handlers (query and draw codes) against LVGL.

use lvgl_rust::{
    CoverResult, Error, Lvgl, LvglAccess, Obj, QueryEvent, Session, color,
    enums::{EventCode, QueryCode},
    handler_allowed, query_handler_allowed, sys,
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

fn add_flag(object: &Obj, flag: sys::lv_obj_flag_t) {
    // SAFETY: session held, live object; these flags free nothing.
    unsafe { sys::lv_obj_add_flag(object.as_ptr(), flag) };
}

fn set_size(object: &Obj, width: i32, height: i32) {
    // SAFETY: session held, live object, no handler of it deletes anything.
    unsafe { sys::lv_obj_set_size(object.as_ptr(), width, height) };
}

/// Renders the active screen now, which sends the draw and cover codes.
fn render() {
    // SAFETY: called with a session held.
    unsafe { sys::lv_refr_now(core::ptr::null_mut()) };
}

#[test]
fn query_codes_follow_the_audit() {
    use EventCode as C;
    for code in [
        C::HIT_TEST,
        C::COVER_CHECK,
        C::REFR_EXT_DRAW_SIZE,
        C::GET_SELF_SIZE,
        C::DRAW_MAIN_BEGIN,
        C::DRAW_MAIN,
        C::DRAW_MAIN_END,
        C::DRAW_POST_BEGIN,
        C::DRAW_POST,
        C::DRAW_POST_END,
        C::DRAW_TASK_ADDED,
    ] {
        assert!(query_handler_allowed(code), "{code:?}");
        // Shared handlers stay refused for them.
        assert!(!handler_allowed(code), "{code:?}");
    }
    for code in [C::CLICKED, C::DELETE, C::CREATE, C::INVALIDATE_AREA, C::ALL] {
        assert!(!query_handler_allowed(code), "{code:?}");
    }
    assert_eq!(EventCode::from(QueryCode::HIT_TEST), C::HIT_TEST);
}

#[test]
fn self_size_answers_reach_layout() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let object = Obj::create(lv.screen_active().unwrap()).unwrap();
    object
        .on_query(QueryCode::GET_SELF_SIZE, |e: &QueryEvent<'_>| {
            // Only the code's own parameter is typed.
            assert!(e.hit_test().is_none() && e.layer().is_none());
            e.self_size().unwrap().extend(120, 40);
        })
        .unwrap();
    // SAFETY: session held, live object.
    let (width, height) = unsafe {
        (
            sys::lv_obj_get_self_width(object.as_ptr()),
            sys::lv_obj_get_self_height(object.as_ptr()),
        )
    };
    assert!(width >= 120, "{width}");
    assert!(height >= 40, "{height}");
    lv.edit(last).unwrap().delete().unwrap();
}

#[test]
fn hit_tests_are_answered_with_user_data() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let object = Obj::create(lv.screen_active().unwrap()).unwrap();
    set_size(object, 100, 100);
    add_flag(object, sys::lv_obj_flag_t_LV_OBJ_FLAG_ADV_HITTEST);
    // Points left of `limit` do not click the object.
    object
        .on_query_with(
            QueryCode::HIT_TEST,
            30i32,
            |e: &QueryEvent<'_>, limit: i32| {
                let hit = e.hit_test().unwrap();
                assert!(hit.result());
                hit.set_result(hit.point().x >= limit);
            },
        )
        .unwrap();
    // SAFETY: called with a session held.
    unsafe { sys::lv_obj_update_layout(object.as_ptr()) };
    let test = |x| {
        let point = sys::lv_point_t { x, y: 10 };
        // SAFETY: session held, live object.
        unsafe { sys::lv_obj_hit_test(object.as_ptr(), &point) }
    };
    assert!(!test(10));
    assert!(test(50));
    lv.edit(last).unwrap().delete().unwrap();
}

#[test]
fn ext_draw_size_keeps_the_largest_request() {
    static SEEN: AtomicI32 = AtomicI32::new(0);
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let object = Obj::create(lv.screen_active().unwrap()).unwrap();
    object
        .on_query(QueryCode::REFR_EXT_DRAW_SIZE, |e: &QueryEvent<'_>| {
            let size = e.ext_draw_size().unwrap();
            size.extend(25);
            size.extend(5);
        })
        .unwrap();
    object
        .on_query(QueryCode::REFR_EXT_DRAW_SIZE, |e: &QueryEvent<'_>| {
            SEEN.store(e.ext_draw_size().unwrap().get(), Relaxed);
        })
        .unwrap();
    // SAFETY: session held, live object.
    unsafe { sys::lv_obj_refresh_ext_draw_size(object.as_ptr()) };
    assert!(SEEN.load(Relaxed) >= 25);
    lv.edit(last).unwrap().delete().unwrap();
}

static DRAWN: AtomicU32 = AtomicU32::new(0);
static COVER: AtomicU32 = AtomicU32::new(0);
static RECOLORED: AtomicU32 = AtomicU32::new(0);
static REENTERED: AtomicU32 = AtomicU32::new(0);

/// A second token, only to show that a restricted handler cannot start a
/// session (it never gets one).
struct Probe;

// SAFETY: never yields access: `Lvgl::enter` fails inside callbacks.
unsafe impl LvglAccess for Probe {}

fn on_draw(e: &QueryEvent<'_>) {
    let layer = e.layer().unwrap();
    let clip = layer.clip_area();
    assert!(clip.x2 >= clip.x1 && clip.y2 >= clip.y1);
    assert!(!layer.as_ptr().is_null());
    if matches!(Lvgl::enter(&mut Probe), Err(Error::Reentrant)) {
        REENTERED.fetch_add(1, Relaxed);
    }
    DRAWN.fetch_add(1, Relaxed);
}

#[test]
fn draw_and_cover_codes_reach_restricted_handlers() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let object = Obj::create(lv.screen_active().unwrap()).unwrap();
    set_size(object, 60, 40);
    add_flag(object, sys::lv_obj_flag_t_LV_OBJ_FLAG_SEND_DRAW_TASK_EVENTS);
    object.on_query_fn(QueryCode::DRAW_MAIN, on_draw).unwrap();
    object
        .on_query(QueryCode::COVER_CHECK, |e: &QueryEvent<'_>| {
            let check = e.cover_check().unwrap();
            let area = check.area();
            assert!(area.x2 >= area.x1);
            check.set_result(CoverResult::NotCover);
            COVER.fetch_add(1, Relaxed);
        })
        .unwrap();
    object
        .on_query(QueryCode::DRAW_TASK_ADDED, |e: &QueryEvent<'_>| {
            let Some(task) = e.draw_task() else { return };
            let area = task.area();
            assert!(area.x2 >= area.x1);
            if task.kind() == sys::lv_draw_task_type_t_LV_DRAW_TASK_TYPE_FILL
                && task.part() == sys::lv_part_t_LV_PART_MAIN
            {
                let red = color::hex(0xff0000);
                assert!(task.set_color(red));
                assert!(task.set_opa(200));
                let read = task.color().unwrap();
                if (read.red, read.green, read.blue) == (0xff, 0, 0) && task.opa() == Some(200) {
                    RECOLORED.fetch_add(1, Relaxed);
                }
            }
        })
        .unwrap();
    // An area inside the object: LVGL asks it whether it covers the area
    // (to start drawing from it), then draws it.
    let inside = sys::lv_area_t {
        x1: 10,
        y1: 10,
        x2: 20,
        y2: 20,
    };
    // Pending invalidations (creation, layout) cover more than the object.
    render();
    let drawn = DRAWN.load(Relaxed);
    assert!(drawn > 0);
    // SAFETY: session held, live object.
    unsafe { sys::lv_obj_invalidate_area(object.as_ptr(), &inside) };
    render();
    assert!(DRAWN.load(Relaxed) > drawn);
    assert_eq!(REENTERED.load(Relaxed), DRAWN.load(Relaxed));
    assert!(COVER.load(Relaxed) > 0);
    assert!(RECOLORED.load(Relaxed) > 0);
    lv.edit(last).unwrap().delete().unwrap();
}

#[test]
fn safe_sends_reach_restricted_handlers_without_a_layer() {
    static WITHOUT: AtomicU32 = AtomicU32::new(0);
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let object = Obj::create(lv.screen_active().unwrap()).unwrap();
    object
        .on_query(QueryCode::DRAW_MAIN_BEGIN, |e: &QueryEvent<'_>| {
            if e.layer().is_none() {
                WITHOUT.fetch_add(1, Relaxed);
            }
        })
        .unwrap();
    object
        .send_event(lvgl_rust::enums::SafeEventCode::DRAW_MAIN_BEGIN)
        .unwrap();
    assert_eq!(WITHOUT.load(Relaxed), 1);
    lv.edit(last).unwrap().delete().unwrap();
}
