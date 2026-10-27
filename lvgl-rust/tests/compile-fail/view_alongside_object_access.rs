//! `ObjMut` borrows `&mut Lvgl`: no view can exist while it does, so a view
//! cannot dangle after `clean`.

use lvgl_rust::{Lvgl, LvglAccess};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    let mut screen = lv.edit(|lv| lv.screen_active()).unwrap();
    let child = lv.screen_active().unwrap().child(0);
    screen.clean().unwrap();
    let _ = child;
}
