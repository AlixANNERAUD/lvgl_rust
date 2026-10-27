//! A queue holds 1 to 255 events (its indices are bytes): 0 is refused.

use lvgl_rust::{Event, StaticQueue};

fn capture(_: &Event<'_>) -> Option<u8> {
    None
}

static QUEUE: StaticQueue<u8, 0> = StaticQueue::new(capture);

fn main() {
    let _ = &QUEUE;
}
