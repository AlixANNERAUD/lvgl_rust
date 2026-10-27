//! Creating through a parent's `ObjMut` lends a view of the new object,
//! not an `ObjMut`: a deleting send on the child, which could delete the
//! parent while its `ObjMut` is in scope, needs the child's own access.

use lvgl_rust::{
    Lvgl, LvglAccess,
    enums::SendableEventCode,
    widgets::{Button, Label},
};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    let mut button = lv
        .edit(|lv| lv.screen_active()?.child(0)?.downcast::<Button>())
        .unwrap();
    let label = Label::create(&mut button).unwrap();
    let _ = label.send_event(SendableEventCode::CLICKED);
    let _ = button.view();
}
