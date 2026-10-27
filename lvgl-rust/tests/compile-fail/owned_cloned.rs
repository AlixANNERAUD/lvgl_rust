//! An `Owned` is the single owner of its object: it cannot be cloned (two
//! owners would delete it twice).

use lvgl_rust::{Lvgl, LvglAccess};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let lv = Lvgl::enter(&mut guard).unwrap();
    let screen = lv.create_screen().unwrap();
    let _second = screen.clone();
}
