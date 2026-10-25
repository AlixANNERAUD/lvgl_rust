//! Async events: handlers that record events into a static queue, read
//! from async code with [`next_event`]. Queues are touched only under the
//! token: no atomics, no critical section, no compare-and-swap.

use crate::{Error, Event, Lvgl, LvglAccess, Obj, enums::EventCode, event::dispatch, sys};
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
        // SAFETY: as above.
        let q = unsafe { &mut *self.0.get() };
        let overflow = q.flags & OVERFLOW != 0;
        q.flags &= !OVERFLOW;
        overflow
    }
}

/// The receiver of a [`StaticQueue`]: one per queue, not `Clone`.
pub struct StaticRx<E: 'static, const N: usize>(&'static StaticQueue<E, N>);

impl<E: Copy + Send + 'static, const N: usize> Receiver for StaticRx<E, N> {
    type Item = E;

    fn try_recv(&self, lv: &Lvgl) -> Recv<E> {
        let _ = lv;
        // SAFETY: under the token; no user code runs during the borrow.
        let q = unsafe { &mut *self.0.0.get() };
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

    fn register(&self, lv: &Lvgl, waker: &Waker) {
        let _ = lv;
        // SAFETY: as above; the old waker is dropped after the borrow.
        let old = {
            let q = unsafe { &mut *self.0.0.get() };
            if q.waker.as_ref().is_some_and(|w| w.will_wake(waker)) {
                return;
            }
            q.waker.replace(waker.clone())
        };
        drop(old);
    }
}

unsafe extern "C" fn push_trampoline<E: Copy + Send + 'static, const N: usize>(
    e: *mut sys::lv_event_t,
) {
    // SAFETY: the user data of a subscription is its `'static` queue.
    let queue = unsafe { &*sys::lv_event_get_user_data(e).cast::<StaticQueue<E, N>>() };
    dispatch(e, &mut |event| {
        // `capture` runs with no borrow of the queue held: it may send
        // events that push into it.
        // SAFETY: under the token, short borrows.
        let capture = unsafe { (*queue.0.get()).capture };
        let Some(record) = capture(event) else { return };
        let waker = {
            let q = unsafe { &mut *queue.0.get() };
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
    });
}

unsafe extern "C" fn close_trampoline<E: Copy + Send + 'static, const N: usize>(
    e: *mut sys::lv_event_t,
) {
    // SAFETY: as above; DELETE of a subscribed object. Runs no user code
    // but the waker's.
    let queue = unsafe { &*sys::lv_event_get_user_data(e).cast::<StaticQueue<E, N>>() };
    let waker = {
        let q = unsafe { &mut *queue.0.get() };
        q.flags |= CLOSED;
        q.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
}

impl Obj {
    /// Records `code` events of this object (or every allowed one with
    /// `EventCode::ALL`) into `queue`, through its `capture`. Deleting the
    /// object closes the queue.
    pub fn subscribe<E: Copy + Send + 'static, const N: usize>(
        &self,
        code: EventCode,
        queue: &'static StaticQueue<E, N>,
    ) -> Result<(), Error> {
        let data = core::ptr::from_ref(queue).cast_mut().cast();
        self.add_handler(code, push_trampoline::<E, N>, data)?;
        self.add_handler(EventCode::DELETE, close_trampoline::<E, N>, data)?;
        Ok(())
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
