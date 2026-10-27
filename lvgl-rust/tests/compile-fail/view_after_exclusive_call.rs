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
    Obj::create(&mut screen.get_mut(&mut lv).unwrap()).unwrap();
    let child = screen.get(&lv).unwrap().child(0).unwrap();
    screen.delete(&mut lv).unwrap();
    let _ = child.parent();
}
