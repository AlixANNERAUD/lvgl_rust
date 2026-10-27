//! A view from an `Owned` borrows the session, not the owner, but the owner
//! deletes at the next drain: `timer_handler` (exclusive) is what ends the
//! view.

use lvgl_rust::{Lvgl, LvglAccess};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let mut lv = Lvgl::enter(&mut guard).unwrap();
    let screen = lv.create_screen().unwrap();
    let view = screen.get(&lv).unwrap();
    drop(screen);
    lv.timer_handler();
    let _ = view.child(0);
}
