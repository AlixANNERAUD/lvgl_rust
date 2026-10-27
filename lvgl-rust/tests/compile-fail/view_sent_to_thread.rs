//! Views are `!Send`: another thread could use them without the lock.

use lvgl_rust::{Lvgl, LvglAccess};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let lv = Lvgl::enter(&mut guard).unwrap();
    let screen = lv.screen_active().unwrap();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let _ = screen.child(0);
        });
    });
}
