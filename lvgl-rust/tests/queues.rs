//! Async event queues against LVGL (`--features async`).
#![cfg(feature = "async")]

use lvgl_rust::{
    Event, Lvgl, LvglAccess, Obj, Receiver, Recv, StaticQueue, enums::EventCode, next_event, sys,
};
use std::{
    future::Future,
    pin::pin,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU32, Ordering::Relaxed},
    },
    task::{Context, Poll, Wake, Waker},
};

static LOCK: Mutex<()> = Mutex::new(());

struct Guard(#[allow(dead_code)] MutexGuard<'static, ()>);

// SAFETY: a guard of the mutex serializing every LVGL call of this process.
unsafe impl LvglAccess for Guard {}

/// Counts its wake-ups.
struct Counter(AtomicU32);

impl Wake for Counter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Relaxed);
    }
}

/// Polls `f` once with `waker`.
fn poll_once<F: Future>(f: std::pin::Pin<&mut F>, waker: &Waker) -> Poll<F::Output> {
    f.poll(&mut Context::from_waker(waker))
}

/// The code of each event, as a `u32`.
fn capture(e: &Event<'_>) -> Option<u32> {
    Some(e.code().bits())
}

static CLICKS: StaticQueue<u32, 2> = StaticQueue::new(capture);

fn send(object: *mut sys::lv_obj_t, code: EventCode) {
    // SAFETY: called with the lock held, on a live object; no param.
    unsafe { sys::lv_obj_send_event(object, code.bits() as _, core::ptr::null_mut()) };
}

#[test]
fn queues_record_events_for_async_code() {
    let mut guard = Guard(LOCK.lock().unwrap());
    let mut lv = lvgl_rust::init(&mut guard).unwrap();
    // SAFETY: session held.
    unsafe { sys::lv_display_create(240, 160) };
    let mut screen = lv.screen_active_mut().unwrap();
    let object = Obj::create(&mut screen).unwrap().into_view();
    object.subscribe(EventCode::CLICKED, &CLICKS).unwrap();
    let object = object.as_ptr();
    let rx = CLICKS.receiver(&lv).unwrap();
    assert!(CLICKS.receiver(&lv).is_none(), "one receiver");
    drop(lv);
    drop(guard);

    let counter = Arc::new(Counter(AtomicU32::new(0)));
    let waker = Waker::from(counter.clone());
    let lock = || async { Guard(LOCK.lock().unwrap()) };

    // Nothing yet: the task waits, with its waker registered.
    {
        let mut next = pin!(next_event(lock, &rx));
        assert!(poll_once(next.as_mut(), &waker).is_pending());
        // An event wakes it, and the next poll returns it with the guard.
        {
            let mut guard = Guard(LOCK.lock().unwrap());
            let _lv = Lvgl::enter(&mut guard).unwrap();
            send(object, EventCode::CLICKED);
        }
        assert_eq!(counter.0.load(Relaxed), 1);
        match poll_once(next.as_mut(), &waker) {
            Poll::Ready((_guard, event)) => assert_eq!(event, Some(EventCode::CLICKED.bits())),
            Poll::Pending => panic!("woken but not ready"),
        }
    }

    // Overflow drops the newest event; deletion closes the queue after the
    // queued events.
    let mut guard = Guard(LOCK.lock().unwrap());
    {
        let mut lv = Lvgl::enter(&mut guard).unwrap();
        for _ in 0..3 {
            send(object, EventCode::CLICKED);
        }
        assert!(CLICKS.take_overflow(&lv));
        lv.edit(|lv| lv.screen_active()?.child(-1))
            .unwrap()
            .delete()
            .unwrap();
        assert!(matches!(rx.try_recv(&lv), Recv::Event(_)));
        assert!(matches!(rx.try_recv(&lv), Recv::Event(_)));
        assert_eq!(rx.try_recv(&lv), Recv::Closed);
    }
    drop(guard);
    let mut next = pin!(next_event(lock, &rx));
    match poll_once(next.as_mut(), &waker) {
        Poll::Ready((_guard, event)) => assert_eq!(event, None),
        Poll::Pending => panic!("a closed queue returns at once"),
    }
}
