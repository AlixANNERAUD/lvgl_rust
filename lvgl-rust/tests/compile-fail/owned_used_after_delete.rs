//! `Owned::delete` consumes the owner: it cannot be used afterwards.

use lvgl_rust::{Lvgl, LvglAccess};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    let screen = lv.create_screen().unwrap();
    screen.delete(&mut lv).unwrap();
    let _ = screen.get(&lv);
}
