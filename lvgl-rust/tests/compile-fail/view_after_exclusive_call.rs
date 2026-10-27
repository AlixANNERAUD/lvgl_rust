//! Deleting an owned object takes `&mut Lvgl`: a view of one of its children
//! cannot be used afterwards.

use lvgl_rust::{Lvgl, LvglAccess, Obj};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    let screen = lv.create_screen().unwrap();
    let child = Obj::create(screen.get(&lv).unwrap()).unwrap();
    screen.delete(&mut lv).unwrap();
    let _ = child.parent();
}
