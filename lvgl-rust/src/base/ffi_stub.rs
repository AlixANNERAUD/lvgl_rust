//! Rust model of the LVGL calls in `ffi.rs`, used under Miri and loom where C
//! cannot run. It keeps what the core relies on: a tree of objects with an
//! `id` field, deletion of the children before their parent, `is_deleting`,
//! the OBJ_ID free hook at the end of each deletion, one display with three
//! layers, and the loading/previous screen pointers.

#![allow(non_camel_case_types)]

extern crate std;

use core::ptr;
use std::{boxed::Box, sync::Mutex, vec::Vec};

/// Opaque in the bindings: the stub's objects stand behind these pointers.
pub(crate) type lv_obj_t = lvgl_rust_sys::lv_obj_t;
pub(crate) type lv_obj_class_t = lvgl_rust_sys::lv_obj_class_t;
pub(crate) type lv_display_t = lvgl_rust_sys::lv_display_t;

struct Object {
    parent: *mut Object,
    children: Vec<*mut Object>,
    id: usize,
    deleting: bool,
    class: *const lv_obj_class_t,
}

/// Stand-ins for the class descriptors (only their addresses matter).
pub(crate) static OBJ_CLASS: u8 = 0;
pub(crate) static OTHER_CLASS: u8 = 0;

struct Display {
    layers: [*mut Object; 3],
    active: *mut Object,
    loading: *mut Object,
    prev: *mut Object,
}

struct State(Option<Display>);
// SAFETY: the stub is used by one token holder at a time, like LVGL.
unsafe impl Send for State {}

static STATE: Mutex<State> = Mutex::new(State(None));

fn with_display<R>(f: impl FnOnce(&mut Display) -> R) -> R {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(state.0.as_mut().expect("stub LVGL is not initialized"))
}

fn new_object(parent: *mut Object, class: *const lv_obj_class_t) -> *mut Object {
    let object = Box::into_raw(Box::new(Object {
        parent,
        children: Vec::new(),
        id: 0,
        deleting: false,
        class,
    }));
    // C pointers carry no Rust provenance: the core may round-trip them
    // through zero-sized views. Model that by exposing the allocation and
    // recovering it from the address.
    object.expose_provenance();
    if !parent.is_null() {
        unsafe { (*parent).children.push(object) };
    }
    object
}

fn obj(p: *const lv_obj_t) -> *mut Object {
    ptr::with_exposed_provenance_mut(p.addr())
}

pub(crate) fn obj_class() -> *const lv_obj_class_t {
    (&raw const OBJ_CLASS).cast()
}

/// A second class, for class-check tests.
pub(crate) fn other_class() -> *const lv_obj_class_t {
    (&raw const OTHER_CLASS).cast()
}

pub(crate) unsafe fn init() {
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let class = obj_class();
    state.0 = Some(Display {
        layers: [
            new_object(ptr::null_mut(), class),
            new_object(ptr::null_mut(), class),
            new_object(ptr::null_mut(), class),
        ],
        active: new_object(ptr::null_mut(), class),
        loading: ptr::null_mut(),
        prev: ptr::null_mut(),
    });
}

pub(crate) unsafe fn deinit() {
    // Objects are leaked: the stub only has to stop answering.
    STATE.lock().unwrap_or_else(|e| e.into_inner()).0 = None;
}

pub(crate) unsafe fn set_tick_cb(_: unsafe extern "C" fn() -> u32) {}

pub(crate) unsafe fn timer_handler() -> u32 {
    1
}

pub(crate) unsafe fn obj_create(parent: *mut lv_obj_t) -> *mut lv_obj_t {
    new_object(obj(parent), obj_class()).cast()
}

/// Creates an object of [`other_class`].
pub(crate) unsafe fn obj_create_other(parent: *mut lv_obj_t) -> *mut lv_obj_t {
    new_object(obj(parent), other_class()).cast()
}

unsafe fn delete_tree(o: *mut Object) {
    unsafe {
        (*o).deleting = true;
        let children = core::mem::take(&mut (*o).children);
        for child in children {
            delete_tree(child);
        }
        // LVGL runs `lv_obj_free_id` at the end of the destructor.
        super::hook::lv_obj_free_id(o.cast());
        drop(Box::from_raw(o));
    }
}

pub(crate) unsafe fn obj_delete(o: *mut lv_obj_t) {
    let o = obj(o);
    unsafe {
        let parent = (*o).parent;
        if !parent.is_null() {
            (*parent).children.retain(|&c| c != o);
        }
        with_display(|d| {
            if d.active == o {
                d.active = ptr::null_mut();
            }
        });
        delete_tree(o);
    }
}

pub(crate) unsafe fn obj_clean(o: *mut lv_obj_t) {
    let o = obj(o);
    unsafe {
        let children = core::mem::take(&mut (*o).children);
        for child in children {
            delete_tree(child);
        }
    }
}

pub(crate) unsafe fn obj_set_parent(o: *mut lv_obj_t, parent: *mut lv_obj_t) {
    let (o, parent) = (obj(o), obj(parent));
    unsafe {
        let old = (*o).parent;
        if old.is_null() {
            return; // LVGL refuses to reparent a screen.
        }
        (*old).children.retain(|&c| c != o);
        (*parent).children.push(o);
        (*o).parent = parent;
    }
}

pub(crate) unsafe fn obj_parent(o: *const lv_obj_t) -> *mut lv_obj_t {
    unsafe { (*obj(o)).parent.cast() }
}

pub(crate) unsafe fn obj_child(o: *const lv_obj_t, index: i32) -> *mut lv_obj_t {
    let children = unsafe { &(*obj(o)).children };
    let index = if index < 0 {
        children.len() as i64 + index as i64
    } else {
        index as i64
    };
    usize::try_from(index)
        .ok()
        .and_then(|i| children.get(i))
        .map_or(ptr::null_mut(), |&c| c.cast())
}

pub(crate) unsafe fn obj_class_of(o: *const lv_obj_t) -> *const lv_obj_class_t {
    unsafe { (*obj(o)).class }
}

pub(crate) unsafe fn obj_has_class(o: *const lv_obj_t, class: *const lv_obj_class_t) -> bool {
    let own = unsafe { (*obj(o)).class };
    own == class || class == obj_class()
}

pub(crate) unsafe fn obj_is_deleting(o: *const lv_obj_t) -> bool {
    unsafe { (*obj(o)).deleting }
}

pub(crate) unsafe fn obj_id(o: *const lv_obj_t) -> usize {
    unsafe { (*obj(o)).id }
}

pub(crate) unsafe fn obj_set_id(o: *mut lv_obj_t, id: usize) {
    unsafe { (*obj(o)).id = id }
}

pub(crate) unsafe fn screen_active() -> *mut lv_obj_t {
    with_display(|d| d.active.cast())
}

pub(crate) unsafe fn screen_load(screen: *mut lv_obj_t, _: u32, _: u32, _: u32, _: bool) {
    with_display(|d| d.active = obj(screen));
}

/// Marks screens as loading / previous, as during an animated load.
pub(crate) fn set_loading_screens(loading: *mut lv_obj_t, prev: *mut lv_obj_t) {
    with_display(|d| {
        d.loading = obj(loading);
        d.prev = obj(prev);
    });
}

pub(crate) unsafe fn display_next(display: *mut lv_display_t) -> *mut lv_display_t {
    if display.is_null() {
        // Any non-null value: the stub has a single display.
        ptr::NonNull::<lv_display_t>::dangling().as_ptr()
    } else {
        ptr::null_mut()
    }
}

pub(crate) unsafe fn display_layers(_: *mut lv_display_t) -> [*mut lv_obj_t; 3] {
    with_display(|d| d.layers.map(|l| l.cast()))
}

pub(crate) unsafe fn display_loading_screens(_: *mut lv_display_t) -> [*mut lv_obj_t; 2] {
    with_display(|d| [d.loading.cast(), d.prev.cast()])
}

/// The model has no LVGL widget classes.
pub(crate) fn composites() -> &'static [super::structural::Composite] {
    &[]
}

pub(crate) fn leaves() -> &'static [super::structural::ClassRef] {
    &[]
}
