//! The handle registry: one zeroed static in `.bss`.
//!
//! Every field has a single writer. `Cell` fields are read and written only by
//! the token holder (a [`Session`](crate::Session), or a trampoline called by
//! it). Code without a token (`Owned::drop`, `tick_inc`) touches only the
//! atomics `req`, `any_req` and `ticks`, with stores, never read-modify-write.

use super::sync::{AtomicBool, AtomicU8, AtomicU32};
#[cfg(not(lvgl_rust_views_only))]
use crate::ScreenAnim;
use core::cell::Cell;

#[cfg(not(lvgl_rust_views_only))]
use super::{
    ffi,
    sync::{Ordering, fence},
};
#[cfg(not(lvgl_rust_views_only))]
use crate::{Error, config::HANDLE_SLOTS as N};
#[cfg(not(lvgl_rust_views_only))]
use core::{ffi::c_void, ptr, ptr::NonNull};

/// Defines a `const fn`, except under loom, whose atomics have no `const`
/// constructors.
macro_rules! const_fn {
    ($(#[$m:meta])* $v:vis fn $name:ident() -> $ret:ty $body:block) => {
        #[cfg(not(loom))]
        $(#[$m])* $v const fn $name() -> $ret $body
        #[cfg(loom)]
        $(#[$m])* $v fn $name() -> $ret $body
    };
}

#[cfg(not(lvgl_rust_views_only))]
pub(crate) mod meta {
    pub(crate) const STATE: u8 = 0b11;
    pub(crate) const FREE: u8 = 0;
    pub(crate) const LIVE: u8 = 1;
    pub(crate) const DEAD: u8 = 2;
    /// An `Owned` exists for the slot: the slot is freed only once its
    /// request has been observed.
    pub(crate) const OWNED: u8 = 1 << 2;
    /// Minted by a deferred request: released when the request completes,
    /// unless a handle was handed out meanwhile.
    pub(crate) const TEMP: u8 = 1 << 3;
    pub(crate) const KIND: u8 = 0b11 << 4;
    pub(crate) const P_DELETE: u8 = 1 << 6;
    pub(crate) const P_CLEAN: u8 = 1 << 7;
}
#[cfg(not(lvgl_rust_views_only))]
use meta::*;

/// What a slot tracks (the KIND bits of `meta`). Timers and groups take the
/// next values.
#[cfg(not(lvgl_rust_views_only))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub(crate) enum Kind {
    Obj = 0,
}

#[cfg(not(lvgl_rust_views_only))]
impl Kind {
    const fn bits(self) -> u8 {
        (self as u8) << 4
    }
}

/// 8 B on 32-bit targets, 16 B on 64-bit ones.
#[cfg(not(lvgl_rust_views_only))]
#[repr(C)]
pub(crate) struct Slot {
    /// The tracked pointer while LIVE or DEAD, the next free index + 1 while
    /// FREE.
    ptr: Cell<*mut c_void>,
    generation: Cell<u16>,
    meta: Cell<u8>,
    /// Stored by `Owned::drop` without a token, cleared by the drain.
    pub(crate) req: AtomicU8,
}

#[cfg(not(lvgl_rust_views_only))]
impl Slot {
    const_fn! {
        fn new() -> Self {
            Self {
                ptr: Cell::new(ptr::null_mut()),
                generation: Cell::new(0),
                meta: Cell::new(0),
                req: AtomicU8::new(0),
            }
        }
    }

    pub(crate) fn ptr(&self) -> *mut c_void {
        self.ptr.get()
    }

    pub(crate) fn state(&self) -> u8 {
        self.meta.get() & STATE
    }

    pub(crate) fn has(&self, bits: u8) -> bool {
        self.meta.get() & bits != 0
    }

    pub(crate) fn set(&self, bits: u8) {
        self.meta.set(self.meta.get() | bits)
    }

    pub(crate) fn clear(&self, bits: u8) {
        self.meta.set(self.meta.get() & !bits)
    }

    fn is_live(&self, kind: Kind) -> bool {
        self.meta.get() & (STATE | KIND) == LIVE | kind.bits()
    }

    fn set_next(&self, next: u16) {
        self.ptr.set(ptr::without_provenance_mut(next as usize))
    }

    fn next(&self) -> u16 {
        self.ptr.get().addr() as u16
    }
}

/// 24 B on every target with slots, 8 B in views-only mode.
#[repr(C)]
pub(crate) struct Header {
    /// Milliseconds returned to LVGL by the crate's tick callback.
    pub(crate) ticks: AtomicU32,
    #[cfg(not(lvgl_rust_views_only))]
    hwm: Cell<u16>,
    #[cfg(not(lvgl_rust_views_only))]
    head: Cell<u16>,
    #[cfg(not(lvgl_rust_views_only))]
    tail: Cell<u16>,
    /// Pending `load_screen_later`: slot index + 1, 0 when none.
    #[cfg(not(lvgl_rust_views_only))]
    pub(crate) load: Cell<u16>,
    #[cfg(not(lvgl_rust_views_only))]
    pub(crate) load_anim: Cell<ScreenAnim>,
    /// Callback nesting depth.
    pub(crate) depth: Cell<u8>,
    /// Some slot may have a request to act on.
    #[cfg(not(lvgl_rust_views_only))]
    pub(crate) any_req: AtomicBool,
    /// A session is active.
    pub(crate) entered: AtomicBool,
    /// 0 before `init`, 1 after it, 2 after `deinit`.
    pub(crate) init: AtomicU8,
}

#[repr(C)]
pub(crate) struct Registry {
    pub(crate) hdr: Header,
    #[cfg(not(lvgl_rust_views_only))]
    slots: [Slot; N],
}

// SAFETY: the `Cell` fields are accessed only while a session exists, or by
// trampolines called by its holder; sessions are ordered by happens-before
// through the `LvglAccess` contract. Other threads touch only the atomics.
unsafe impl Sync for Registry {}

impl Registry {
    const_fn! {
        fn new() -> Self {
            Self {
                hdr: Header {
                    ticks: AtomicU32::new(0),
                    #[cfg(not(lvgl_rust_views_only))]
                    hwm: Cell::new(0),
                    #[cfg(not(lvgl_rust_views_only))]
                    head: Cell::new(0),
                    #[cfg(not(lvgl_rust_views_only))]
                    tail: Cell::new(0),
                    #[cfg(not(lvgl_rust_views_only))]
                    load: Cell::new(0),
                    #[cfg(not(lvgl_rust_views_only))]
                    load_anim: Cell::new(ScreenAnim::NONE),
                    depth: Cell::new(0),
                    #[cfg(not(lvgl_rust_views_only))]
                    any_req: AtomicBool::new(false),
                    entered: AtomicBool::new(false),
                    init: AtomicU8::new(0),
                },
                #[cfg(all(not(lvgl_rust_views_only), not(loom)))]
                slots: [const { Slot::new() }; N],
                #[cfg(all(not(lvgl_rust_views_only), loom))]
                slots: core::array::from_fn(|_| Slot::new()),
            }
        }
    }
}

#[cfg(not(loom))]
static REG: Registry = Registry::new();

#[cfg(not(loom))]
pub(crate) fn reg() -> &'static Registry {
    &REG
}

#[cfg(loom)]
loom::lazy_static! {
    static ref REG: Registry = Registry::new();
}

#[cfg(loom)]
pub(crate) fn reg() -> &'static Registry {
    &REG
}

// Loom's atomics are larger than core's: sizes are checked without loom.
#[cfg(all(not(lvgl_rust_views_only), not(loom)))]
const _: () = {
    assert!(N.is_power_of_two() && N <= 32_768);
    assert!(size_of::<Slot>() == 2 * size_of::<usize>());
    assert!(size_of::<Header>() == 24);
};
#[cfg(all(lvgl_rust_views_only, not(loom)))]
const _: () = assert!(size_of::<Header>() == 8);

/// Slot index encoded in a raw handle (`generation << 16 | index + 1`). The
/// mask keeps it in bounds without a panic path.
#[cfg(not(lvgl_rust_views_only))]
pub(crate) fn index(raw: u32) -> usize {
    (raw as usize & 0xFFFF).wrapping_sub(1) & (N - 1)
}

#[cfg(not(lvgl_rust_views_only))]
impl Registry {
    pub(crate) fn slot(&self, index: usize) -> &Slot {
        &self.slots[index & (N - 1)]
    }

    /// Slots ever used: those at or above it are implicitly free.
    pub(crate) fn high_water_mark(&self) -> usize {
        self.hdr.hwm.get() as usize
    }

    fn raw(&self, index: usize) -> u32 {
        (self.slot(index).generation.get() as u32) << 16 | (index as u32 + 1)
    }

    /// Returns the slot tracking `p`, taking one if needed (`fresh`).
    pub(crate) fn mint(&self, p: *mut c_void, kind: Kind) -> Result<Minted, Error> {
        if kind == Kind::Obj {
            // SAFETY: `p` is a live object (callers hold a view of it).
            let id = unsafe { ffi::obj_id(p.cast()) };
            if let Some(index) = id.checked_sub(1).filter(|&i| i < N) {
                let slot = self.slot(index);
                if slot.ptr.get() == p && slot.is_live(kind) {
                    return Ok(Minted {
                        index,
                        raw: self.raw(index),
                        fresh: false,
                    });
                }
            }
        }
        let index = self.alloc().ok_or(Error::RegistryFull)?;
        let slot = self.slot(index);
        slot.ptr.set(p);
        slot.meta.set(LIVE | kind.bits());
        if kind == Kind::Obj {
            // SAFETY: as above; only the registry writes `obj->id`.
            unsafe { ffi::obj_set_id(p.cast(), index + 1) };
        }
        Ok(Minted {
            index,
            raw: self.raw(index),
            fresh: true,
        })
    }

    /// Checks a raw handle and returns its pointer. `class` (objects only,
    /// null to skip) must be the object's class or one of its bases.
    #[inline(never)]
    pub(crate) fn resolve(
        &self,
        raw: u32,
        kind: Kind,
        class: *const ffi::lv_obj_class_t,
    ) -> Result<NonNull<c_void>, Error> {
        let slot = self.slot(index(raw));
        if slot.generation.get() != (raw >> 16) as u16
            || !slot.is_live(kind)
            || slot.req.load(Ordering::Relaxed) != 0
            || slot.has(P_DELETE)
        {
            return Err(Error::Dead);
        }
        let p = slot.ptr.get();
        if kind == Kind::Obj {
            let o = p.cast::<ffi::lv_obj_t>();
            // SAFETY: a LIVE object slot points to an allocated object (R).
            unsafe {
                if self.hdr.depth.get() != 0 && super::view::in_deletion(o) {
                    return Err(Error::Dead);
                }
                if !class.is_null()
                    && ffi::obj_class_of(o) != class
                    && !ffi::obj_has_class(o, class)
                {
                    return Err(Error::Dead);
                }
            }
        }
        // SAFETY: LIVE slots never hold a null pointer.
        Ok(unsafe { NonNull::new_unchecked(p) })
    }

    /// Fresh slots first, then the oldest free one.
    fn alloc(&self) -> Option<usize> {
        let hwm = self.hdr.hwm.get();
        if (hwm as usize) < N {
            self.hdr.hwm.set(hwm + 1);
            return Some(hwm as usize);
        }
        let head = self.hdr.head.get();
        let index = (head as usize).checked_sub(1)?;
        let next = self.slot(index).next();
        self.hdr.head.set(next);
        if next == 0 {
            self.hdr.tail.set(0);
        }
        Some(index)
    }

    /// Records a dropped `Owned` (no token needed). Paired with the drain's
    /// fence: either the drain's scan sees `req`, or `any_req` stays set for
    /// the next drain.
    pub(crate) fn request_drop(&self, index: usize) {
        self.slot(index).req.store(1, Ordering::Relaxed);
        fence(Ordering::SeqCst);
        self.hdr.any_req.store(true, Ordering::Relaxed);
    }

    /// Frees a slot exactly once: new generation, then the tail of the FIFO.
    pub(crate) fn free(&self, index: usize) {
        let slot = self.slot(index);
        slot.meta.set(FREE);
        slot.req.store(0, Ordering::Relaxed);
        slot.generation.set(slot.generation.get().wrapping_add(1));
        slot.set_next(0);
        let tail = self.hdr.tail.get();
        match (tail as usize).checked_sub(1) {
            Some(last) => self.slot(last).set_next(index as u16 + 1),
            None => self.hdr.head.set(index as u16 + 1),
        }
        self.hdr.tail.set(index as u16 + 1);
    }

    /// Releases a TEMP slot whose object stays alive.
    pub(crate) fn release(&self, index: usize) {
        let slot = self.slot(index);
        if slot.state() == LIVE {
            // SAFETY: a LIVE object slot points to an allocated object (R).
            unsafe { ffi::obj_set_id(slot.ptr.get().cast(), 0) };
        }
        self.free(index);
    }

    /// The OBJ_ID free hook: `o` is being destroyed and `id` is its id.
    pub(crate) fn on_free(&self, o: *mut ffi::lv_obj_t, id: usize) {
        let Some(index) = id.checked_sub(1).filter(|&i| i < N) else {
            return; // untracked object
        };
        let slot = self.slot(index);
        if slot.ptr.get() != o.cast() || !slot.is_live(Kind::Obj) {
            return; // foreign or garbage id
        }
        if slot.has(OWNED) {
            // The `Owned` still refers to the slot: keep it until the drain
            // observes its request.
            slot.meta
                .set((slot.meta.get() & !(STATE | P_DELETE | P_CLEAN)) | DEAD);
        } else {
            self.free(index);
        }
        if self.hdr.load.get() as usize == index + 1 {
            self.hdr.load.set(0);
        }
    }
}

/// Result of [`Registry::mint`].
#[cfg(not(lvgl_rust_views_only))]
pub(crate) struct Minted {
    pub(crate) index: usize,
    pub(crate) raw: u32,
    /// The slot was taken by this call.
    pub(crate) fresh: bool,
}
