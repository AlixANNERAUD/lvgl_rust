//! `timer_handler` runs timers that may delete objects: it takes `&mut Lvgl`,
//! so no view can be used across it.

use lvgl_rust::{Lvgl, LvglAccess};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    let screen = lv.screen_active().unwrap();
    lv.timer_handler();
    let _ = screen.child(0);
}
