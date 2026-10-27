//! A static queue has a single receiver: `StaticRx` cannot be cloned, so
//! popping needs no compare-and-swap.

use lvgl_rust::{Event, Lvgl, LvglAccess, StaticQueue};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn capture(event: &Event<'_>) -> Option<u32> {
    Some(event.code().bits())
}

static QUEUE: StaticQueue<u32, 4> = StaticQueue::new(capture);

fn main() {
    let mut guard = Guard;
    let lv = Lvgl::enter(&mut guard).unwrap();
    let rx = QUEUE.receiver(&lv).unwrap();
    let _second = rx.clone();
}
