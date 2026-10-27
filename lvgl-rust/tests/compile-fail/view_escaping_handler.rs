//! The objects an event gives borrow the handler's shared access: they
//! cannot escape the handler.

use lvgl_rust::{Event, Obj};

fn escape(event: &Event<'_>) -> &'static Obj {
    event.target().unwrap()
}

fn main() {
    let _ = escape;
}
