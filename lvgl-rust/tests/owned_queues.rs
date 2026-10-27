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
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    let object = last(&lv).unwrap();
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
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    let object = last(&lv).unwrap();
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
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    let object = last(&lv).unwrap();
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
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    Obj::create(&mut lv.edit(last).unwrap()).unwrap();
    let parent = last(&lv).unwrap();
    let options = SubscribeOptions::new(EventCode::CLICKED).bubble_children();
    let queue = parent.subscribe_owned::<u32, 8>(options, capture).unwrap();
    parent.subscribe(options, &BUBBLED).unwrap();
    let rx = BUBBLED.receiver(&lv).unwrap();
    let before = event_count(parent);
    // A second subscription adds no second CHILD_CREATED handler.
    let _more = parent.subscribe_owned::<u32, 1>(options, capture).unwrap();
    assert_eq!(event_count(parent), before + 2);
    Obj::create(&mut lv.edit(last).unwrap()).unwrap();
    Obj::create(&mut lv.edit(|lv| last(lv)?.child(-1)).unwrap()).unwrap();
    let parent = last(&lv).unwrap();
    let (early, late) = (parent.child(0).unwrap(), parent.child(1).unwrap());
    let grandchild = late.child(0).unwrap();
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
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    Obj::create(&mut lv.edit(last).unwrap()).unwrap();
    let plain = last(&lv).unwrap();
    let child = plain.child(0).unwrap();
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

/// Deletes its target from inside the recording frame (raw code that
/// breaks the deferral rules on purpose): the block outlives that frame,
/// freed at the next drain (AddressSanitizer checks the order).
fn delete_target(e: &Event<'_>) -> Option<u32> {
    if e.code() == EventCode::CLICKED {
        let target = e.target()?.as_ptr();
        // SAFETY: test only: the session holder runs this callback and
        // nothing touches the object afterwards.
        unsafe { sys::lv_obj_delete(target) };
    }
    Some(e.code().bits())
}

#[test]
fn a_queue_deleted_while_recording_outlives_the_frame() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    let object = last(&lv).unwrap();
    let queue = object
        .subscribe_owned::<u32, 2>(EventCode::CLICKED, delete_target)
        .unwrap();
    let counter = Arc::new(Counter(AtomicU32::new(0)));
    queue.register(&lv, &Waker::from(counter.clone()));
    send(object, EventCode::CLICKED);
    // DELETE woke the task; the queue went with the object.
    assert_eq!(counter.0.load(Relaxed), 1);
    assert_eq!(Arc::strong_count(&counter), 1);
    assert_eq!(queue.try_recv(&lv), Recv::Closed);
}

static CLOSING: Mutex<Option<lvgl_rust::OwnedQueue<u32, 2>>> = Mutex::new(None);

/// Closes the queue it records into (through `Lvgl::assume`, as raw code
/// may): the handlers are only marked removed while LVGL walks them, and
/// the block outlives the frame.
fn close_own_queue(e: &Event<'_>) -> Option<u32> {
    if let Some(queue) = CLOSING.lock().unwrap().take() {
        // SAFETY: test only: the session holder runs this callback, and no
        // other reference to the token is used while this one lives.
        queue.close(unsafe { Lvgl::assume() });
    }
    Some(e.code().bits())
}

#[test]
fn a_queue_closed_while_recording_outlives_the_frame() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    let object = last(&lv).unwrap();
    let before = event_count(object);
    let queue = object
        .subscribe_owned::<u32, 2>(EventCode::CLICKED, close_own_queue)
        .unwrap();
    *CLOSING.lock().unwrap() = Some(queue);
    send(object, EventCode::CLICKED);
    let object = last(&lv).unwrap();
    assert_eq!(event_count(object), before);
    send(object, EventCode::CLICKED);
    lv.edit(last).unwrap().delete().unwrap();
}

static REFUSED: StaticQueue<u32, 1> = StaticQueue::new(capture);

fn bubbles(object: &Obj) -> bool {
    // SAFETY: session held, live object.
    unsafe { sys::lv_obj_has_flag(object.as_ptr(), sys::lv_obj_flag_t_LV_OBJ_FLAG_EVENT_BUBBLE) }
}

#[test]
fn refused_subscriptions_leave_children_alone() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    Obj::create(&mut lv.edit(last).unwrap()).unwrap();
    let parent = last(&lv).unwrap();
    let child = parent.child(0).unwrap();
    let before = event_count(parent);
    // A code handlers never receive: refused before anything changes.
    let options = SubscribeOptions::new(EventCode::DRAW_MAIN).bubble_children();
    assert_eq!(
        parent.subscribe(options, &REFUSED),
        Err(lvgl_rust::Error::InvalidArg)
    );
    assert!(
        parent
            .subscribe_owned::<u32, 1>(options, capture)
            .is_err_and(|error| error == lvgl_rust::Error::InvalidArg)
    );
    assert_eq!(event_count(parent), before);
    assert!(!bubbles(child));
    // Children created later do not bubble either.
    Obj::create(&mut lv.edit(last).unwrap()).unwrap();
    let late = last(&lv).unwrap().child(-1).unwrap();
    assert!(!bubbles(late));
    lv.edit(last).unwrap().delete().unwrap();
}

static TRICKLED: AtomicU32 = AtomicU32::new(0);

fn count_ready(e: &Event<'_>) -> Option<u32> {
    if e.code() == EventCode::READY {
        TRICKLED.fetch_add(1, Relaxed);
    }
    Some(e.code().bits())
}

/// DELETE handler sending READY to the object's parent, which trickles it
/// back down to the object while LVGL still holds its handlers (safe code
/// can do the same: `set_event_trickle(true)` on the parent, then
/// `Obj::send_event(SafeEventCode::READY)` through a handle).
unsafe extern "C" fn send_to_parent(e: *mut sys::lv_event_t) {
    // SAFETY: a live event of a live object, under the session.
    unsafe {
        let object = sys::lv_event_get_current_target(e).cast::<sys::lv_obj_t>();
        let parent = sys::lv_obj_get_parent(object);
        sys::lv_obj_send_event(
            parent,
            sys::lv_event_code_t_LV_EVENT_READY,
            core::ptr::null_mut(),
        );
    }
}

#[test]
fn owned_queues_ignore_events_trickled_after_delete() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    let mut parent = lv.edit(last).unwrap();
    // SAFETY: session held, live object.
    unsafe {
        sys::lv_obj_add_flag(
            parent.view().as_ptr(),
            sys::lv_obj_flag_t_LV_OBJ_FLAG_EVENT_TRICKLE,
        )
    };
    let child = Obj::create(&mut parent).unwrap();
    let queue = child
        .subscribe_owned::<u32, 4>(EventCode::ALL, count_ready)
        .unwrap();
    // SAFETY: session held, live object; the callback needs no data.
    unsafe {
        sys::lv_obj_add_event_cb(
            child.as_ptr(),
            Some(send_to_parent),
            sys::lv_event_code_t_LV_EVENT_DELETE,
            core::ptr::null_mut(),
        )
    };
    send(last(&lv).unwrap(), EventCode::READY);
    assert_eq!(TRICKLED.load(Relaxed), 1);
    lv.edit(|lv| last(lv)?.child(0)).unwrap().delete().unwrap();
    // The closed queue recorded nothing more; its block waited for the
    // drain (AddressSanitizer checks the order).
    assert_eq!(TRICKLED.load(Relaxed), 1);
    assert_eq!(queue.try_recv(&lv), Recv::Closed);
    lv.edit(last).unwrap().delete().unwrap();
}

/// Blocks in use on LVGL's builtin heap (`None` with another allocator).
fn used_blocks() -> Option<usize> {
    // SAFETY: session held; the monitor only reads the heap.
    let monitor = unsafe {
        let mut monitor = core::mem::zeroed::<sys::lv_mem_monitor_t>();
        sys::lv_mem_monitor(&mut monitor);
        monitor
    };
    (monitor.total_size != 0).then_some(monitor.used_cnt)
}

unsafe extern "C" fn nothing(_: *mut sys::lv_event_t) {}

#[test]
fn a_full_registry_refuses_subscriptions_without_a_trace() {
    use lvgl_rust::WidgetExt;
    let mut guard = guard();
    let mut lv = session(&mut guard);
    // Every free slot goes to a child of `holders`.
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    let mut holders = lv.edit(last).unwrap();
    let mut handles = Vec::new();
    loop {
        let holder = Obj::create(&mut holders).unwrap();
        match holder.handle() {
            Ok(handle) => handles.push(handle),
            Err(error) => {
                assert_eq!(error, lvgl_rust::Error::RegistryFull);
                break;
            }
        }
        assert!(handles.len() < 4096, "the registry never filled");
    }
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    Obj::create(&mut lv.edit(last).unwrap()).unwrap();
    let target = last(&lv).unwrap();
    let child = target.child(0).unwrap();
    // One handler for good: the object's event list exists before and
    // after, so the heap is comparable.
    // SAFETY: session held, live object; the callback needs no data.
    unsafe {
        sys::lv_obj_add_event_cb(
            target.as_ptr(),
            Some(nothing),
            sys::lv_event_code_t_LV_EVENT_CLICKED,
            core::ptr::null_mut(),
        )
    };
    let before = event_count(target);
    let used = used_blocks();
    let options = SubscribeOptions::new(EventCode::CLICKED).bubble_children();
    // Every handler was added, then the slot was refused: all removed.
    assert!(
        target
            .subscribe_owned::<u32, 4>(options, capture)
            .is_err_and(|error| error == lvgl_rust::Error::RegistryFull)
    );
    assert_eq!(event_count(target), before);
    assert_eq!(used_blocks(), used);
    assert!(!bubbles(child));
    Obj::create(&mut lv.edit(last).unwrap()).unwrap();
    let late = last(&lv).unwrap().child(-1).unwrap();
    assert!(!bubbles(late));
    drop(handles);
    lv.edit(last).unwrap().delete().unwrap();
    lv.edit(last).unwrap().delete().unwrap();
}

#[test]
#[cfg_attr(target_os = "wasi", ignore = "WASI has no threads")]
fn owned_queues_drop_on_other_threads() {
    let mut guard = guard();
    let mut lv = session(&mut guard);
    Obj::create(&mut lv.screen_active_mut().unwrap()).unwrap();
    let object = last(&lv).unwrap();
    let queue = object
        .subscribe_owned::<u32, 2>(EventCode::CLICKED, capture)
        .unwrap();
    // Dropping needs no token: the queue stays attached.
    std::thread::spawn(move || drop(queue)).join().unwrap();
    let object = last(&lv).unwrap();
    let before = event_count(object);
    send(object, EventCode::CLICKED);
    assert_eq!(event_count(object), before);
    lv.edit(last).unwrap().delete().unwrap();
}
