//! Handlers run inside the LVGL call that sent the event, with shared access
//! only: no exclusive call (here `timer_handler`, which may delete
//! objects) is possible from them.

use lvgl_rust::Event;

fn handler(event: &Event<'_>) {
    event.lv().timer_handler();
}

fn main() {
    let _ = handler;
}
