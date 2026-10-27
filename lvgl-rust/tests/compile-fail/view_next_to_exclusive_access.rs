//! A view cannot be passed next to `&mut Lvgl`: the callee could delete its
//! object, then use it.

use lvgl_rust::{Lvgl, LvglAccess, Obj};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn both(lv: &mut Lvgl, object: &Obj) {
    lv.timer_handler();
    let _ = object.child(0);
}

fn main() {
    let mut guard = Guard;
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    both(&mut lv, lv.screen_active().unwrap());
}
