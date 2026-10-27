//! Handlers cannot get exclusive access to an object either: deletions from
//! handlers go through `delete_later`.

use lvgl_rust::{Event, HandleCell, Obj};

static TARGET: HandleCell<Obj> = HandleCell::new();

fn handler(event: &Event<'_>) {
    if let Some(handle) = TARGET.get() {
        let _ = handle.get_mut(event.lv()).unwrap().delete();
    }
}

fn main() {
    let _ = handler;
}
