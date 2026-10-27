//! `CLICKED` handlers may delete objects (a message box's close button):
//! the code is a `SendableEventCode`, which only `ObjMut::send_event`
//! takes, not a `SafeEventCode` a view can send.

use lvgl_rust::{Lvgl, LvglAccess, enums::SendableEventCode};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let lv = Lvgl::enter(&mut guard).unwrap();
    let _ = lv
        .screen_active()
        .unwrap()
        .send_event(SendableEventCode::CLICKED);
}
