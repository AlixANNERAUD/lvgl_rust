//! The handle registry: one zeroed static in `.bss`.
//!
//! With the `growable-registry` feature (cfg `lvgl_rust_growable`), slots
//! beyond the static ones live in chunks taken from the global allocator:
//! chunk `k` holds slots `N·2^k` to `N·2^(k+1) - 1`, so the capacity doubles
//! with each chunk, up to the 65,535 slots raw handles can address. Chunks
//! never move and are never freed, not even by `deinit`: an `Owned` may store
//! into its slot's `req` at any time. That leaks at most `65,535 - N` slots
//! (524,024 B on 32-bit targets with N = 32, 1,048,048 B on 64-bit ones). The
//! global allocator must therefore outlive LVGL: it must not take its memory
//! from LVGL's heap, which `deinit` releases.
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

#[cfg(lvgl_rust_growable)]
use super::sync::AtomicPtr;

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
    /// Timers never get cleaned: on a timer slot, `P_CLEAN` marks a
    /// `timer_boxed` closure for the drain to free.
    #[cfg(feature = "heap")]
    pub(crate) const TIMER_BOXED: u8 = P_CLEAN;
}
#[cfg(not(lvgl_rust_views_only))]
use meta::*;

/// What a slot tracks (the KIND bits of `meta`).
#[cfg(not(lvgl_rust_views_only))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub(crate) enum Kind {
    Obj = 0,
    Timer = 1,
    Group = 2,
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

    /// Whether the slot tracks (or tracked) a timer.
    pub(crate) fn is_timer(&self) -> bool {
        self.meta.get() & KIND == Kind::Timer.bits()
    }

    /// Whether the slot tracks (or tracked) a group.
    pub(crate) fn is_group(&self) -> bool {
        self.meta.get() & KIND == Kind::Group.bits()
    }

    pub(crate) fn is_live(&self, kind: Kind) -> bool {
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
    #[cfg(lvgl_rust_growable)]
    growth: Growth,
}

/// The slots beyond the static ones: `CHUNKS` pointers and one slot, 56 B on
/// 32-bit targets with N = 16 (12 pointers of 4 B and a slot of 8 B), 52 B
/// with the default 32.
#[cfg(lvgl_rust_growable)]
#[repr(C)]
struct Growth {
    /// Chunk `k` (slots `N·2^k` to `N·2^(k+1) - 1`, see [`chunk_len`]), null
    /// until published. Written once, by the token holder: see
    /// [`Registry::grow`] for the ordering.
    chunks: [AtomicPtr<Slot>; CHUNKS],
    /// What indices of no published chunk resolve to: always FREE, so a
    /// handle to it is dead. No index the registry hands out reaches it.
    sentinel: Slot,
}

#[cfg(lvgl_rust_growable)]
impl Growth {
    const_fn! {
        fn new() -> Self {
            Self {
                #[cfg(not(loom))]
                chunks: [const { AtomicPtr::new(ptr::null_mut()) }; CHUNKS],
                #[cfg(loom)]
                chunks: core::array::from_fn(|_| AtomicPtr::new(ptr::null_mut())),
                sentinel: Slot::new(),
            }
        }
    }
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
                #[cfg(lvgl_rust_growable)]
                growth: Growth::new(),
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
#[cfg(all(lvgl_rust_growable, not(loom)))]
const _: () = assert!(
    size_of::<Growth>() == CHUNKS * size_of::<usize>() + size_of::<Slot>()
        && size_of::<Registry>()
            == size_of::<Header>() + (N + 1) * size_of::<Slot>() + CHUNKS * size_of::<usize>()
);

/// Slot index encoded in a raw handle (`generation << 16 | index + 1`). The
/// mask keeps it in bounds without a panic path.
#[cfg(all(not(lvgl_rust_views_only), not(lvgl_rust_growable)))]
pub(crate) fn index(raw: u32) -> usize {
    (raw as usize & 0xFFFF).wrapping_sub(1) & (N - 1)
}

/// Slot index encoded in a raw handle (`generation << 16 | index + 1`).
/// [`Registry::slot`] maps the indices of no published chunk to a FREE slot.
#[cfg(lvgl_rust_growable)]
pub(crate) fn index(raw: u32) -> usize {
    (raw as usize & 0xFFFF).wrapping_sub(1)
}

/// The indices an object id or a raw handle can name.
#[cfg(all(not(lvgl_rust_views_only), not(lvgl_rust_growable)))]
const INDEX_LIMIT: usize = N;
#[cfg(lvgl_rust_growable)]
const INDEX_LIMIT: usize = LIMIT;

/// Slots raw handles can address: their low half holds `index + 1`.
#[cfg(lvgl_rust_growable)]
pub(crate) const LIMIT: usize = 0xFFFF;
/// `N = 2^SHIFT`.
#[cfg(lvgl_rust_growable)]
const SHIFT: u32 = N.trailing_zeros();
/// Chunks needed to reach `LIMIT`: 16 - log2(N), 1 to 16.
#[cfg(lvgl_rust_growable)]
pub(crate) const CHUNKS: usize = 16 - SHIFT as usize;

/// The chunk holding slot `index` (at least `2^shift`) and its offset there,
/// when the static part has `2^shift` slots: chunk `k` starts at
/// `2^(shift + k)`, so `k = log2(index) - shift`. Wrapping arithmetic: no
/// panic path for indices out of range (the chunk is then out of range too).
#[cfg(lvgl_rust_growable)]
pub(crate) const fn locate(index: usize, shift: u32) -> (usize, usize) {
    let log2 = (usize::BITS - 1).wrapping_sub(index.leading_zeros());
    let chunk = log2.wrapping_sub(shift) as usize;
    (chunk, index.wrapping_sub(1usize.wrapping_shl(log2)))
}

/// Slots of chunk `chunk`: as many as before it, `2^(shift + chunk)`, except
/// the last one, which stops at `LIMIT`.
#[cfg(lvgl_rust_growable)]
pub(crate) const fn chunk_len(chunk: usize, shift: u32) -> usize {
    let start = 1usize.wrapping_shl(shift + chunk as u32);
    if start > LIMIT - start {
        LIMIT - start
    } else {
        start
    }
}

/// The chunk that starts at `hwm` once every slot below it is in use, or
/// `None` at `LIMIT`.
#[cfg(lvgl_rust_growable)]
pub(crate) const fn next_chunk(hwm: usize, shift: u32) -> Option<usize> {
    if hwm >= LIMIT {
        None
    } else {
        Some(locate(hwm, shift).0)
    }
}

/// Slots addressable once `hwm` slots were used: `2^shift`, doubled by each
/// chunk. A chunk is taken only when every slot is in use, so the chunks
/// published are exactly those below `hwm`.
#[cfg(lvgl_rust_growable)]
pub(crate) const fn capacity(hwm: usize, shift: u32) -> usize {
    let static_slots = 1usize << shift;
    let total = if hwm <= static_slots {
        static_slots
    } else {
        hwm.next_power_of_two()
    };
    if total > LIMIT { LIMIT } else { total }
}

/// Makes the next growth fail as an allocator returning null would.
#[cfg(all(test, lvgl_rust_growable))]
pub(crate) static FAIL_GROWTH: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

#[cfg(not(lvgl_rust_views_only))]
impl Registry {
    #[cfg(not(lvgl_rust_growable))]
    pub(crate) fn slot(&self, index: usize) -> &Slot {
        &self.slots[index & (N - 1)]
    }

    #[cfg(lvgl_rust_growable)]
    pub(crate) fn slot(&self, index: usize) -> &Slot {
        match self.slots.get(index) {
            Some(slot) => slot,
            None => self.chunk_slot(index),
        }
    }

    /// A slot of a chunk, or the FREE sentinel for an index of no published
    /// chunk. Kept out of line: the static slots stay a bounds check away.
    #[cfg(lvgl_rust_growable)]
    #[inline(never)]
    fn chunk_slot(&self, index: usize) -> &Slot {
        let (chunk, offset) = locate(index, SHIFT);
        let start = match self.growth.chunks.get(chunk) {
            // Acquire: pairs with the Release of `grow`.
            Some(cell) if index < LIMIT => cell.load(Ordering::Acquire),
            _ => ptr::null_mut(),
        };
        if start.is_null() {
            return &self.growth.sentinel;
        }
        // SAFETY: a published chunk holds `chunk_len(chunk)` initialized
        // slots, never moved or freed; `index < LIMIT` keeps `offset` below
        // that length.
        unsafe { &*start.add(offset) }
    }

    /// Slots that can be taken without growing.
    #[cfg(not(lvgl_rust_growable))]
    fn capacity(&self) -> usize {
        N
    }

    /// Slots that can be taken without growing.
    #[cfg(lvgl_rust_growable)]
    pub(crate) fn capacity(&self) -> usize {
        capacity(self.high_water_mark(), SHIFT)
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
            if let Some(index) = id.checked_sub(1).filter(|&i| i < INDEX_LIMIT) {
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
        let index = self.alloc()?;
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

    /// Fresh slots first, then the oldest free one, then (with growth) the
    /// first slot of a new chunk.
    fn alloc(&self) -> Result<usize, Error> {
        let hwm = self.hdr.hwm.get();
        if (hwm as usize) < self.capacity() {
            self.hdr.hwm.set(hwm + 1);
            return Ok(hwm as usize);
        }
        let head = self.hdr.head.get();
        let Some(index) = (head as usize).checked_sub(1) else {
            return self.grow(hwm);
        };
        let next = self.slot(index).next();
        self.hdr.head.set(next);
        if next == 0 {
            self.hdr.tail.set(0);
        }
        Ok(index)
    }

    #[cfg(not(lvgl_rust_growable))]
    fn grow(&self, _: u16) -> Result<usize, Error> {
        Err(Error::RegistryFull)
    }

    /// Every slot is in use (`hwm` is the capacity): publishes the next
    /// chunk and takes its first slot.
    ///
    /// Publication: the chunk is fully initialized (zeroed: FREE,
    /// generation 0, no request) before its pointer is stored with Release,
    /// and readers load it with Acquire. The token holder reads its own
    /// stores, or those of an earlier holder, ordered by the `LvglAccess`
    /// contract. Code without a token (`Owned::drop`, through
    /// `request_drop`) reads only the chunk of an `Owned`, which was minted
    /// under the token after the store; moving the `Owned` to another
    /// thread synchronizes with that thread (as `Send` requires: a channel,
    /// a spawn, a lock), so the store happens before the load and the load
    /// sees the pointer (it is written once: no later value exists). Loads
    /// and stores only, no read-modify-write: this works without
    /// compare-and-swap.
    #[cfg(lvgl_rust_growable)]
    #[cold]
    fn grow(&self, hwm: u16) -> Result<usize, Error> {
        let index = hwm as usize;
        let Some((chunk, cell)) = next_chunk(index, SHIFT)
            .and_then(|chunk| Some((chunk, self.growth.chunks.get(chunk)?)))
        else {
            return Err(Error::RegistryFull);
        };
        let len = chunk_len(chunk, SHIFT);
        // SAFETY: a nonzero size (a chunk has at least one slot) of at most
        // 32,767 slots of at most 16 B, aligned for `Slot`.
        let layout = unsafe {
            alloc::alloc::Layout::from_size_align_unchecked(
                len * size_of::<Slot>(),
                align_of::<Slot>(),
            )
        };
        #[cfg(test)]
        if FAIL_GROWTH.load(core::sync::atomic::Ordering::Relaxed) {
            FAIL_GROWTH.store(false, core::sync::atomic::Ordering::Relaxed);
            return Err(Error::OutOfMemory);
        }
        // SAFETY: nonzero size. All-zero bytes are a FREE slot: null
        // pointer, generation 0, no flags, no request.
        let start = unsafe { alloc::alloc::alloc_zeroed(layout) }.cast::<Slot>();
        if start.is_null() {
            return Err(Error::OutOfMemory);
        }
        // Loom's atomics are not plain bytes: construct them.
        #[cfg(loom)]
        for offset in 0..len {
            // SAFETY: in the block just allocated; the zeroes need no drop.
            unsafe { start.add(offset).write(Slot::new()) };
        }
        cell.store(start, Ordering::Release);
        self.hdr.hwm.set(hwm + 1);
        Ok(index)
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
        if slot.state() == LIVE && !slot.is_timer() && !slot.is_group() {
            // SAFETY: a LIVE object slot points to an allocated object (R).
            unsafe { ffi::obj_set_id(slot.ptr.get().cast(), 0) };
        }
        self.free(index);
    }

    /// The timer or group of the slot was deleted: the slot stays DEAD
    /// while an `Owned` refers to it, and is freed otherwise.
    pub(crate) fn resource_deleted(&self, index: usize) {
        let slot = self.slot(index);
        if slot.has(OWNED) {
            slot.meta
                .set((slot.meta.get() & !(STATE | P_DELETE | P_CLEAN)) | DEAD);
        } else {
            self.free(index);
        }
    }

    /// The slot of a live timer, by its pointer (timers have no id): a scan
    /// up to the high-water mark, only when a timer callback asks.
    pub(crate) fn find_timer(&self, p: *mut c_void) -> Option<usize> {
        (0..self.high_water_mark()).find(|&i| {
            let slot = self.slot(i);
            slot.ptr.get() == p && slot.is_live(Kind::Timer)
        })
    }

    /// The raw handle of a slot.
    pub(crate) fn handle_raw(&self, index: usize) -> u32 {
        self.raw(index)
    }

    /// The OBJ_ID free hook: `o` is being destroyed and `id` is its id.
    pub(crate) fn on_free(&self, o: *mut ffi::lv_obj_t, id: usize) {
        let Some(index) = id.checked_sub(1).filter(|&i| i < INDEX_LIMIT) else {
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
