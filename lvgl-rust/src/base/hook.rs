//! LVGL's OBJ_ID hooks (`LV_USE_OBJ_ID 1`, `LV_OBJ_ID_AUTO_ASSIGN 1`,
//! `LV_USE_OBJ_ID_BUILTIN 0`). `obj->id` holds the object's slot index + 1
//! (0: untracked). `lv_obj_free_id` runs at the end of every object's
//! destructor, whatever the deletion path, and keeps the registry in sync.

use super::{ffi, registry::reg};
use core::ffi::{c_char, c_int, c_void};
use lvgl_rust_sys::{lv_obj_class_t, lv_obj_t};

/// Ids are assigned lazily, when safe code first takes a handle.
#[unsafe(no_mangle)]
pub(crate) unsafe extern "C" fn lv_obj_assign_id(_: *const lv_obj_class_t, _: *mut lv_obj_t) {}

#[unsafe(no_mangle)]
pub(crate) unsafe extern "C" fn lv_obj_free_id(obj: *mut lv_obj_t) {
    if obj.is_null() {
        return;
    }
    // SAFETY: LVGL passes the object being destroyed.
    let id = unsafe { ffi::obj_id(obj) };
    reg().on_free(obj, id);
}

/// Only the registry sets ids: `NULL` untracks the object, as the builtin
/// frees the old id first; other values are ignored.
#[unsafe(no_mangle)]
pub(crate) unsafe extern "C" fn lv_obj_set_id(obj: *mut lv_obj_t, id: *mut c_void) {
    debug_assert!(id.is_null(), "lv_obj_set_id is reserved to lvgl-rust");
    if obj.is_null() || !id.is_null() {
        return;
    }
    // SAFETY: LVGL API contract: `obj` is a live object.
    unsafe {
        lv_obj_free_id(obj);
        ffi::obj_set_id(obj, 0);
    }
}

#[unsafe(no_mangle)]
pub(crate) unsafe extern "C" fn lv_obj_id_compare(a: *const c_void, b: *const c_void) -> c_int {
    (a != b) as c_int
}

/// Writes `#<slot index + 1>` (`#0` when untracked).
#[unsafe(no_mangle)]
pub(crate) unsafe extern "C" fn lv_obj_stringify_id(
    obj: *mut lv_obj_t,
    buf: *mut c_char,
    len: u32,
) -> *const c_char {
    if obj.is_null() || buf.is_null() || len == 0 {
        return core::ptr::null();
    }
    // SAFETY: LVGL passes a live object and a buffer of `len` bytes.
    let (mut id, out) = unsafe {
        (
            ffi::obj_id(obj),
            core::slice::from_raw_parts_mut(buf.cast::<u8>(), len as usize),
        )
    };
    let mut digits = [0u8; 20];
    let mut count = 0;
    loop {
        digits[count] = b'0' + (id % 10) as u8;
        count += 1;
        id /= 10;
        if id == 0 || count == digits.len() {
            break;
        }
    }
    let text = core::iter::once(b'#').chain(digits[..count].iter().rev().copied());
    let mut written = 0;
    let room = out.len() - 1;
    for (dst, byte) in out[..room].iter_mut().zip(text) {
        *dst = byte;
        written += 1;
    }
    out[written] = 0;
    buf
}

/// Keeps the hooks linked: LVGL's archive references them, but the linker
/// only pulls the object file defining them if Rust code does too.
pub(crate) fn keep() {
    let hooks: [*const (); 5] = [
        lv_obj_assign_id as *const (),
        lv_obj_free_id as *const (),
        lv_obj_set_id as *const (),
        lv_obj_id_compare as *const (),
        lv_obj_stringify_id as *const (),
    ];
    core::hint::black_box(hooks);
}
