//! Views-only mode with a host that owns LVGL's object `id` field.
//!
//! In views-only mode (`LVGL_RUST_HANDLE_SLOTS=0`) the crate defines no OBJ_ID
//! hooks: `LV_USE_OBJ_ID 1` with `LV_USE_OBJ_ID_BUILTIN 0` makes the host
//! define the five `lv_obj_*_id` functions. This host keeps a sorted
//! `(id, pointer)` table of its own objects, removes entries by pointer in
//! `lv_obj_free_id`, and routes every write of the field through one function.
//! The test proves the premise of that design: the free hook runs once for
//! every object, on every deletion path, while the object's `id` is still
//! readable.
#![cfg(lvgl_rust_views_only)]

use core::ffi::{c_char, c_int, c_void};
use lvgl_rust::{Lvgl, LvglAccess, Obj, sys};
use std::sync::Mutex;

/// A sorted table of `(id, object)` pairs, allocated to its exact size.
struct Table(Vec<(u32, usize)>);

impl Table {
    const fn new() -> Self {
        Self(Vec::new())
    }

    fn find(&self, id: u32) -> Option<*mut sys::lv_obj_t> {
        let index = self.0.binary_search_by_key(&id, |entry| entry.0).ok()?;
        Some(self.0[index].1 as *mut sys::lv_obj_t)
    }

    fn insert(&mut self, id: u32, object: *mut sys::lv_obj_t) {
        let index = self
            .0
            .binary_search_by_key(&id, |entry| entry.0)
            .unwrap_err();
        self.0.reserve_exact(1);
        self.0.insert(index, (id, object as usize));
    }

    /// Removes the entry of `object`, found through its stored `id`.
    fn remove(&mut self, id: u32, object: *mut sys::lv_obj_t) -> bool {
        let Ok(index) = self.0.binary_search_by_key(&id, |entry| entry.0) else {
            return false;
        };
        if self.0[index].1 != object as usize {
            return false; // another table's id with the same value
        }
        self.0.remove(index);
        self.0.shrink_to_fit();
        true
    }
}

static TABLE: Mutex<Table> = Mutex::new(Table::new());
static FREED: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// The only function that writes an object's `id`.
fn set_id(object: *mut sys::lv_obj_t, id: u32) {
    // SAFETY: the caller holds the LVGL access; `object` is live.
    unsafe {
        lv_obj_free_id(object);
        sys::lvgl_sys_obj_set_id_raw(object, core::ptr::without_provenance_mut(id as usize));
    }
    if id != 0 {
        TABLE.lock().unwrap().insert(id, object);
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn lv_obj_assign_id(_: *const sys::lv_obj_class_t, _: *mut sys::lv_obj_t) {}

#[unsafe(no_mangle)]
unsafe extern "C" fn lv_obj_free_id(object: *mut sys::lv_obj_t) {
    // SAFETY: LVGL passes the object being destroyed; its memory is still valid.
    let id = unsafe { sys::lv_obj_get_id(object) }.addr() as u32;
    if id != 0 && TABLE.lock().unwrap().remove(id, object) {
        FREED.lock().unwrap().push(object as usize);
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn lv_obj_set_id(object: *mut sys::lv_obj_t, id: *mut c_void) {
    set_id(object, id.addr() as u32);
}

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

struct Guard;

// SAFETY: this binary has one test, which holds the guard for every LVGL call.
unsafe impl LvglAccess for Guard {}

fn at(lv: &Lvgl, ptr: *mut sys::lv_obj_t) -> &Obj {
    // SAFETY: the callers pass live objects of the current session.
    unsafe { Obj::from_raw(lv, core::ptr::NonNull::new(ptr).unwrap()) }
}

fn tag(object: &Obj, id: u32) {
    set_id(object.as_ptr(), id);
}

fn raw_id(object: &Obj) -> u32 {
    // SAFETY: the view is live.
    unsafe { sys::lv_obj_get_id(object.as_ptr()) }.addr() as u32
}

fn len() -> usize {
    TABLE.lock().unwrap().0.len()
}

#[test]
fn the_free_hook_sees_every_deletion_path() {
    let mut guard = Guard;
    let mut lv = lvgl_rust::init(&mut guard).unwrap();
    // SAFETY: LVGL is initialized and the guard holds exclusive access.
    unsafe { sys::lv_display_create(240, 160) };

    // Untagged objects never reach the table, whatever happens to them.
    let (a, b, c, d, e);
    {
        let mut screen = lv.screen_active_mut().unwrap();
        a = Obj::create(&mut screen).unwrap().as_ptr();
        b = Obj::create(&mut screen).unwrap().as_ptr();
    }
    {
        let mut parent = lv.edit(|lv| lv.screen_active()?.child(0)).unwrap();
        c = Obj::create(&mut parent).unwrap().as_ptr();
        d = Obj::create(&mut parent).unwrap().as_ptr();
    }
    {
        let mut parent = lv.edit(|lv| lv.screen_active()?.child(1)).unwrap();
        e = Obj::create(&mut parent).unwrap().as_ptr();
    }
    for (id, object) in [(10, a), (3, b), (7, c), (8, d), (5, e)] {
        tag(at(&lv, object), id);
    }
    assert_eq!(len(), 5);
    // The table is sorted by id, and ids read back from the objects.
    let ids: Vec<u32> = TABLE.lock().unwrap().0.iter().map(|e| e.0).collect();
    assert_eq!(ids, [3, 5, 7, 8, 10]);
    assert_eq!(raw_id(at(&lv, c)), 7);
    // A foreign or stale id is simply absent.
    assert!(TABLE.lock().unwrap().find(99).is_none());

    // Direct deletion.
    let object = at(&lv, d).as_ptr();
    lv.edit(|lv| lv.screen_active()?.child(0)?.child(1))
        .unwrap()
        .delete()
        .unwrap();
    assert_eq!(len(), 4);
    assert!(TABLE.lock().unwrap().find(8).is_none());
    assert_eq!(FREED.lock().unwrap().last(), Some(&(object as usize)));

    // Cascade: deleting a parent frees it and its tagged descendant.
    lv.edit(|lv| lv.screen_active()?.child(1))
        .unwrap()
        .delete()
        .unwrap();
    assert_eq!(len(), 2);
    assert!(TABLE.lock().unwrap().find(3).is_none());
    assert!(TABLE.lock().unwrap().find(5).is_none());

    // `clean` frees every child.
    lv.edit(|lv| lv.screen_active()?.child(0))
        .unwrap()
        .clean()
        .unwrap();
    assert_eq!(len(), 1);
    assert!(TABLE.lock().unwrap().find(7).is_none());

    // Re-tagging an object replaces its entry instead of leaking the old one.
    tag(at(&lv, a), 11);
    assert_eq!(len(), 1);
    assert!(TABLE.lock().unwrap().find(10).is_none());
    assert_eq!(TABLE.lock().unwrap().find(11), Some(a));
    // Untagging removes the entry too.
    set_id(a, 0);
    assert_eq!(len(), 0);

    // A deleted screen frees the objects below it.
    // SAFETY: the guard holds exclusive access; the screen is not loaded.
    unsafe {
        let screen = sys::lv_obj_create(core::ptr::null_mut());
        let child = sys::lv_obj_create(screen);
        set_id(child, 42);
        assert_eq!(len(), 1);
        sys::lv_obj_delete(screen);
    }
    assert_eq!(len(), 0);

    // Objects still tagged when LVGL shuts down are released too.
    tag(at(&lv, a), 50);
    assert_eq!(len(), 1);
    Lvgl::deinit(lv).unwrap();
    assert_eq!(len(), 0);
}
