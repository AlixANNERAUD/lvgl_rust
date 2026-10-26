//! The deferred-free list (feature `heap`): blocks of LVGL's heap that
//! LVGL may still know by address (owned image sources), freed at the next
//! drain, after the deletions that retired them have finished.
//!
//! Each block starts with its link (`*mut c_void`, the header of an
//! `LvBox<*mut c_void, T>` whose `T` needs no drop). One static pointer,
//! touched only by the token holder: 4 B on 32-bit targets.

use super::ffi;
use core::{cell::Cell, ffi::c_void, ptr};

/// The list head; [`DEINIT`] once `deinit` started, after which blocks are
/// freed at once (LVGL frees its heap at the end of `lv_deinit`).
struct Head(Cell<*mut c_void>);

// SAFETY: read and written only by the token holder (a session, or a
// callback it runs); sessions are ordered by the `LvglAccess` contract.
unsafe impl Sync for Head {}

static HEAD: Head = Head(Cell::new(ptr::null_mut()));

/// Never a block address: blocks are aligned to a pointer.
fn deinit_marker() -> *mut c_void {
    ptr::without_provenance_mut(1)
}

/// Queues `block` for the next drain.
///
/// # Safety
///
/// Token held. `block` comes from `lv_malloc`, starts with a writable
/// `*mut c_void` link, holds nothing that needs a drop, is not queued
/// already, and nothing reads it once the current deletion or replacement
/// has finished.
pub(crate) unsafe fn defer_free(block: *mut c_void) {
    let head = HEAD.0.get();
    if head == deinit_marker() {
        // SAFETY: caller contract; during `lv_deinit` nothing draws or
        // decodes any more, and the heap is still there.
        unsafe { ffi::free(block) };
        return;
    }
    // SAFETY: caller contract: the link is the block's first word.
    unsafe { block.cast::<*mut c_void>().write(head) };
    HEAD.0.set(block);
}

/// Frees every queued block. Token held, depth 0 (the drain).
pub(crate) fn free_deferred() {
    let mut block = HEAD.0.get();
    if block == deinit_marker() {
        return;
    }
    HEAD.0.set(ptr::null_mut());
    while !block.is_null() {
        // SAFETY: queued by `defer_free`: a live block whose first word is
        // the next link, and nothing uses it any more.
        unsafe {
            let next = block.cast::<*mut c_void>().read();
            ffi::free(block);
            block = next;
        }
    }
}

/// Frees the queued blocks and makes later ones free at once: called just
/// before `lv_deinit`, whose deletions retire blocks after the last drain.
pub(crate) fn begin_deinit() {
    free_deferred();
    HEAD.0.set(deinit_marker());
}

/// Blocks waiting for the next drain (tests).
#[cfg(test)]
pub(crate) fn queued() -> usize {
    let mut count = 0;
    let mut block = HEAD.0.get();
    if block == deinit_marker() {
        return 0;
    }
    while !block.is_null() {
        count += 1;
        // SAFETY: as in `free_deferred`.
        block = unsafe { block.cast::<*mut c_void>().read() };
    }
    count
}
