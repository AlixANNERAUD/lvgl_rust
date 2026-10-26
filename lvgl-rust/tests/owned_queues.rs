//! Event queues on LVGL's heap, attached to their object
//! (`--features async,heap`), and children that bubble into a queue.
#![cfg(all(feature = "async", feature = "heap", not(lvgl_rust_views_only)))]

use lvgl_rust::{
    Event, Lvgl, LvglAccess, Obj, Receiver, Recv, StaticQueue, SubscribeOptions, enums::EventCode,
    sys,
};
use std::{
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU32, Ordering::Relaxed},
    },
    task::{Wake, Waker},
};

static LOCK: Mutex<bool> = Mutex::new(false);

struct Guard(MutexGuard<'static, bool>);

// SAFETY: a guard of the mutex serializing every LVGL call of this process.
unsafe impl LvglAccess for Guard {}

fn guard() -> Guard {
    Guard(LOCK.lock().unwrap_or_else(|p| p.into_inner()))
}

fn session(guard: &mut Guard) -> lvgl_rust::Session<'_> {
    if !*guard.0 {
        *guard.0 = true;
        let lv = lvgl_rust::init(guard).unwrap();
        // SAFETY: session held.
        unsafe { sys::lv_display_create(240, 160) };
        lv
    } else {
        Lvgl::enter(guard).unwrap()
    }
}

/// Counts its wake-ups.
struct Counter(AtomicU32);

impl Wake for Counter {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Relaxed);
    }
}

fn capture(e: &Event<'_>) -> Option<u32> {
    Some(e.code().bits())
}

fn send(object: &Obj, code: EventCode) {
    // SAFETY: session held, live object; no param.
    unsafe { sys::lv_obj_send_event(object.as_ptr(), code.bits() as _, core::ptr::null_mut()) };
}

fn event_count(object: &Obj) -> u32 {
    // SAFETY: session held, live object.
    unsafe { sys::lv_obj_get_event_count(object.as_ptr()) }
}

fn last(lv: &Lvgl) -> Option<&Obj> {
    lv.screen_active()?.child(-1)
}

#[test]
fn owned_queues_record_until_their_object_is_deleted() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let object = Obj::create(lv.screen_active().unwrap()).unwrap();
    let queue = object
        .subscribe_owned::<u32, 2>(EventCode::CLICKED, capture)
        .unwrap();
    // The object's slot is shared with an existing handle.
    let again = object
        .subscribe_owned::<u32, 1>(EventCode::PRESSED, capture)
        .unwrap();
    assert_eq!(queue.try_recv(&lv), Recv::Empty);
    let counter = Arc::new(Counter(AtomicU32::new(0)));
    let waker = Waker::from(counter.clone());
    queue.register(&lv, &waker);
    send(object, EventCode::CLICKED);
    send(object, EventCode::PRESSED);
    assert_eq!(counter.0.load(Relaxed), 1);
    for _ in 0..2 {
        send(object, EventCode::CLICKED);
    }
    assert!(queue.take_overflow(&lv));
    assert_eq!(queue.try_recv(&lv), Recv::Event(EventCode::CLICKED.bits()));
    assert_eq!(queue.try_recv(&lv), Recv::Event(EventCode::CLICKED.bits()));
    assert_eq!(queue.try_recv(&lv), Recv::Empty);
    assert_eq!(again.try_recv(&lv), Recv::Event(EventCode::PRESSED.bits()));
    // Dropping a receiver leaves its queue attached; deletion frees both
    // and wakes the registered task.
    drop(again);
    queue.register(&lv, &waker);
    lv.edit(last).unwrap().delete().unwrap();
    assert_eq!(counter.0.load(Relaxed), 2);
    assert_eq!(queue.try_recv(&lv), Recv::Closed);
    // A closed queue registers nothing (the box is gone).
    queue.register(&lv, &waker);
    queue.close(&mut lv);
    // Only `waker` is left.
    assert_eq!(Arc::strong_count(&counter), 2);
}

#[test]
fn closing_an_owned_queue_detaches_it() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let object = Obj::create(lv.screen_active().unwrap()).unwrap();
    let before = event_count(object);
    let queue = object
        .subscribe_owned::<u32, 4>(EventCode::ALL, capture)
        .unwrap();
    assert_eq!(event_count(object), before + 2);
    let counter = Arc::new(Counter(AtomicU32::new(0)));
    queue.register(&lv, &Waker::from(counter.clone()));
    assert_eq!(Arc::strong_count(&counter), 2);
    queue.close(&mut lv);
    // The waker went with the queue.
    assert_eq!(Arc::strong_count(&counter), 1);
    let object = last(&lv).unwrap();
    assert_eq!(event_count(object), before);
    send(object, EventCode::CLICKED);
    lv.edit(last).unwrap().delete().unwrap();
}

#[test]
fn queues_of_any_alignment_are_attached() {
    #[derive(Clone, Copy, PartialEq, Debug)]
    #[repr(align(16))]
    struct Wide(u64);
    fn wide(e: &Event<'_>) -> Option<Wide> {
        Some(Wide(e.code().bits() as u64))
    }
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let object = Obj::create(lv.screen_active().unwrap()).unwrap();
    let queue = object
        .subscribe_owned::<Wide, 3>(EventCode::CLICKED, wide)
        .unwrap();
    send(object, EventCode::CLICKED);
    assert_eq!(
        queue.try_recv(&lv),
        Recv::Event(Wide(EventCode::CLICKED.bits() as u64))
    );
    lv.edit(last).unwrap().delete().unwrap();
    assert_eq!(queue.try_recv(&lv), Recv::Closed);
}

static BUBBLED: StaticQueue<u32, 8> = StaticQueue::new(capture);

#[test]
fn children_bubble_into_queues_when_asked() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    let parent = Obj::create(lv.screen_active().unwrap()).unwrap();
    let early = Obj::create(parent).unwrap();
    let options = SubscribeOptions::new(EventCode::CLICKED).bubble_children();
    let queue = parent.subscribe_owned::<u32, 8>(options, capture).unwrap();
    parent.subscribe(options, &BUBBLED).unwrap();
    let rx = BUBBLED.receiver(&lv).unwrap();
    let before = event_count(parent);
    // A second subscription adds no second CHILD_CREATED handler.
    let _more = parent.subscribe_owned::<u32, 1>(options, capture).unwrap();
    assert_eq!(event_count(parent), before + 2);
    let late = Obj::create(parent).unwrap();
    let grandchild = Obj::create(late).unwrap();
    send(early, EventCode::CLICKED);
    send(late, EventCode::CLICKED);
    assert_eq!(queue.try_recv(&lv), Recv::Event(EventCode::CLICKED.bits()));
    assert_eq!(queue.try_recv(&lv), Recv::Event(EventCode::CLICKED.bits()));
    assert_eq!(rx.try_recv(&lv), Recv::Event(EventCode::CLICKED.bits()));
    assert_eq!(rx.try_recv(&lv), Recv::Event(EventCode::CLICKED.bits()));
    // Only direct children bubble.
    send(grandchild, EventCode::CLICKED);
    assert_eq!(queue.try_recv(&lv), Recv::Empty);
    // Without the option, children keep their events.
    let plain = Obj::create(lv.screen_active().unwrap()).unwrap();
    let child = Obj::create(plain).unwrap();
    let quiet = plain
        .subscribe_owned::<u32, 1>(EventCode::CLICKED, capture)
        .unwrap();
    send(child, EventCode::CLICKED);
    assert_eq!(quiet.try_recv(&lv), Recv::Empty);
    lv.edit(last).unwrap().delete().unwrap();
    lv.edit(last).unwrap().delete().unwrap();
    assert_eq!(queue.try_recv(&lv), Recv::Closed);
    assert_eq!(rx.try_recv(&lv), Recv::Closed);
}
