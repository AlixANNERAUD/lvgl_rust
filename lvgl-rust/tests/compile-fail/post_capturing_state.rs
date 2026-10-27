//! `post` queues a function, not a closure with state: a capturing closure
//! is refused when `post` is instantiated.

use lvgl_rust::{Lvgl, LvglAccess};

struct Guard;
// SAFETY: never run; the case only has to compile or not.
unsafe impl LvglAccess for Guard {}

fn main() {
    let mut guard = Guard;
    let lv = Lvgl::enter(&mut guard).unwrap();
    let delay = 3u32;
    let _ = lv.post(
        move |lv, ()| {
            let _ = (lv.timer_handler(), delay);
        },
        (),
    );
}
