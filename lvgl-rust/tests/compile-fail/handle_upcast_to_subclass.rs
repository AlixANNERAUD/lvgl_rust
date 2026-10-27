//! `upcast` only goes to a base class: an `Obj` is not known to be a label.

use lvgl_rust::{Handle, Obj, widgets::Label};

fn label(handle: Handle<Obj>) -> Handle<Label> {
    handle.upcast::<Label>()
}

fn main() {
    let _ = label;
}
