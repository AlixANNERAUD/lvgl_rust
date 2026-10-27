//! `&Lvgl` is `!Send`: shared access stays on the thread holding the lock.

use lvgl_rust::{Lvgl, LvglAccess};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let lv = Lvgl::enter(&mut guard).unwrap();
    let lv: &Lvgl = &lv;
    std::thread::scope(|scope| {
        scope.spawn(move || lv.screen_active().is_some());
    });
}
