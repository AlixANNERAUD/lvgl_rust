//! `subscribe` stores the queue in the object's handler: a local queue is
//! refused.

use lvgl_rust::{Event, Lvgl, LvglAccess, StaticQueue, enums::EventCode};

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
    let _ = lv
        .screen_active()
        .unwrap()
        .subscribe(EventCode::CLICKED, &queue);
}
