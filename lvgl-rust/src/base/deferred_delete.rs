//! Deferred deletion of host-owned objects (feature `deferred-delete`),
//! without the handle registry.
//!
//! A host that wraps LVGL objects of its own classes in owner types (a
//! window, an application logo) deletes the object when the owner is
//! dropped. The drop often runs while another task holds the LVGL lock, so
//! it cannot take it: it queues the object with
//! [`DeferredClass::request_delete`], and the next drain (at
//! [`Lvgl::enter`] and around [`Lvgl::timer_handler`]) deletes it.
//!
//! The queue is intrusive: each participating object carries a
//! [`DeferredLink`] (one pointer) in its instance struct, and each host
//! class has one [`DeferredClass`] (four pointers, in a `static`) holding
//! its queue, first in first out. A class is chained to the others the first time one of its
//! objects is requested; one static head (a pointer and a flag) starts
//! that chain. Nothing is allocated, and requesting calls no LVGL
//! function.
//!
//! Every access to the queues is made inside a
//! [`critical_section`](https://docs.rs/critical-section): a lock, rather
//! than atomics, because removing an object from the middle of a queue
//! (its parent was deleted first) cannot be done with loads and stores
//! alone, and the crate uses no read-modify-write so as to support
//! `thumbv6m` and `riscv32imc`. Entering and leaving the section orders the
//! queue's memory between threads (acquire and release), as a mutex does.
//! The final binary provides the implementation (`critical-section`'s
//! `std` feature on hosts, the HAL or the OS on targets). Under loom a loom
//! mutex stands in for it.
//!
//! [`Lvgl::enter`]: crate::Lvgl::enter
//! [`Lvgl::timer_handler`]: crate::Lvgl::timer_handler

use super::ffi::{self, lv_obj_t};
use crate::{
    Error, Lvgl,
    counters::{Counter, count},
};
use core::{cell::Cell, ptr, ptr::NonNull};

/// A link in a [`DeferredClass`] queue, embedded in the instance struct of
/// a host class:
///
/// ```ignore
/// #[repr(C)]
/// struct Window {
///     obj: lv_obj_t,
///     link: DeferredLink,
///     // ...
/// }
/// ```
///
/// One pointer. All zero bits (LVGL zeroes every new instance) means "not
/// queued". Only [`DeferredClass`] reads or writes it.
#[repr(transparent)]
pub struct DeferredLink(Cell<*mut lv_obj_t>);

impl DeferredLink {
    /// A link that is not queued: the value LVGL's zeroed allocation gives.
    pub const fn new() -> Self {
        Self(Cell::new(ptr::null_mut()))
    }
}

impl Default for DeferredLink {
    fn default() -> Self {
        Self::new()
    }
}

/// Value of a queued object's link when it is the last of its queue.
/// Objects are aligned to a pointer: never an object address.
fn last() -> *mut lv_obj_t {
    ptr::without_provenance_mut(1)
}

/// Value of the link of an object being deleted or destroyed: a request
/// made before its memory is freed is ignored.
fn gone() -> *mut lv_obj_t {
    ptr::without_provenance_mut(2)
}

/// The end of the chain of classes (a null `next` means "not chained").
fn last_class() -> *const DeferredClass {
    ptr::without_provenance(1)
}

/// The deferred deletions of one host class: its queue, and its place in
/// the chain the drain walks. Declare one per class, in a `static`:
///
/// ```ignore
/// // SAFETY: every object given to WINDOWS is a `Window`, whose `link`
/// // field is only used through WINDOWS.
/// static WINDOWS: DeferredClass = unsafe { DeferredClass::new(offset_of!(Window, link)) };
/// ```
///
/// The class's destructor (`destructor_cb`) must call
/// [`forget`](DeferredClass::forget): an object can die before the drain
/// reaches it (its parent was deleted, or the host deleted it at once).
///
/// Four pointers: 16 B on 32-bit targets.
pub struct DeferredClass {
    /// Offset of the [`DeferredLink`] from the start of the object.
    offset: usize,
    /// The first queued object, null when the queue is empty.
    head: Cell<*mut lv_obj_t>,
    /// The last queued object, null when the queue is empty.
    tail: Cell<*mut lv_obj_t>,
    /// The next chained class, [`last_class`] at the end, null while not
    /// chained.
    next: Cell<*const DeferredClass>,
}

// SAFETY: `head`, `tail`, `next` and the links are read and written only inside the
// critical section (`locked`), which serializes them across threads and
// orders their memory; `offset` never changes.
unsafe impl Sync for DeferredClass {}

/// The start of the chain of classes that have had requests.
struct Chain {
    first: Cell<*const DeferredClass>,
    /// Set once `deinit` starts: requests and `forget` then do nothing
    /// (LVGL frees every object, and owners dropped later hold dangling
    /// pointers).
    deinit: Cell<bool>,
}

// SAFETY: as `DeferredClass`: only accessed inside the critical section.
unsafe impl Sync for Chain {}

impl Chain {
    #[cfg_attr(loom, allow(dead_code))]
    const fn new() -> Self {
        Self {
            first: Cell::new(ptr::null()),
            deinit: Cell::new(false),
        }
    }
}

#[cfg(not(loom))]
static CHAIN: Chain = Chain::new();

/// Runs `f` inside the critical section, with the chain.
#[cfg(not(loom))]
fn locked<R>(f: impl FnOnce(&Chain) -> R) -> R {
    critical_section::with(|_| f(&CHAIN))
}

#[cfg(loom)]
loom::lazy_static! {
    static ref LOCK: loom::sync::Mutex<()> = loom::sync::Mutex::new(());
    static ref CHAIN: Chain = Chain {
        first: Cell::new(ptr::null()),
        deinit: Cell::new(false),
    };
}

#[cfg(loom)]
fn locked<R>(f: impl FnOnce(&Chain) -> R) -> R {
    let _guard = LOCK.lock().unwrap();
    f(&CHAIN)
}

impl DeferredClass {
    /// The deferred deletions of a class whose instances hold a
    /// [`DeferredLink`] `offset` bytes from their start.
    ///
    /// # Safety
    ///
    /// For every object later given to this value's methods:
    ///
    /// - it is an instance of a class whose instance struct has a
    ///   [`DeferredLink`] at `offset`, created by LVGL (which zeroes the
    ///   instance), and the pointer is the one LVGL returned (it covers
    ///   the whole instance, unlike a view's [`Obj::as_ptr`]);
    /// - its link is used through this value only: one `DeferredClass`
    ///   per class (a subclass may share its base's), the same one for
    ///   every call about a given object;
    /// - the class's destructor calls [`forget`](DeferredClass::forget)
    ///   with this value, so an object that dies before the drain leaves
    ///   the queue;
    /// - no `&mut` to the instance struct is held while the link may be
    ///   written (whenever a request can run): reach the host's fields
    ///   through raw pointers or interior mutability.
    ///
    /// [`Obj::as_ptr`]: crate::Obj::as_ptr
    pub const unsafe fn new(offset: usize) -> Self {
        Self {
            offset,
            head: Cell::new(ptr::null_mut()),
            tail: Cell::new(ptr::null_mut()),
            next: Cell::new(ptr::null()),
        }
    }

    /// The link of `obj`.
    ///
    /// # Safety
    ///
    /// `obj` is an object of this class whose memory is not freed yet
    /// (the contract of [`new`](DeferredClass::new)).
    unsafe fn link<'a>(&self, obj: *mut lv_obj_t) -> &'a Cell<*mut lv_obj_t> {
        // SAFETY: caller contract: a `DeferredLink` lies at `offset`.
        unsafe { &(*obj.cast::<u8>().add(self.offset).cast::<DeferredLink>()).0 }
    }

    /// Queues `obj` for deletion at the next drain. Needs no session and
    /// no LVGL call, never allocates and never blocks beyond the critical
    /// section: call it from an owner's `Drop`, on any thread, while
    /// another task holds the LVGL lock. Not from an interrupt handler
    /// unless the critical-section implementation masks interrupts.
    ///
    /// Requesting an object already queued, or one being deleted, does
    /// nothing. After [`Lvgl::deinit`] started, the request is ignored
    /// without touching `obj`.
    ///
    /// With exclusive access at hand, delete at once instead
    /// ([`delete_now`](DeferredClass::delete_now)).
    ///
    /// # Safety
    ///
    /// The contract of [`new`](DeferredClass::new) for `obj`. The host
    /// vouches that deleting `obj` is safe: nothing keeps a pointer to it
    /// that outlives it (it is not a part a widget keeps, such as a tile
    /// of a tile view or a tab's content), since the drain checks only
    /// what LVGL itself holds (display layers, screens being loaded).
    /// And `obj` must not be freed before this call returns: if the LVGL side can
    /// delete it while its owner lives on another thread (a parent's
    /// deletion), the class's destructor tells the owner inside
    /// `critical_section::with`, and the owner checks before requesting,
    /// inside its own `critical_section::with` (sections nest).
    pub unsafe fn request_delete(&'static self, obj: NonNull<lv_obj_t>) {
        let obj = obj.as_ptr();
        locked(|chain| {
            if chain.deinit.get() {
                return;
            }
            // SAFETY: caller contract.
            let link = unsafe { self.link(obj) };
            if !link.get().is_null() {
                return; // queued, being deleted, or destroyed
            }
            self.push(obj);
            if self.next.get().is_null() {
                let first = chain.first.get();
                self.next
                    .set(if first.is_null() { last_class() } else { first });
                chain.first.set(self);
            }
        });
    }

    /// Forgets `obj`, which LVGL is destroying: removes it from the queue
    /// if it is there, and marks it so that a request arriving before its
    /// memory is freed is ignored. Call it from the class's destructor
    /// (`destructor_cb`), which LVGL runs for every deletion path (a
    /// deletion, a parent's, `clean`, a screen deleted by a load,
    /// `lv_deinit`), so the queue never keeps a freed object.
    ///
    /// # Safety
    ///
    /// The contract of [`new`](DeferredClass::new) for `obj`, called from
    /// its class's destructor.
    pub unsafe fn forget(&self, obj: NonNull<lv_obj_t>) {
        let obj = obj.as_ptr();
        locked(|chain| {
            if chain.deinit.get() {
                return;
            }
            // SAFETY: caller contract.
            let link = unsafe { self.link(obj) };
            let next = link.get();
            link.set(gone());
            if next.is_null() || next == gone() {
                return;
            }
            // SAFETY: queued objects are live: each one leaves the queue in
            // its destructor (here) or in the drain before its deletion.
            unsafe { self.unlink(obj, next) };
        });
    }

    /// Queues the live `obj`, which is not queued, at the end. Inside the
    /// critical section.
    fn push(&self, obj: *mut lv_obj_t) {
        // SAFETY: `obj` is live (callers), and so is the queued tail.
        unsafe {
            self.link(obj).set(last());
            let tail = self.tail.get();
            if tail.is_null() {
                self.head.set(obj);
            } else {
                self.link(tail).set(obj);
            }
        }
        self.tail.set(obj);
    }

    /// Removes the queued `obj`, whose link was `next`. Inside the
    /// critical section.
    ///
    /// # Safety
    ///
    /// Every queued object of this class is live.
    unsafe fn unlink(&self, obj: *mut lv_obj_t, next: *mut lv_obj_t) {
        let successor = if next == last() {
            ptr::null_mut()
        } else {
            next
        };
        let mut previous: *mut lv_obj_t = ptr::null_mut();
        let mut current = self.head.get();
        while !current.is_null() {
            if current == obj {
                if previous.is_null() {
                    self.head.set(successor);
                } else {
                    // SAFETY: a queued object (function contract).
                    unsafe { self.link(previous) }.set(next);
                }
                if self.tail.get() == obj {
                    self.tail.set(previous);
                }
                return;
            }
            previous = current;
            // SAFETY: as above.
            let link = unsafe { self.link(current) }.get();
            current = if link == last() {
                ptr::null_mut()
            } else {
                link
            };
        }
        debug_assert!(false, "a queued object was missing from its queue");
    }

    /// Takes the first queued object, marked as being deleted. Inside the
    /// critical section.
    fn pop(&self) -> Option<*mut lv_obj_t> {
        let obj = self.head.get();
        if obj.is_null() {
            return None;
        }
        // SAFETY: queued objects are live.
        let link = unsafe { self.link(obj) };
        if link.get() == last() {
            self.head.set(ptr::null_mut());
            self.tail.set(ptr::null_mut());
        } else {
            self.head.set(link.get());
        }
        link.set(gone());
        Some(obj)
    }

    /// Queued objects. Inside the critical section.
    fn len(&self) -> usize {
        let mut count = 0;
        let mut current = self.head.get();
        while !current.is_null() && current != last() {
            count += 1;
            // SAFETY: queued objects are live.
            current = unsafe { self.link(current) }.get();
        }
        count
    }

    /// Deletes `obj` now, with exclusive access, after the checks the
    /// drain makes: [`Error::Internal`] for a display layer, [`Error::Busy`]
    /// for a screen being loaded (or the previous one of an animated load).
    /// A queued `obj` leaves its queue through the destructor's
    /// [`forget`](DeferredClass::forget).
    ///
    /// # Safety
    ///
    /// The contract of [`new`](DeferredClass::new) for `obj`, and nothing
    /// holds a pointer to `obj` that outlives it: it is the host's own
    /// object, deleted as its owner intends.
    pub unsafe fn delete_now(&self, lv: &mut Lvgl, obj: NonNull<lv_obj_t>) -> Result<(), Error> {
        let _ = lv;
        let obj = obj.as_ptr();
        super::structural::check_host_delete(obj)?;
        // SAFETY: exclusive access at depth 0 (`&mut Lvgl`), checks passed.
        unsafe { ffi::obj_delete(obj) };
        Ok(())
    }
}

/// Deletes the queued objects: called first by the drain (token held,
/// depth 0).
pub(crate) fn drain() {
    drain_with(|obj| {
        super::structural::check_host_delete(obj)?;
        count(Counter::Drained);
        // SAFETY: token held at depth 0, checks passed; the object was
        // queued, so live (`forget` removes it from the queue otherwise).
        unsafe { ffi::obj_delete(obj) };
        Ok(())
    });
}

/// Runs `delete` on each object queued when the call started, outside the
/// critical section, so its destructors can call `forget`. `Busy` queues
/// the object again for the next drain; another error drops the request.
pub(crate) fn drain_with(mut delete: impl FnMut(*mut lv_obj_t) -> Result<(), Error>) {
    let mut class = locked(|chain| {
        if chain.deinit.get() {
            ptr::null()
        } else {
            chain.first.get()
        }
    });
    while !class.is_null() && class != last_class() {
        // SAFETY: chained classes are `'static` (`request_delete`).
        let current = unsafe { &*class };
        // First in, first out: objects requested while these are deleted,
        // and objects queued again (`Busy`), come after them and wait for
        // the next drain.
        for _ in 0..locked(|_| current.len()) {
            let Some(obj) = locked(|_| current.pop()) else {
                break;
            };
            match delete(obj) {
                Ok(()) => {}
                Err(Error::Busy) => locked(|_| current.push(obj)),
                Err(_) => {
                    count(Counter::Refused);
                    // Not deleted: requests may come again.
                    // SAFETY: popped and not deleted: live.
                    locked(|_| unsafe { current.link(obj) }.set(ptr::null_mut()));
                }
            }
        }
        class = locked(|_| current.next.get());
    }
}

/// Stops the queues for good: called by `deinit` after its last drain.
pub(crate) fn begin_deinit() {
    locked(|chain| {
        chain.deinit.set(true);
        let mut class = chain.first.get();
        while !class.is_null() && class != last_class() {
            // SAFETY: chained classes are `'static`.
            let current = unsafe { &*class };
            current.head.set(ptr::null_mut());
            current.tail.set(ptr::null_mut());
            class = current.next.get();
        }
    });
}

/// Queued objects of every chained class (tests).
#[cfg(test)]
pub(crate) fn queued() -> usize {
    locked(|chain| {
        let mut count = 0;
        let mut class = chain.first.get();
        while !class.is_null() && class != last_class() {
            // SAFETY: chained classes are `'static`.
            let current = unsafe { &*class };
            count += current.len();
            class = current.next.get();
        }
        count
    })
}

/// Undoes [`begin_deinit`] (tests, which share one process).
#[cfg(all(test, not(loom)))]
pub(crate) fn end_deinit() {
    locked(|chain| chain.deinit.set(false));
}
