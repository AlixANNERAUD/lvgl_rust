//! Subscriptions keep a pointer to their queue until the object is deleted:
//! queues and receivers need a `'static` queue.

use lvgl_rust::{Event, Lvgl, LvglAccess, StaticQueue};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn capture(event: &Event<'_>) -> Option<u32> {
    Some(event.code().bits())
}

fn main() {
    let mut guard = Guard;
    let lv = Lvgl::enter(&mut guard).unwrap();
    let queue = StaticQueue::<u32, 4>::new(capture);
    let _rx = queue.receiver(&lv);
}
