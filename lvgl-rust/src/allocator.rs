//! LVGL's heap from a Rust allocator (feature `allocator`, with
//! `LV_USE_STDLIB_MALLOC LV_STDLIB_CUSTOM` in `lv_conf.h`).

use crate::heap_align::HEAP_ALIGN;
use core::alloc::{GlobalAlloc, Layout};

/// The allocator LVGL uses, installed with
/// [`lvgl_allocator!`](crate::lvgl_allocator).
///
/// # Safety
///
/// `alloc` and `realloc` return null or blocks of at least the requested
/// size aligned to a pointer (`size_of::<usize>()`), and `realloc` and
/// `free` accept any block they returned. LVGL never passes them NULL,
/// size 0 or its zero-size sentinel (`lv_mem.c`); the macro maps a
/// `realloc` of NULL to `alloc`.
pub unsafe trait LvglAllocator: Sync {
    /// A new block of `size` bytes, or null.
    fn alloc(&self, size: usize) -> *mut u8;

    /// Resizes `ptr` to `new_size`, keeping its contents; null on failure
    /// (`ptr` stays valid).
    ///
    /// # Safety
    ///
    /// `ptr` comes from this allocator and was not freed.
    unsafe fn realloc(&self, ptr: *mut u8, new_size: usize) -> *mut u8;

    /// Frees `ptr`.
    ///
    /// # Safety
    ///
    /// As for `realloc`.
    unsafe fn free(&self, ptr: *mut u8);
}

/// Adapts a Rust allocator: LVGL's `realloc` and `free` do not pass the
/// size, so each block starts with a header holding it (one pointer, which
/// keeps LVGL's blocks aligned to a pointer).
pub struct SizedHeader<A>(pub A);

const HEADER: usize = HEAP_ALIGN;

fn layout(size: usize) -> Option<Layout> {
    Layout::from_size_align(size.checked_add(HEADER)?, HEAP_ALIGN).ok()
}

// SAFETY: blocks are `HEAP_ALIGN`-aligned allocations of the header plus
// the size, handed out past the header, which records the size the layout
// of `realloc` and `free` needs.
unsafe impl<A: GlobalAlloc + Sync> LvglAllocator for SizedHeader<A> {
    fn alloc(&self, size: usize) -> *mut u8 {
        let Some(layout) = layout(size) else {
            return core::ptr::null_mut();
        };
        // SAFETY: a non-zero layout (the header).
        let base = unsafe { self.0.alloc(layout) };
        if base.is_null() {
            return base;
        }
        // SAFETY: the block starts with room for the header, aligned.
        unsafe {
            base.cast::<usize>().write(size);
            base.add(HEADER)
        }
    }

    unsafe fn realloc(&self, ptr: *mut u8, new_size: usize) -> *mut u8 {
        // SAFETY: `ptr` follows its header (caller contract).
        unsafe {
            let base = ptr.sub(HEADER);
            let Some((old, new)) = layout(base.cast::<usize>().read()).zip(layout(new_size)) else {
                return core::ptr::null_mut();
            };
            let moved = self.0.realloc(base, old, new.size());
            if moved.is_null() {
                return moved;
            }
            moved.cast::<usize>().write(new_size);
            moved.add(HEADER)
        }
    }

    unsafe fn free(&self, ptr: *mut u8) {
        // SAFETY: as above.
        unsafe {
            let base = ptr.sub(HEADER);
            if let Some(layout) = layout(base.cast::<usize>().read()) {
                self.0.dealloc(base, layout);
            }
        }
    }
}

/// Installs `$allocator` (a `static` implementing [`LvglAllocator`]) as
/// LVGL's heap: defines the functions LVGL calls with
/// `LV_USE_STDLIB_MALLOC LV_STDLIB_CUSTOM`. Use it once, in the binary.
///
/// ```ignore
/// static HEAP: SizedHeader<std::alloc::System> = SizedHeader(std::alloc::System);
/// lvgl_rust::lvgl_allocator!(HEAP);
/// ```
#[macro_export]
macro_rules! lvgl_allocator {
    ($allocator:path) => {
        const _: () = {
            use core::ffi::c_void;
            use $crate::sys;

            #[unsafe(no_mangle)]
            extern "C" fn lv_mem_init() {}

            #[unsafe(no_mangle)]
            extern "C" fn lv_mem_deinit() {}

            #[unsafe(no_mangle)]
            extern "C" fn lv_mem_add_pool(_mem: *mut c_void, _bytes: usize) -> sys::lv_mem_pool_t {
                core::ptr::null_mut()
            }

            #[unsafe(no_mangle)]
            extern "C" fn lv_mem_remove_pool(_pool: sys::lv_mem_pool_t) {}

            #[unsafe(no_mangle)]
            extern "C" fn lv_malloc_core(size: usize) -> *mut c_void {
                $crate::LvglAllocator::alloc(&$allocator, size).cast()
            }

            #[unsafe(no_mangle)]
            unsafe extern "C" fn lv_realloc_core(p: *mut c_void, new_size: usize) -> *mut c_void {
                if p.is_null() {
                    return $crate::LvglAllocator::alloc(&$allocator, new_size).cast();
                }
                // SAFETY: LVGL passes a block of this allocator.
                unsafe { $crate::LvglAllocator::realloc(&$allocator, p.cast(), new_size).cast() }
            }

            #[unsafe(no_mangle)]
            unsafe extern "C" fn lv_free_core(p: *mut c_void) {
                // SAFETY: as above; never NULL (`lv_free` checks).
                unsafe { $crate::LvglAllocator::free(&$allocator, p.cast()) }
            }

            #[unsafe(no_mangle)]
            unsafe extern "C" fn lv_mem_monitor_core(mon: *mut sys::lv_mem_monitor_t) {
                // SAFETY: LVGL passes a monitor to fill; zeroes: unknown.
                unsafe { mon.write(core::mem::zeroed()) }
            }

            #[unsafe(no_mangle)]
            extern "C" fn lv_mem_test_core() -> sys::lv_result_t {
                sys::lv_result_t_LV_RESULT_OK
            }
        };
    };
}
