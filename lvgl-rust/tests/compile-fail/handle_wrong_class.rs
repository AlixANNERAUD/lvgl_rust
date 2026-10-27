//! Handles are typed: a `Handle<Obj>` is not a `Handle<Label>` without a
//! checked downcast.

use lvgl_rust::{Handle, Obj, widgets::Label};

fn label(handle: Handle<Obj>) -> Handle<Label> {
    handle
}

fn main() {
    let _ = label;
}
