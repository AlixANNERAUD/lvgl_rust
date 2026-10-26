//! Async events: handlers that record events into a static queue, read
//! from async code with [`next_event`]. Queues are touched only under the
//! token: no atomics, no critical section, no compare-and-swap.

use crate::{Error, Event, Lvgl, LvglAccess, Obj, enums::EventCode, event::dispatch, sys};
#[cfg(all(feature = "heap", not(lvgl_rust_views_only)))]
use crate::{Handle, heap::LvBox};
use core::{
    cell::UnsafeCell,
    future::Future,
    mem::MaybeUninit,
    pin::Pin,
    task::{Context, Poll, Waker},
};

const CLOSED: u8 = 1;
const OVERFLOW: u8 = 1 << 1;
const RX_TAKEN: u8 = 1 << 2;
/// An owned queue whose object was deleted while a frame was recording into
/// it: the outermost frame frees it.
#[cfg(all(feature = "heap", not(lvgl_rust_views_only)))]
const DOOMED: u8 = 1 << 3;

/// What a receiver returns.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Recv<E> {
    /// The oldest recorded event.
    Event(E),
    /// Nothing recorded yet.
    Empty,
    /// The subscribed object was deleted (sticky).
    Closed,
}

/// The receiving end of an event queue.
pub trait Receiver {
    /// The recorded event type.
    type Item;
    /// Pops the oldest event.
    fn try_recv(&self, lv: &Lvgl) -> Recv<Self::Item>;
    /// Wakes `waker` at the next event or when the queue closes.
    fn register(&self, lv: &Lvgl, waker: &Waker);
}

struct EventQueue<E, const N: usize> {
    head: u8,
    len: u8,
    flags: u8,
    /// Frames recording into an owned queue (it is freed only at 0). In
    /// the padding after the flags: static queues pay nothing for it.
    #[cfg_attr(
        not(all(feature = "heap", not(lvgl_rust_views_only))),
        allow(dead_code)
    )]
    busy: u8,
    waker: Option<Waker>,
    capture: fn(&Event<'_>) -> Option<E>,
    buf: [MaybeUninit<E>; N],
}

/// A queue of `N` events of type `E` in a `static`, filled by
/// [`Obj::subscribe`] through `capture`, which runs in the handler and
/// chooses what to record. Never freed: deleting a subscribed object closes
/// it. Overflow drops the newest event.
pub struct StaticQueue<E, const N: usize>(UnsafeCell<EventQueue<E, N>>);

// SAFETY: touched only under the token (a handler, or a receiver given
// `&Lvgl`); `E: Send` because events cross threads with the token.
unsafe impl<E: Copy + Send, const N: usize> Sync for StaticQueue<E, N> {}

impl<E: Copy + Send + 'static, const N: usize> StaticQueue<E, N> {
    /// An empty queue recording what `capture` returns for each event.
    pub const fn new(capture: fn(&Event<'_>) -> Option<E>) -> Self {
        const { assert!(N >= 1 && N <= 255, "a queue holds 1 to 255 events") };
        Self(UnsafeCell::new(EventQueue {
            head: 0,
            len: 0,
            flags: 0,
            busy: 0,
            waker: None,
            capture,
            buf: [const { MaybeUninit::uninit() }; N],
        }))
    }

    /// The single receiver of the queue (`None` once taken).
    pub fn receiver(&'static self, lv: &Lvgl) -> Option<StaticRx<E, N>> {
        let _ = lv;
        // SAFETY: under the token; a short borrow.
        let q = unsafe { &mut *self.0.get() };
        if q.flags & RX_TAKEN != 0 {
            return None;
        }
        q.flags |= RX_TAKEN;
        Some(StaticRx(self))
    }

    /// Whether events were dropped because the queue was full; clears the
    /// flag.
    pub fn take_overflow(&'static self, lv: &Lvgl) -> bool {
        let _ = lv;
        // SAFETY: under the token.
        unsafe { take_overflow(self.0.get()) }
    }
}

/// The receiver of a [`StaticQueue`]: one per queue, not `Clone`.
pub struct StaticRx<E: 'static, const N: usize>(&'static StaticQueue<E, N>);

impl<E: Copy + Send + 'static, const N: usize> Receiver for StaticRx<E, N> {
    type Item = E;

    fn try_recv(&self, lv: &Lvgl) -> Recv<E> {
        let _ = lv;
        // SAFETY: under the token.
        unsafe { pop(self.0.0.get()) }
    }

    fn register(&self, lv: &Lvgl, waker: &Waker) {
        let _ = lv;
        // SAFETY: under the token.
        unsafe { register(self.0.0.get(), waker) }
    }
}

/// Pops the oldest event of `q`.
///
/// # Safety
///
/// Under the token; `q` is a live queue with no reference into it.
unsafe fn pop<E: Copy, const N: usize>(q: *mut EventQueue<E, N>) -> Recv<E> {
    // SAFETY: caller contract; no user code runs during the borrow.
    let q = unsafe { &mut *q };
    if q.len == 0 {
        return if q.flags & CLOSED != 0 {
            Recv::Closed
        } else {
            Recv::Empty
        };
    }
    let head = q.head as usize;
    // SAFETY: `head` is below `N` and holds a pushed event.
    let event = unsafe { q.buf.get_unchecked(head).assume_init() };
    q.head = if head + 1 == N { 0 } else { head as u8 + 1 };
    q.len -= 1;
    Recv::Event(event)
}

/// Makes `waker` the one woken at the next event of `q`.
///
/// # Safety
///
/// As for [`pop`].
unsafe fn register<E, const N: usize>(q: *mut EventQueue<E, N>, waker: &Waker) {
    // SAFETY: caller contract; the new waker is cloned before the borrow
    // and the old one dropped after it.
    let old = {
        let q = unsafe { &mut *q };
        if q.waker.as_ref().is_some_and(|w| w.will_wake(waker)) {
            return;
        }
        let new = waker.clone();
        q.waker.replace(new)
    };
    drop(old);
}

/// Records `event` into `q` through its `capture`, then wakes its waker.
///
/// # Safety
///
/// As for [`pop`]; `q` stays allocated during the call.
unsafe fn record<E, const N: usize>(q: *mut EventQueue<E, N>, event: &Event<'_>) {
    // `capture` runs with no borrow of the queue held: it may send events
    // that push into it.
    // SAFETY: caller contract, short borrows.
    let capture = unsafe { (*q).capture };
    let Some(record) = capture(event) else { return };
    let waker = {
        // SAFETY: as above; `len` and `flags` are read again, after
        // `capture`.
        let q = unsafe { &mut *q };
        if q.len as usize >= N {
            q.flags |= OVERFLOW;
        } else {
            let mut tail = q.head as usize + q.len as usize;
            if tail >= N {
                tail -= N;
            }
            if let Some(slot) = q.buf.get_mut(tail) {
                slot.write(record);
                q.len += 1;
            }
        }
        q.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
}

/// Closes `q` (sticky) and takes its waker, to wake once nothing borrows
/// the queue.
///
/// # Safety
///
/// As for [`pop`].
unsafe fn close<E, const N: usize>(q: *mut EventQueue<E, N>) -> Option<Waker> {
    // SAFETY: caller contract; a short borrow.
    let q = unsafe { &mut *q };
    q.flags |= CLOSED;
    q.waker.take()
}

/// Takes and clears the overflow flag of `q`.
///
/// # Safety
///
/// As for [`pop`].
unsafe fn take_overflow<E, const N: usize>(q: *mut EventQueue<E, N>) -> bool {
    // SAFETY: caller contract; a short borrow.
    let q = unsafe { &mut *q };
    let overflow = q.flags & OVERFLOW != 0;
    q.flags &= !OVERFLOW;
    overflow
}

unsafe extern "C" fn push_trampoline<E: Copy + Send + 'static, const N: usize>(
    e: *mut sys::lv_event_t,
) {
    // SAFETY: the user data of a subscription is its `'static` queue.
    let queue = unsafe { &*sys::lv_event_get_user_data(e).cast::<StaticQueue<E, N>>() };
    // SAFETY: under the token; the queue is never freed.
    dispatch(e, &mut |event| unsafe { record(queue.0.get(), event) });
}

unsafe extern "C" fn close_trampoline<E: Copy + Send + 'static, const N: usize>(
    e: *mut sys::lv_event_t,
) {
    // SAFETY: as above; DELETE of a subscribed object. Runs no user code
    // but the waker's, after the borrow.
    let queue = unsafe { &*sys::lv_event_get_user_data(e).cast::<StaticQueue<E, N>>() };
    if let Some(waker) = unsafe { close(queue.0.get()) } {
        waker.wake();
    }
}

/// How [`Obj::subscribe`] records events: the code (`EventCode::ALL` for
/// every allowed one) and whether the object's children report theirs.
/// An [`EventCode`] converts into options with only that code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SubscribeOptions {
    code: EventCode,
    bubble_children: bool,
}

impl SubscribeOptions {
    /// Records `code` events of the subscribed object.
    pub const fn new(code: EventCode) -> Self {
        Self {
            code,
            bubble_children: false,
        }
    }

    /// Also records the events of the object's children: they get the
    /// `EVENT_BUBBLE` flag now, and children created later get it when
    /// they are created (`CHILD_CREATED`). Their events then also reach
    /// the object's other handlers and the class handlers of widgets.
    pub const fn bubble_children(mut self) -> Self {
        self.bubble_children = true;
        self
    }
}

impl From<EventCode> for SubscribeOptions {
    fn from(code: EventCode) -> Self {
        Self::new(code)
    }
}

/// `CHILD_CREATED` (which always bubbles) of an object subscribed with
/// `bubble_children`: the new object bubbles if it is a direct child.
unsafe extern "C" fn bubble_trampoline(e: *mut sys::lv_event_t) {
    // SAFETY: a live event; for CHILD_CREATED the param is the new object
    // or NULL (safe code sends NULL), fully constructed
    // (`lv_obj_class.c:131`). `lv_obj_set_event_bubble` only writes the
    // flag bit (`lv_obj.c:644-648`).
    unsafe {
        if sys::lv_event_get_code(e) != sys::lv_event_code_t_LV_EVENT_CHILD_CREATED {
            return;
        }
        let child = sys::lv_event_get_param(e).cast::<sys::lv_obj_t>();
        let parent = sys::lv_event_get_current_target(e).cast::<sys::lv_obj_t>();
        if !child.is_null() && sys::lv_obj_get_parent(child) == parent {
            sys::lv_obj_set_event_bubble(child, true);
        }
    }
}

impl Obj {
    /// Records the events `options` select (an [`EventCode`], or
    /// [`SubscribeOptions`]) into `queue`, through its `capture`. Deleting
    /// the object closes the queue.
    pub fn subscribe<E: Copy + Send + 'static, const N: usize>(
        &self,
        options: impl Into<SubscribeOptions>,
        queue: &'static StaticQueue<E, N>,
    ) -> Result<(), Error> {
        let options = options.into();
        self.bubble_children(options)?;
        let data = core::ptr::from_ref(queue).cast_mut().cast();
        self.add_handler(options.code, push_trampoline::<E, N>, data)?;
        self.add_handler(EventCode::DELETE, close_trampoline::<E, N>, data)?;
        Ok(())
    }

    /// Sets up `bubble_children`: one `CHILD_CREATED` handler per object,
    /// and the flag on the current children.
    fn bubble_children(&self, options: SubscribeOptions) -> Result<(), Error> {
        if !options.bubble_children {
            return Ok(());
        }
        let o = self.as_ptr();
        // SAFETY: a live object under the token; descriptors are only
        // read.
        let present = unsafe {
            (0..sys::lv_obj_get_event_count(o)).any(|i| {
                sys::lv_event_dsc_get_cb(sys::lv_obj_get_event_dsc(o, i)).is_some_and(|cb| {
                    core::ptr::fn_addr_eq(cb, bubble_trampoline as unsafe extern "C" fn(_))
                })
            })
        };
        if !present {
            self.add_handler(
                EventCode::CHILD_CREATED,
                bubble_trampoline,
                core::ptr::null_mut(),
            )?;
        }
        // SAFETY: as above; setting the flag only writes a bit.
        unsafe {
            for i in 0..sys::lv_obj_get_child_count(o) {
                sys::lv_obj_set_event_bubble(sys::lv_obj_get_child(o, i as i32), true);
            }
        }
        Ok(())
    }
}

/// A queue on LVGL's heap, attached to one object by
/// [`Obj::subscribe_owned`] (features `async` and `heap`): the receiver
/// of its events. 8 bytes on 32-bit targets, not `Clone`.
///
/// The queue lives as long as its object: deleting the object closes it
/// (events still queued are dropped) and frees it. Dropping an
/// `OwnedQueue` needs no token and leaves the queue attached until then;
/// [`OwnedQueue::close`] detaches and frees it at once.
#[cfg(all(feature = "heap", not(lvgl_rust_views_only)))]
#[must_use = "dropping it keeps the queue until its object is deleted"]
pub struct OwnedQueue<E, const N: usize> {
    object: Handle<Obj>,
    /// The block returned by the allocator, also the handlers' user data.
    block: core::ptr::NonNull<core::ffi::c_void>,
    _e: core::marker::PhantomData<fn() -> E>,
}

// SAFETY: the block is reached only under the token, after the handle and
// the object's handlers confirm it is still attached; events are `Send`.
#[cfg(all(feature = "heap", not(lvgl_rust_views_only)))]
unsafe impl<E: Send, const N: usize> Send for OwnedQueue<E, N> {}

#[cfg(all(feature = "heap", not(lvgl_rust_views_only)))]
type QueueBox<E, const N: usize> = LvBox<(), EventQueue<E, N>>;

#[cfg(all(feature = "heap", not(lvgl_rust_views_only)))]
unsafe extern "C" fn owned_push_trampoline<E: Copy + Send + 'static, const N: usize>(
    e: *mut sys::lv_event_t,
) {
    // SAFETY: the user data is the queue's block, freed only by DELETE or
    // `close` when no frame records into it (`busy`), and reached through
    // raw pointers only.
    unsafe {
        let block = sys::lv_event_get_user_data(e);
        let q = QueueBox::<E, N>::value(block);
        (*q).busy += 1;
        dispatch(e, &mut |event| record(q, event));
        (*q).busy -= 1;
        if (*q).flags & DOOMED != 0 && (*q).busy == 0 {
            QueueBox::<E, N>::free(block);
        }
    }
}

#[cfg(all(feature = "heap", not(lvgl_rust_views_only)))]
unsafe extern "C" fn owned_delete_trampoline<E: Copy + Send + 'static, const N: usize>(
    e: *mut sys::lv_event_t,
) {
    // SAFETY: as above; DELETE of the queue's object, which never bubbles
    // (`lv_obj_event.c:431`). The block is freed here, or by the outermost
    // recording frame.
    unsafe {
        let block = sys::lv_event_get_user_data(e);
        let q = QueueBox::<E, N>::value(block);
        let waker = close(q);
        if (*q).busy == 0 {
            QueueBox::<E, N>::free(block);
        } else {
            (*q).flags |= DOOMED;
        }
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

#[cfg(all(feature = "heap", not(lvgl_rust_views_only)))]
impl Obj {
    /// Records the events `options` select into a queue of `N` events
    /// (1 to 255) on LVGL's heap, through `capture`, and returns its
    /// receiver. Takes the object's registry slot ([`Error::RegistryFull`]),
    /// or shares the one it has.
    pub fn subscribe_owned<E: Copy + Send + 'static, const N: usize>(
        &self,
        options: impl Into<SubscribeOptions>,
        capture: fn(&Event<'_>) -> Option<E>,
    ) -> Result<OwnedQueue<E, N>, Error> {
        use crate::WidgetExt;
        const { assert!(N >= 1 && N <= 255, "a queue holds 1 to 255 events") };
        let options = options.into();
        let object = self.handle()?;
        self.bubble_children(options)?;
        let queue = EventQueue::<E, N> {
            head: 0,
            len: 0,
            flags: 0,
            busy: 0,
            waker: None,
            capture,
            buf: [const { MaybeUninit::uninit() }; N],
        };
        let block = QueueBox::new((), queue)
            .map_err(|_| Error::OutOfMemory)?
            .into_raw();
        let push = match self.add_handler(options.code, owned_push_trampoline::<E, N>, block) {
            Ok(dsc) => dsc,
            Err(error) => {
                // SAFETY: not attached: nothing else has the block.
                unsafe { QueueBox::<E, N>::free(block) };
                return Err(error);
            }
        };
        if let Err(error) =
            self.add_handler(EventCode::DELETE, owned_delete_trampoline::<E, N>, block)
        {
            // SAFETY: the push descriptor was just added, so LVGL is not
            // walking this object's handlers (`add_handler` checks it in
            // callbacks): removing it frees nothing anyone holds. Then
            // nothing else has the block.
            unsafe {
                sys::lv_obj_remove_event_dsc(self.as_ptr(), push);
                QueueBox::<E, N>::free(block);
            }
            return Err(error);
        }
        Ok(OwnedQueue {
            object,
            // SAFETY: `into_raw` returns the non-null block.
            block: unsafe { core::ptr::NonNull::new_unchecked(block) },
            _e: core::marker::PhantomData,
        })
    }
}

#[cfg(all(feature = "heap", not(lvgl_rust_views_only)))]
impl<E: Copy + Send + 'static, const N: usize> OwnedQueue<E, N> {
    /// The queue, if its object is alive and still has this queue's
    /// handlers (a block is only ever reached after this check).
    fn queue(&self, lv: &Lvgl) -> Option<*mut EventQueue<E, N>> {
        let object = self.object.get(lv).ok()?;
        let o = object.as_ptr();
        let block = self.block.as_ptr();
        // SAFETY: a live object under the token; descriptors are only
        // read and compared.
        let attached = unsafe {
            (0..sys::lv_obj_get_event_count(o)).any(|i| {
                let dsc = sys::lv_obj_get_event_dsc(o, i);
                sys::lv_event_dsc_get_user_data(dsc) == block
                    && sys::lv_event_dsc_get_cb(dsc).is_some_and(|cb| {
                        core::ptr::fn_addr_eq(
                            cb,
                            owned_delete_trampoline::<E, N> as unsafe extern "C" fn(_),
                        )
                    })
            })
        };
        attached.then(|| QueueBox::<E, N>::value(block))
    }

    /// Whether events were dropped because the queue was full; clears the
    /// flag. `false` once closed.
    pub fn take_overflow(&self, lv: &Lvgl) -> bool {
        // SAFETY: under the token, an attached queue.
        self.queue(lv).is_some_and(|q| unsafe { take_overflow(q) })
    }

    /// Detaches the queue from its object and frees it (exclusive access:
    /// the object's handler list shrinks). Does nothing if the object was
    /// deleted (the queue went with it) or is about to be.
    pub fn close(self, lv: &mut Lvgl) {
        let Ok(object) = self.object.get(lv) else {
            return;
        };
        let o = object.as_ptr();
        let block = self.block.as_ptr();
        let mut attached = false;
        // SAFETY: exclusive access, depth 0: LVGL is not walking the
        // object's handlers. Indices go downwards, so a removal never
        // shifts one still to visit.
        unsafe {
            let mut i = sys::lv_obj_get_event_count(o);
            while i > 0 {
                i -= 1;
                let dsc = sys::lv_obj_get_event_dsc(o, i);
                if sys::lv_event_dsc_get_user_data(dsc) != block {
                    continue;
                }
                let Some(cb) = sys::lv_event_dsc_get_cb(dsc) else {
                    continue;
                };
                let delete = core::ptr::fn_addr_eq(
                    cb,
                    owned_delete_trampoline::<E, N> as unsafe extern "C" fn(_),
                );
                let push = core::ptr::fn_addr_eq(
                    cb,
                    owned_push_trampoline::<E, N> as unsafe extern "C" fn(_),
                );
                if (delete || push) && sys::lv_obj_remove_event(o, i) {
                    attached |= delete;
                }
            }
            if attached {
                let q = QueueBox::<E, N>::value(block);
                if (*q).busy == 0 {
                    QueueBox::<E, N>::free(block);
                } else {
                    (*q).flags |= DOOMED;
                }
            }
        }
    }
}

#[cfg(all(feature = "heap", not(lvgl_rust_views_only)))]
impl<E: Copy + Send + 'static, const N: usize> Receiver for OwnedQueue<E, N> {
    type Item = E;

    fn try_recv(&self, lv: &Lvgl) -> Recv<E> {
        match self.queue(lv) {
            // SAFETY: under the token, an attached queue.
            Some(q) => unsafe { pop(q) },
            None => Recv::Closed,
        }
    }

    fn register(&self, lv: &Lvgl, waker: &Waker) {
        if let Some(q) = self.queue(lv) {
            // SAFETY: as above.
            unsafe { register(q, waker) }
        }
    }
}

/// Pending once, then ready: for waiting until woken.
struct Woken(bool);

impl Future for Woken {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            Poll::Pending
        }
    }
}

enum Step<E> {
    Item(E),
    Closed,
    Wait,
}

/// Waits for the next event of `rx`: takes the lock with `lock`, starts a
/// session and pops an event, or registers the task's waker, releases the
/// lock and waits. Returns the lock's guard with the event (`None` once
/// the queue is closed, or if a session could not start).
pub async fn next_event<A, R, F>(mut lock: impl FnMut() -> F, rx: &R) -> (A, Option<R::Item>)
where
    A: LvglAccess,
    R: Receiver,
    F: Future<Output = A>,
{
    loop {
        let mut guard = lock().await;
        let step = core::future::poll_fn(|cx| {
            Poll::Ready(match Lvgl::enter(&mut guard) {
                Err(_) => Step::Closed,
                Ok(lv) => match rx.try_recv(&lv) {
                    Recv::Event(event) => Step::Item(event),
                    Recv::Closed => Step::Closed,
                    Recv::Empty => {
                        // Registered under the token: no event can slip in
                        // between the pop and the registration.
                        rx.register(&lv, cx.waker());
                        Step::Wait
                    }
                },
            })
        })
        .await;
        match step {
            Step::Item(event) => return (guard, Some(event)),
            Step::Closed => return (guard, None),
            Step::Wait => {
                drop(guard);
                Woken(false).await;
            }
        }
    }
}
