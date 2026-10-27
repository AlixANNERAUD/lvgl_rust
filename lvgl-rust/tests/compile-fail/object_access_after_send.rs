//! `ObjMut::send_event` may run handlers that delete the object: it consumes
//! the access and gives it back only if the object survived.

use lvgl_rust::{Lvgl, LvglAccess, enums::SendableEventCode};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    let mut screen = lv.edit(|lv| lv.screen_active()).unwrap();
    let _ = screen.send_event(SendableEventCode::CLICKED);
    screen.clean().unwrap();
}
