//! Values on LVGL's heap (feature `heap`): a header followed by a value in
//! one `lv_malloc` block, so closures and owned data need no Rust
//! allocator.

use crate::sys;
use core::{
    mem::{align_of, size_of},
    ptr::{self, NonNull},
};

pub(crate) use crate::heap_align::HEAP_ALIGN;

/// The value's address in a block holding `H` then `T`.
fn value_ptr<H, T>(block: NonNull<u8>) -> *mut T {
    let start = block.as_ptr().wrapping_add(size_of::<H>());
    let offset = start.align_offset(align_of::<T>());
    start.wrapping_add(offset).cast()
}

/// An `H` and a `T` in one block of LVGL's heap; the block pointer is what
/// LVGL stores and what is freed.
pub(crate) struct LvBox<H, T> {
    block: NonNull<u8>,
    _t: core::marker::PhantomData<(H, T)>,
}

impl<H, T> LvBox<H, T> {
    /// Allocates and fills the block; gives the value back on failure.
    pub(crate) fn new(header: H, value: T) -> Result<Self, T> {
        const {
            assert!(
                align_of::<H>() <= HEAP_ALIGN,
                "box headers align to the heap"
            )
        };
        // Blocks align to the heap: a `T` no more aligned sits at a fixed
        // offset; a more aligned one needs up to `align - 1` more bytes.
        let size = if align_of::<T>() <= HEAP_ALIGN {
            size_of::<H>().next_multiple_of(align_of::<T>()) + size_of::<T>()
        } else {
            size_of::<H>() + align_of::<T>() - 1 + size_of::<T>()
        };
        // SAFETY: under the token (every caller has shared access).
        let block = unsafe { sys::lv_malloc(size.max(1)) }.cast::<u8>();
        let Some(block) = NonNull::new(block) else {
            return Err(value);
        };
        // SAFETY: a fresh block of `size` bytes, aligned to the heap: the
        // header at its start, the value aligned within it.
        unsafe {
            block.as_ptr().cast::<H>().write(header);
            value_ptr::<H, T>(block).write(value);
        }
        Ok(Self {
            block,
            _t: core::marker::PhantomData,
        })
    }

    /// The block pointer, for LVGL (`user_data`).
    pub(crate) fn into_raw(self) -> *mut core::ffi::c_void {
        self.block.as_ptr().cast()
    }

    /// The header of a block made by [`LvBox::new`].
    pub(crate) fn header(block: *mut core::ffi::c_void) -> *mut H {
        block.cast()
    }

    /// The value of a block made by [`LvBox::new`].
    pub(crate) fn value(block: *mut core::ffi::c_void) -> *mut T {
        match NonNull::new(block.cast::<u8>()) {
            Some(block) => value_ptr::<H, T>(block),
            None => ptr::null_mut(),
        }
    }

    /// Drops the header and the value, and frees the block.
    ///
    /// # Safety
    ///
    /// `block` comes from `into_raw` of an `LvBox<H, T>`, under the token,
    /// and nothing uses it any more.
    pub(crate) unsafe fn free(block: *mut core::ffi::c_void) {
        // SAFETY: caller contract.
        unsafe {
            ptr::drop_in_place(Self::value(block));
            ptr::drop_in_place(Self::header(block));
            sys::lv_free(block);
        }
    }
}
