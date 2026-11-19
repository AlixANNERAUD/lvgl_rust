//! Deferred deletion of host objects against LVGL (`deferred-delete`): a
//! host class whose instances embed a `DeferredLink`, owners dropped
//! without a session or while one is held elsewhere, and objects dying
//! before the drain reaches them. The class descriptor and the instance
//! layout need the private API bindings (`LV_USE_PRIVATE_API=1`, see
//! `mise run test-deferred-delete`).
//!
//! With the registry and `free-observer`, the free observer sees what the
//! drain deletes. In views-only mode this test is the host of the OBJ_ID
//! hooks, and its `lv_obj_free_id` sees it.

use core::{
    mem::{MaybeUninit, offset_of, size_of},
    ptr::{self, NonNull},
};
use lvgl_rust::{DeferredClass, DeferredLink, Lvgl, LvglAccess, sys, tick_inc};
use std::sync::Mutex;

/// The instance struct of the host class: LVGL's object, then the link.
#[repr(C)]
struct Window {
    obj: sys::lv_obj_t,
    link: DeferredLink,
}

// SAFETY: only `Window`s are given to it, whose `link` is used through it
// only.
static WINDOWS: DeferredClass = unsafe { DeferredClass::new(offset_of!(Window, link)) };

// Written once at the start of the test, before any window exists.
static mut WINDOW_CLASS: MaybeUninit<sys::lv_obj_class_t> = MaybeUninit::uninit();

/// Windows whose destructor ran, in order.
static DESTROYED: Mutex<Vec<usize>> = Mutex::new(Vec::new());
/// Objects LVGL freed, as the free observer or the free hook saw them.
static FREED: Mutex<Vec<usize>> = Mutex::new(Vec::new());

unsafe extern "C" fn destructor(_: *const sys::lv_obj_class_t, obj: *mut sys::lv_obj_t) {
    // SAFETY: LVGL destroys a `Window`; its memory is still valid.
    unsafe { WINDOWS.forget(NonNull::new_unchecked(obj)) };
    DESTROYED.lock().unwrap().push(obj as usize);
}

fn window(parent: *mut sys::lv_obj_t) -> NonNull<sys::lv_obj_t> {
    // SAFETY: the class is initialized and the parent is live; the test
    // holds the session.
    unsafe {
        let obj = sys::lv_obj_class_create_obj((&raw const WINDOW_CLASS).cast(), parent);
        sys::lv_obj_class_init_obj(obj);
        NonNull::new(obj).unwrap()
    }
}

fn request(obj: NonNull<sys::lv_obj_t>) {
    // SAFETY: a live window, nothing deletes it during the call.
    unsafe { WINDOWS.request_delete(obj) }
}

/// What a host gives the owner of a window: deletes it when dropped,
/// without a session.
struct OwnedWindow(NonNull<sys::lv_obj_t>);

// SAFETY: the owner only sends the pointer to `request_delete`.
unsafe impl Send for OwnedWindow {}

impl Drop for OwnedWindow {
    fn drop(&mut self) {
        request(self.0);
    }
}

/// Starts a scenario: LVGL reuses the addresses of freed objects, so the
/// destructors are counted from here.
fn reset() {
    DESTROYED.lock().unwrap().clear();
}

/// How many times the destructor of `obj` ran since [`reset`].
fn destroyed(obj: NonNull<sys::lv_obj_t>) -> usize {
    let destroyed = DESTROYED.lock().unwrap();
    destroyed
        .iter()
        .filter(|&&o| o == obj.as_ptr() as usize)
        .count()
}

fn freed(obj: NonNull<sys::lv_obj_t>) -> bool {
    FREED.lock().unwrap().contains(&(obj.as_ptr() as usize))
}

fn children(obj: *mut sys::lv_obj_t) -> u32 {
    // SAFETY: a live object, under the session.
    unsafe { sys::lv_obj_get_child_count(obj) }
}

#[cfg(lvgl_rust_free_observer)]
fn record(freed: lvgl_rust::Freed) {
    FREED.lock().unwrap().push(freed.object() as usize);
}

/// The OBJ_ID hooks of a views-only host (`lvgl-rust/conf` sets
/// `LV_USE_OBJ_ID_BUILTIN 0`): only the free hook matters here.
#[cfg(lvgl_rust_views_only)]
mod hooks {
    use super::*;
    use core::ffi::{c_char, c_int, c_void};

    #[unsafe(no_mangle)]
    unsafe extern "C" fn lv_obj_assign_id(_: *const sys::lv_obj_class_t, _: *mut sys::lv_obj_t) {}

    #[unsafe(no_mangle)]
    unsafe extern "C" fn lv_obj_free_id(obj: *mut sys::lv_obj_t) {
        FREED.lock().unwrap().push(obj as usize);
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn lv_obj_set_id(_: *mut sys::lv_obj_t, _: *mut c_void) {}

    #[unsafe(no_mangle)]
    unsafe extern "C" fn lv_obj_id_compare(a: *const c_void, b: *const c_void) -> c_int {
        (a != b) as c_int
    }

    #[unsafe(no_mangle)]
    unsafe extern "C" fn lv_obj_stringify_id(
        _: *mut sys::lv_obj_t,
        buffer: *mut c_char,
        _: u32,
    ) -> *const c_char {
        buffer
    }
}

struct Guard;

// SAFETY: this binary has one test, which holds the guard for every LVGL
// call; the thread it spawns calls `request_delete` only.
unsafe impl LvglAccess for Guard {}

#[test]
fn host_objects_are_deleted_at_the_next_drain() {
    let mut guard = Guard;
    // Mutated only to install the free observer.
    #[cfg_attr(not(lvgl_rust_free_observer), allow(unused_mut))]
    let mut lv = lvgl_rust::init(&mut guard).unwrap();
    // SAFETY: no window exists yet; the session is held.
    let screen = unsafe {
        let mut class = sys::lv_obj_class_t {
            base_class: &raw const sys::lv_obj_class,
            destructor_cb: Some(destructor),
            ..Default::default()
        };
        class.set_instance_size(size_of::<Window>() as u32);
        (&raw mut WINDOW_CLASS).write(MaybeUninit::new(class));
        sys::lv_display_create(240, 160);
        sys::lv_screen_active()
    };
    #[cfg(lvgl_rust_free_observer)]
    lv.set_free_observer(record).unwrap();

    // An owner dropped without a session: deleted at the next `enter`.
    let owner = OwnedWindow(window(screen));
    let first = owner.0;
    #[cfg(lvgl_rust_free_observer)]
    {
        use lvgl_rust::{Obj, WidgetExt};
        // A tracked object in the window: the free observer sees the drain.
        // SAFETY: a live object of this session.
        let inner = unsafe { sys::lv_obj_create(first.as_ptr()) };
        let view = unsafe { Obj::from_raw(&lv, NonNull::new(inner).unwrap()) };
        view.handle().unwrap();
    }
    drop(lv);
    drop(owner);
    assert_eq!(destroyed(first), 0);
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    assert_eq!(destroyed(first), 1);
    assert_eq!(children(screen), 0);
    #[cfg(lvgl_rust_free_observer)]
    assert_eq!(FREED.lock().unwrap().len(), 1, "the tracked object inside");
    #[cfg(lvgl_rust_views_only)]
    assert!(
        freed(first),
        "the host's free hook sees the drain's deletion"
    );
    let _ = freed;

    reset();
    // Requested twice: deleted once.
    let twice = window(screen);
    request(twice);
    request(twice);
    lv.timer_handler();
    assert_eq!(destroyed(twice), 1);

    reset();
    // Dropped on another thread while this one holds the session: the
    // request waits for this session's next drain.
    let owner = OwnedWindow(window(screen));
    let remote = owner.0;
    std::thread::spawn(move || drop(owner)).join().unwrap();
    assert_eq!(destroyed(remote), 0);
    lv.timer_handler();
    assert_eq!(destroyed(remote), 1);

    reset();
    // A parent and its child both queued, the parent drained first: the
    // child leaves the queue in its destructor, and dies once.
    let parent = window(screen);
    let child = window(parent.as_ptr());
    request(child);
    request(parent);
    lv.timer_handler();
    assert_eq!((destroyed(parent), destroyed(child)), (1, 1));

    reset();
    // A queued window whose container is deleted with exclusive access.
    // SAFETY: the session is held, at depth 0.
    let container = unsafe { sys::lv_obj_create(screen) };
    let inside = window(container);
    request(inside);
    // SAFETY: as above; nothing else refers to the container.
    unsafe { sys::lv_obj_delete(container) };
    assert_eq!(destroyed(inside), 1);
    // The same through `delete_now`, on a window holding a queued window.
    let outer = window(screen);
    let nested = window(outer.as_ptr());
    request(nested);
    // SAFETY: the host's own window, deleted as its owner intends.
    unsafe { WINDOWS.delete_now(&mut lv, outer) }.unwrap();
    assert_eq!((destroyed(outer), destroyed(nested)), (1, 1));
    lv.timer_handler();
    assert_eq!(destroyed(inside) + destroyed(nested), 2);
    assert_eq!(children(screen), 0);

    // A window that is a screen being loaded with an animation: LVGL
    // holds it until the load completes, so the drain keeps it queued
    // (`Busy`) and deletes it at the first drain after the load.
    reset();
    let loading = window(ptr::null_mut());
    // SAFETY: the session is held; both screens are live.
    unsafe {
        sys::lv_screen_load_anim(
            loading.as_ptr(),
            sys::lv_screen_load_anim_t_LV_SCREEN_LOAD_ANIM_FADE_IN,
            100,
            0,
            false,
        )
    };
    request(loading);
    lv.timer_handler();
    assert_eq!(destroyed(loading), 0, "still loading");
    for _ in 0..4 {
        tick_inc(50);
        lv.timer_handler();
    }
    assert_eq!(destroyed(loading), 1);
    // Back to the first screen for the rest.
    // SAFETY: as above.
    unsafe { sys::lv_screen_load(screen) };

    // Queued before `deinit`: its last drain deletes it, once. (An object
    // requested after that drain dies in `lv_deinit`, where `forget` and
    // requests no longer touch anything: see the stand-in below and the
    // queue tests.)
    reset();
    let last = window(screen);
    request(last);
    Lvgl::deinit(lv).unwrap();
    assert_eq!(destroyed(last), 1);

    // After `deinit`, a request touches nothing: owners dropped late hold
    // freed pointers. A zeroed stand-in keeps its link at zero.
    let mut stand_in = MaybeUninit::<Window>::zeroed();
    let obj = NonNull::new(stand_in.as_mut_ptr().cast::<sys::lv_obj_t>()).unwrap();
    request(obj);
    // SAFETY: zero bits are a valid `Window` (LVGL creates them zeroed).
    let link = unsafe {
        ptr::addr_of!((*stand_in.as_ptr()).link)
            .cast::<usize>()
            .read()
    };
    assert_eq!(link, 0);
}
