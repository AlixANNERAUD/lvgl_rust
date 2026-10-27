//! LVGL's heap from a Rust allocator (`mise run test-allocator`: a
//! configuration with `LV_STDLIB_CUSTOM` and `--features allocator`).
#![cfg(feature = "allocator")]

use lvgl_rust::{LvglAccess, LvglAllocator, Obj, SizedHeader, sys, widgets::Label};
use std::{
    alloc::System,
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicUsize, Ordering::Relaxed},
    },
};

/// Counts what LVGL asks of the Rust allocator.
struct Counting {
    inner: SizedHeader<System>,
    allocs: AtomicUsize,
    reallocs: AtomicUsize,
    frees: AtomicUsize,
}

// SAFETY: delegates to `SizedHeader`.
unsafe impl LvglAllocator for Counting {
    fn alloc(&self, size: usize) -> *mut u8 {
        self.allocs.fetch_add(1, Relaxed);
        let p = self.inner.alloc(size);
        assert_eq!(p.addr() % core::mem::size_of::<usize>(), 0);
        p
    }

    unsafe fn realloc(&self, ptr: *mut u8, new_size: usize) -> *mut u8 {
        self.reallocs.fetch_add(1, Relaxed);
        // SAFETY: caller contract.
        unsafe { self.inner.realloc(ptr, new_size) }
    }

    unsafe fn free(&self, ptr: *mut u8) {
        self.frees.fetch_add(1, Relaxed);
        // SAFETY: caller contract.
        unsafe { self.inner.free(ptr) }
    }
}

static HEAP: Counting = Counting {
    inner: SizedHeader(System),
    allocs: AtomicUsize::new(0),
    reallocs: AtomicUsize::new(0),
    frees: AtomicUsize::new(0),
};

lvgl_rust::lvgl_allocator!(HEAP);

static LOCK: Mutex<()> = Mutex::new(());

struct Guard(#[allow(dead_code)] MutexGuard<'static, ()>);

// SAFETY: a guard of the mutex serializing every LVGL call of this process.
unsafe impl LvglAccess for Guard {}

#[test]
fn lvgl_allocates_through_the_rust_allocator() {
    let mut guard = Guard(LOCK.lock().unwrap());
    let mut lv = lvgl_rust::init(&mut guard).unwrap();
    // SAFETY: session held.
    unsafe { sys::lv_display_create(240, 160) };
    let before = HEAP.allocs.load(Relaxed);
    assert!(before > 0, "lv_init allocated through the Rust allocator");
    let mut screen = lv.screen_active_mut().unwrap();
    let label = Label::create(&mut screen).unwrap().into_view();
    label
        .set_text_str("a text long enough to grow the label's buffer in chunks")
        .unwrap();
    assert!(HEAP.allocs.load(Relaxed) > before);
    assert!(HEAP.reallocs.load(Relaxed) > 0);
    let frees = HEAP.frees.load(Relaxed);
    lv.edit(|lv| lv.screen_active()?.child(-1))
        .unwrap()
        .delete()
        .unwrap();
    assert!(HEAP.frees.load(Relaxed) > frees);
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
}
