//! Values on LVGL's heap (feature `heap`): a header followed by a value in
//! one `lv_malloc` block, so closures and owned data need no Rust
//! allocator.

use crate::base::ffi;
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
        let Some(block) = Self::allocate() else {
            return Err(value);
        };
        // SAFETY: a fresh block for `H` and `T`, aligned to the heap: the
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

    /// Allocates the block and fills the value with zero bytes, in place:
    /// a large value never goes through the stack. `None` if the heap is
    /// full.
    ///
    /// # Safety
    ///
    /// All-zero bytes are a valid `T`.
    #[cfg_attr(not(lv_use_image), allow(dead_code))]
    pub(crate) unsafe fn new_zeroed(header: H) -> Option<Self> {
        let block = Self::allocate()?;
        // SAFETY: as in `new`; caller contract for the zeroed value.
        unsafe {
            block.as_ptr().cast::<H>().write(header);
            ptr::write_bytes(value_ptr::<H, T>(block), 0, 1);
        }
        Some(Self {
            block,
            _t: core::marker::PhantomData,
        })
    }

    /// An uninitialized block for `H` then `T`.
    fn allocate() -> Option<NonNull<u8>> {
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
        NonNull::new(unsafe { ffi::malloc(size.max(1)) }.cast::<u8>())
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
            ffi::free(block);
        }
    }
}
