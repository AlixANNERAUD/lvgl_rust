//! The LVGL calls the core makes, in one place. Under Miri and loom this
//! module is replaced by `ffi_stub.rs`, a Rust model of the object tree, so
//! the registry state machine can run there.

pub(crate) use lvgl_rust_sys::{lv_display_t, lv_obj_class_t, lv_obj_t};

use lvgl_rust_sys as sys;

pub(crate) fn obj_class() -> *const lv_obj_class_t {
    &raw const sys::lv_obj_class
}

pub(crate) unsafe fn init() {
    unsafe { sys::lv_init() }
}

pub(crate) unsafe fn deinit() {
    unsafe { sys::lv_deinit() }
}

pub(crate) unsafe fn set_tick_cb(cb: unsafe extern "C" fn() -> u32) {
    unsafe { sys::lv_tick_set_cb(Some(cb)) }
}

pub(crate) unsafe fn timer_handler() -> u32 {
    unsafe { sys::lv_timer_handler() }
}

pub(crate) unsafe fn obj_create(parent: *mut lv_obj_t) -> *mut lv_obj_t {
    unsafe { sys::lv_obj_create(parent) }
}

pub(crate) unsafe fn obj_delete(obj: *mut lv_obj_t) {
    unsafe { sys::lv_obj_delete(obj) }
}

pub(crate) unsafe fn obj_clean(obj: *mut lv_obj_t) {
    unsafe { sys::lv_obj_clean(obj) }
}

pub(crate) unsafe fn obj_set_parent(obj: *mut lv_obj_t, parent: *mut lv_obj_t) {
    unsafe { sys::lv_obj_set_parent(obj, parent) }
}

pub(crate) unsafe fn obj_move_to_index(obj: *mut lv_obj_t, index: i32) {
    unsafe { sys::lv_obj_move_to_index(obj, index) }
}

#[cfg(not(lvgl_rust_views_only))]
pub(crate) unsafe fn timer_delete(timer: *mut sys::lv_timer_t) {
    unsafe { sys::lv_timer_delete(timer) }
}

pub(crate) unsafe fn obj_parent(obj: *const lv_obj_t) -> *mut lv_obj_t {
    unsafe { sys::lv_obj_get_parent(obj) }
}

pub(crate) unsafe fn obj_child(obj: *const lv_obj_t, index: i32) -> *mut lv_obj_t {
    unsafe { sys::lv_obj_get_child(obj, index) }
}

pub(crate) unsafe fn obj_class_of(obj: *const lv_obj_t) -> *const lv_obj_class_t {
    unsafe { sys::lv_obj_get_class(obj) }
}

pub(crate) unsafe fn obj_has_class(obj: *const lv_obj_t, class: *const lv_obj_class_t) -> bool {
    unsafe { sys::lv_obj_has_class(obj, class) }
}

pub(crate) unsafe fn obj_is_deleting(obj: *const lv_obj_t) -> bool {
    unsafe { sys::lvgl_sys_obj_is_deleting(obj) }
}

#[cfg(all(not(lvgl_rust_views_only), not(lvgl_rust_docs)))]
pub(crate) unsafe fn obj_id(obj: *const lv_obj_t) -> usize {
    unsafe { sys::lv_obj_get_id(obj) as usize }
}

/// Writes `obj->id` without running the OBJ_ID hooks.
#[cfg(all(not(lvgl_rust_views_only), not(lvgl_rust_docs)))]
pub(crate) unsafe fn obj_set_id(obj: *mut lv_obj_t, id: usize) {
    unsafe { sys::lvgl_sys_obj_set_id_raw(obj, core::ptr::without_provenance_mut(id)) }
}

// docs.rs builds the bindings with OBJ_ID off: these functions do not exist.
#[cfg(all(not(lvgl_rust_views_only), lvgl_rust_docs))]
pub(crate) unsafe fn obj_id(_: *const lv_obj_t) -> usize {
    0
}

#[cfg(all(not(lvgl_rust_views_only), lvgl_rust_docs))]
pub(crate) unsafe fn obj_set_id(_: *mut lv_obj_t, _: usize) {}

pub(crate) unsafe fn screen_active() -> *mut lv_obj_t {
    unsafe { sys::lv_screen_active() }
}

pub(crate) unsafe fn screen_load(
    screen: *mut lv_obj_t,
    anim: u32,
    time: u32,
    delay: u32,
    auto_delete: bool,
) {
    #[allow(clippy::unnecessary_cast)] // `lv_screen_load_anim_t` is `c_int` on MSVC.
    unsafe {
        sys::lv_screen_load_anim(
            screen,
            anim as sys::lv_screen_load_anim_t,
            time,
            delay,
            auto_delete,
        )
    }
}

/// Displays, in creation order: `display_next(null)` is the first one.
pub(crate) unsafe fn display_next(display: *mut lv_display_t) -> *mut lv_display_t {
    unsafe { sys::lv_display_get_next(display) }
}

/// The three layers of `display` (bottom, top, system).
pub(crate) unsafe fn display_layers(display: *mut lv_display_t) -> [*mut lv_obj_t; 3] {
    unsafe {
        [
            sys::lv_display_get_layer_bottom(display),
            sys::lv_display_get_layer_top(display),
            sys::lv_display_get_layer_sys(display),
        ]
    }
}

/// Screens LVGL keeps pointers to during an animated load.
pub(crate) unsafe fn display_loading_screens(display: *mut lv_display_t) -> [*mut lv_obj_t; 2] {
    unsafe {
        [
            sys::lv_display_get_screen_loading(display),
            sys::lv_display_get_screen_prev(display),
        ]
    }
}

/// The composite widgets of the part tables (generated).
pub(crate) fn composites() -> &'static [super::structural::Composite] {
    crate::generated::COMPOSITES
}

/// Widget classes without parts (generated).
pub(crate) fn leaves() -> &'static [super::structural::ClassRef] {
    crate::generated::LEAVES
}
