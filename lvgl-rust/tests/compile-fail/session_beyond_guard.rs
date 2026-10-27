//! A session borrows the guard it was entered with: releasing the lock ends
//! it.

use lvgl_rust::{Lvgl, LvglAccess};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let lv = Lvgl::enter(&mut guard).unwrap();
    drop(guard);
    let _ = lv.screen_active();
}
