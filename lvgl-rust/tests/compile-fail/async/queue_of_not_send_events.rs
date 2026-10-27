//! Recorded events cross threads with the token: they must be `Send`.

use lvgl_rust::{Event, StaticQueue};

fn capture(_: &Event<'_>) -> Option<*mut u8> {
    None
}

static QUEUE: StaticQueue<*mut u8, 4> = StaticQueue::new(capture);

fn main() {
    let _ = &QUEUE;
}
