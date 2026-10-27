//! `DELETE` is sent by LVGL while it deletes an object, never by safe code:
//! there is no constant for it in either sendable code type.

use lvgl_rust::{Lvgl, LvglAccess, enums::SendableEventCode};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    let screen = lv.edit(|lv| lv.screen_active()).unwrap();
    let _ = screen.send_event(SendableEventCode::DELETE);
}
