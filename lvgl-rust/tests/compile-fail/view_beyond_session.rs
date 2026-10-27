//! A view borrows the session it came from: it cannot be kept after the
//! session ends, when another thread may take the lock.

use lvgl_rust::{Lvgl, LvglAccess, Obj};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let view: &Obj;
    {
        let lv = Lvgl::enter(&mut guard).unwrap();
        view = lv.screen_active().unwrap();
    }
    let _ = view.child(0);
}
